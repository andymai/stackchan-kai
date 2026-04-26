//! `StartleOnLoud`: single-tick reaction to a high-amplitude acoustic
//! transient (clap, shout, slam).
//!
//! ## Detection shape
//!
//! Watches `entity.perception.audio_rms` (linear, `0.0..=1.0`,
//! normalised against full-scale i16). On the **rising edge** above
//! [`STARTLE_RMS_THRESHOLD`] — that is, the previous tick was below
//! threshold and the current tick is above — the modifier fires:
//!
//! - `mind.affect.emotion` ← `Surprised`
//! - `mind.autonomy.manual_until` ← `now + STARTLE_HOLD_MS`
//! - `mind.autonomy.source` ← [`OverrideSource::Startle`]
//! - `mind.intent` ← [`Intent::HearingLoud`]
//! - `voice.chirp_request` ← [`ChirpKind::Startle`]
//!
//! There's no sustain requirement (the contrast with
//! [`super::WakeOnVoice`] / [`super::super::skills::LookAtSound`], both
//! of which want sustained voice). A startle is by definition a single
//! transient; sustain would defeat the responsiveness.
//!
//! ## Re-arm
//!
//! After firing, the modifier holds [`Intent::HearingLoud`] for
//! [`STARTLE_HOLD_MS`] then clears it back to [`Intent::Idle`]. A
//! second startle within the hold window is suppressed (the avatar is
//! already in the reaction). After the hold expires, audio must drop
//! back below threshold before another startle can fire — Schmitt-style
//! hysteresis prevents a sustained loud burst from re-triggering on
//! every tick once the hold lapses.
//!
//! ## Coordination with explicit input
//!
//! Defers to `EmotionTouch` / `RemoteCommand` holds (`OverrideSource::Touch`,
//! `BodyTouch`, `Remote`). Overrides everything else, including
//! [`super::WakeOnVoice`]'s sustained-voice hold — a sudden loud noise
//! during a conversation should still startle the avatar.
//!
//! Self-trigger from the speaker is gated at the firmware boundary:
//! when the audio task is playing TX, `entity.perception.audio_rms` is
//! held at `None`. This modifier therefore never sees its own chirp.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::mind::{Intent, OverrideSource};
use crate::modifier::Modifier;
use crate::voice::ChirpKind;

/// Linear RMS threshold above which a tick counts as a startle-class
/// transient.
///
/// `0.4 ≈ -8 dBFS` — well clear of normal speech (`~-26 dBFS`, the
/// [`super::WAKE_RMS_THRESHOLD`]) and ambient room noise. Tuned
/// against the CoreS3 mic at typical desktop distance; see PR
/// description for on-device measurements.
pub const STARTLE_RMS_THRESHOLD: f32 = 0.4;

/// How long the [`Intent::HearingLoud`] reaction holds before clearing
/// back to [`Intent::Idle`].
///
/// 1500 ms reads as "the avatar reacted visibly and is now settling"
/// — long enough for the LED flash + head recoil to play out, short
/// enough that the avatar doesn't get stuck in startle.
pub const STARTLE_HOLD_MS: u64 = 1_500;

/// Modifier that watches `entity.perception.audio_rms` for a transient
/// above [`STARTLE_RMS_THRESHOLD`] and reacts on the rising edge.
#[derive(Debug, Clone, Copy)]
pub struct StartleOnLoud {
    /// Linear-RMS threshold above which a tick counts as loud.
    pub threshold: f32,
    /// Hold duration on `Intent::HearingLoud`, in ms.
    pub hold_ms: u64,
    /// Was the previous tick above threshold? Used for rising-edge
    /// detection. `None` for "no audio reading yet" (initial / gated).
    last_loud: Option<bool>,
    /// Instant of the most recent fire. `None` between startles.
    /// Drives the hold / re-arm window.
    fired_at: Option<Instant>,
}

impl StartleOnLoud {
    /// Construct with default tuning ([`STARTLE_RMS_THRESHOLD`] /
    /// [`STARTLE_HOLD_MS`]).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            threshold: STARTLE_RMS_THRESHOLD,
            hold_ms: STARTLE_HOLD_MS,
            last_loud: None,
            fired_at: None,
        }
    }

    /// Construct with custom threshold + hold.
    #[must_use]
    pub const fn with_config(threshold: f32, hold_ms: u64) -> Self {
        Self {
            threshold,
            hold_ms,
            last_loud: None,
            fired_at: None,
        }
    }
}

impl Default for StartleOnLoud {
    fn default() -> Self {
        Self::new()
    }
}

/// Is the active hold owned by an explicit-input modifier (touch /
/// body-touch / remote)? Those beat startle.
const fn is_explicit_input(source: Option<OverrideSource>) -> bool {
    matches!(
        source,
        Some(OverrideSource::Touch | OverrideSource::BodyTouch | OverrideSource::Remote)
    )
}

impl Modifier for StartleOnLoud {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "StartleOnLoud",
            description: "Rising-edge perception.audio_rms above STARTLE_RMS_THRESHOLD fires \
                          Surprised + manual hold + HearingLoud intent + Startle chirp. \
                          Defers to explicit-input holds (Touch / BodyTouch / Remote); \
                          overrides Voice / Pickup / Ambient. Holds intent for STARTLE_HOLD_MS \
                          then releases to Idle.",
            phase: Phase::Affect,
            // After WakeOnVoice (-70) so a startle wins over a
            // sustained-voice hold; before AmbientSleepy / EmotionCycle.
            priority: -65,
            reads: &[Field::AudioRms, Field::Autonomy, Field::Intent],
            writes: &[
                Field::Emotion,
                Field::Autonomy,
                Field::Intent,
                Field::ChirpRequest,
            ],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;

        // Release the hold first so a startled-then-quiet sequence
        // returns to Idle even on ticks where audio_rms is None.
        if let Some(fired) = self.fired_at
            && now.saturating_duration_since(fired) >= self.hold_ms
            && matches!(entity.mind.intent, Intent::HearingLoud)
        {
            entity.mind.intent = Intent::Idle;
            self.fired_at = None;
        }

        let Some(rms) = entity.perception.audio_rms else {
            // No reading (boot, or firmware-gated during TX) — treat
            // as "not loud," reset edge state so the next non-None
            // sample can fire.
            self.last_loud = None;
            return;
        };

        let loud = rms > self.threshold;
        let prev_loud = self.last_loud;
        self.last_loud = Some(loud);

        // Rising edge: previous tick was below threshold (or unknown)
        // AND current tick is above. Unknown-then-loud counts as a
        // rising edge so the very first startle after gate-release
        // doesn't get swallowed.
        if !loud || prev_loud == Some(true) {
            return;
        }

        // Suppress while still inside an existing hold of our own.
        if self.fired_at.is_some() {
            return;
        }

        // Defer only to explicit-input holds. Voice/Pickup/Ambient
        // get overridden — startle is meant to break through.
        if let Some(until) = entity.mind.autonomy.manual_until
            && now < until
            && is_explicit_input(entity.mind.autonomy.source)
        {
            return;
        }

        entity.mind.affect.emotion = Emotion::Surprised;
        entity.mind.autonomy.manual_until = Some(now + self.hold_ms);
        entity.mind.autonomy.source = Some(OverrideSource::Startle);
        entity.mind.intent = Intent::HearingLoud;
        entity.voice.chirp_request = Some(ChirpKind::Startle);
        self.fired_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Instant;

    fn at(now_ms: u64) -> Entity {
        let mut e = Entity::default();
        e.tick.now = Instant::from_millis(now_ms);
        e
    }

    fn with_rms(now_ms: u64, rms: Option<f32>) -> Entity {
        let mut e = at(now_ms);
        e.perception.audio_rms = rms;
        e
    }

    #[test]
    fn no_audio_does_not_fire() {
        let mut m = StartleOnLoud::new();
        let mut entity = at(0);
        for t in (0..500).step_by(33) {
            entity.tick.now = Instant::from_millis(t);
            m.update(&mut entity);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert_eq!(entity.mind.intent, Intent::Idle);
        assert!(entity.voice.chirp_request.is_none());
    }

    #[test]
    fn quiet_audio_does_not_fire() {
        let mut m = StartleOnLoud::new();
        let mut entity = with_rms(0, Some(0.01));
        for t in (0..500).step_by(33) {
            entity.tick.now = Instant::from_millis(t);
            m.update(&mut entity);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn rising_edge_above_threshold_fires() {
        let mut m = StartleOnLoud::new();
        let mut entity = with_rms(0, Some(0.01));
        m.update(&mut entity);

        entity.tick.now = Instant::from_millis(33);
        entity.perception.audio_rms = Some(0.6);
        m.update(&mut entity);

        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
        assert_eq!(entity.mind.intent, Intent::HearingLoud);
        assert_eq!(entity.voice.chirp_request, Some(ChirpKind::Startle));
        assert_eq!(entity.mind.autonomy.source, Some(OverrideSource::Startle));
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(33 + STARTLE_HOLD_MS))
        );
    }

    #[test]
    fn unknown_then_loud_counts_as_rising_edge() {
        // Boot path: audio_rms is None, then becomes loud once the
        // firmware publishes a sample. That counts as a rising edge.
        let mut m = StartleOnLoud::new();
        let mut entity = at(0);
        m.update(&mut entity);

        entity.tick.now = Instant::from_millis(33);
        entity.perception.audio_rms = Some(0.6);
        m.update(&mut entity);

        assert_eq!(entity.mind.intent, Intent::HearingLoud);
    }

    #[test]
    fn sustained_loud_only_fires_once_per_burst() {
        let mut m = StartleOnLoud::new();
        let mut entity = with_rms(0, Some(0.01));
        m.update(&mut entity);

        // Rising edge — fires.
        entity.tick.now = Instant::from_millis(33);
        entity.perception.audio_rms = Some(0.6);
        m.update(&mut entity);
        assert_eq!(entity.mind.intent, Intent::HearingLoud);
        let first_until = entity.mind.autonomy.manual_until;

        // Drain the chirp the way firmware would.
        entity.voice.chirp_request = None;

        // Continue loud for several ticks well within the hold.
        for t in (66..600).step_by(33) {
            entity.tick.now = Instant::from_millis(t);
            m.update(&mut entity);
        }
        assert_eq!(
            entity.mind.autonomy.manual_until, first_until,
            "sustained loud must not extend the hold"
        );
        assert!(
            entity.voice.chirp_request.is_none(),
            "sustained loud must not re-chirp"
        );
    }

    #[test]
    fn hold_expires_to_idle() {
        let mut m = StartleOnLoud::new();
        let mut entity = with_rms(0, Some(0.01));
        m.update(&mut entity);
        entity.tick.now = Instant::from_millis(33);
        entity.perception.audio_rms = Some(0.6);
        m.update(&mut entity);
        assert_eq!(entity.mind.intent, Intent::HearingLoud);

        // Quiet down. Step past the hold.
        entity.perception.audio_rms = Some(0.01);
        entity.tick.now = Instant::from_millis(33 + STARTLE_HOLD_MS + 100);
        m.update(&mut entity);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn re_armed_after_quiet_then_loud() {
        let mut m = StartleOnLoud::new();
        let mut entity = with_rms(0, Some(0.01));
        m.update(&mut entity);

        // First fire.
        entity.tick.now = Instant::from_millis(33);
        entity.perception.audio_rms = Some(0.6);
        m.update(&mut entity);
        assert_eq!(entity.mind.intent, Intent::HearingLoud);

        // Quiet beyond hold.
        entity.perception.audio_rms = Some(0.01);
        entity.tick.now = Instant::from_millis(33 + STARTLE_HOLD_MS + 100);
        m.update(&mut entity);
        assert_eq!(entity.mind.intent, Intent::Idle);
        entity.voice.chirp_request = None;
        // Clear the hold so the second fire isn't suppressed by a
        // stale Voice/Startle hold (firmware would have done this).
        entity.mind.autonomy.manual_until = None;

        // Second loud transient.
        entity.tick.now = Instant::from_millis(33 + STARTLE_HOLD_MS + 133);
        entity.perception.audio_rms = Some(0.6);
        m.update(&mut entity);
        assert_eq!(entity.mind.intent, Intent::HearingLoud);
        assert_eq!(entity.voice.chirp_request, Some(ChirpKind::Startle));
    }

    #[test]
    fn touch_hold_blocks_startle() {
        let mut m = StartleOnLoud::new();
        let mut entity = with_rms(0, Some(0.01));
        // Pretend EmotionTouch already claimed the avatar.
        entity.mind.affect.emotion = Emotion::Happy;
        entity.mind.autonomy.manual_until = Some(Instant::from_millis(10_000));
        entity.mind.autonomy.source = Some(OverrideSource::Touch);
        m.update(&mut entity);

        entity.tick.now = Instant::from_millis(33);
        entity.perception.audio_rms = Some(0.6);
        m.update(&mut entity);

        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert_eq!(entity.mind.intent, Intent::Idle);
        assert!(entity.voice.chirp_request.is_none());
    }

    #[test]
    fn voice_hold_does_not_block_startle() {
        // WakeOnVoice has set Happy + Voice hold. A loud transient
        // mid-conversation must still startle the avatar.
        let mut m = StartleOnLoud::new();
        let mut entity = with_rms(0, Some(0.01));
        entity.mind.affect.emotion = Emotion::Happy;
        entity.mind.autonomy.manual_until = Some(Instant::from_millis(10_000));
        entity.mind.autonomy.source = Some(OverrideSource::Voice);
        m.update(&mut entity);

        entity.tick.now = Instant::from_millis(33);
        entity.perception.audio_rms = Some(0.6);
        m.update(&mut entity);

        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
        assert_eq!(entity.mind.intent, Intent::HearingLoud);
        assert_eq!(entity.mind.autonomy.source, Some(OverrideSource::Startle));
    }

    #[test]
    fn at_threshold_does_not_count_as_loud() {
        // The check is `rms > threshold`, so exactly-at-threshold is
        // quiet. Pin so it doesn't drift to `>=`.
        let mut m = StartleOnLoud::new();
        let mut entity = with_rms(0, Some(0.01));
        m.update(&mut entity);
        entity.tick.now = Instant::from_millis(33);
        entity.perception.audio_rms = Some(STARTLE_RMS_THRESHOLD);
        m.update(&mut entity);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn gating_during_loud_resets_edge_for_post_release() {
        // Firmware gates audio_rms = None during TX. Without the edge
        // reset, "loud → None → loud" would not re-fire (prev_loud
        // would still be true). Confirm that the None tick clears the
        // edge so a post-release loud spike fires fresh.
        let mut m = StartleOnLoud::new();
        let mut entity = with_rms(0, Some(0.01));
        m.update(&mut entity);
        entity.tick.now = Instant::from_millis(33);
        entity.perception.audio_rms = Some(0.6);
        m.update(&mut entity);
        assert_eq!(entity.mind.intent, Intent::HearingLoud);
        entity.voice.chirp_request = None;

        // Wait out the hold; gate inactive (audio_rms = None) the
        // whole time (e.g. our own startle chirp is playing).
        for t in (66..(STARTLE_HOLD_MS + 200)).step_by(33) {
            entity.tick.now = Instant::from_millis(t);
            entity.perception.audio_rms = None;
            m.update(&mut entity);
        }
        assert_eq!(entity.mind.intent, Intent::Idle);
        entity.mind.autonomy.manual_until = None;

        // Gate releases; first non-None tick is loud → must fire.
        entity.tick.now = Instant::from_millis(STARTLE_HOLD_MS + 233);
        entity.perception.audio_rms = Some(0.6);
        m.update(&mut entity);
        assert_eq!(entity.mind.intent, Intent::HearingLoud);
        assert_eq!(entity.voice.chirp_request, Some(ChirpKind::Startle));
    }
}
