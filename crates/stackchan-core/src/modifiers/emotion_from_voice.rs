//! `EmotionFromVoice`: flips `entity.mind.affect.emotion` to `Happy` when
//! the microphone observes sustained voice activity.
//!
//! ## Detection shape
//!
//! Watches `entity.perception.audio_rms` (linear, `0.0..=1.0`,
//! normalised against full-scale i16). A tick "counts as loud" when
//! the RMS exceeds [`WAKE_RMS_THRESHOLD`]. Once the consecutive
//! loud-tick count reaches [`WAKE_SUSTAIN_TICKS`] the modifier
//! triggers: emotion goes to `Happy`, `manual_until` is pinned forward
//! by [`WAKE_HOLD_MS`], and `voice.chirp_request` is set to
//! [`ChirpKind::Wake`] so the firmware can play a wake chirp.
//!
//! Falling below the threshold for any tick resets the counter — the
//! detector requires *uninterrupted* loudness, not cumulative. This
//! keeps it robust against single-frame transients (a cough, a chair
//! squeak) that would otherwise drift the counter upward.
//!
//! Unknown audio (`audio_rms = None`, before the first publish) is
//! treated as silence and never triggers.
//!
//! ## Coordination with the other emotion modifiers
//!
//! Unlike [`super::EmotionFromAmbient`] / [`super::EmotionFromBattery`],
//! this modifier is intended to *override* a sleepy state on voice
//! activity — so it does **not** respect an existing
//! `entity.mind.autonomy.manual_until` hold. It runs early in the
//! modifier chain (right after the explicit-input modifiers but before
//! the environmental-override group) and writes its own `manual_until`
//! that the environmental modifiers will then respect.
//!
//! Touch / pickup / remote inputs still win because they run first
//! and set their own holds before this modifier sees the avatar.

use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::modifier::Modifier;
use crate::voice::ChirpKind;

/// Linear RMS threshold for the "loud" classification. `0.05 ≈ -26
/// dBFS`, well above ambient room noise on the CoreS3 mic but below
/// normal speaking volume at typical desktop distance.
pub const WAKE_RMS_THRESHOLD: f32 = 0.05;

/// Consecutive loud ticks required to fire the wake.
///
/// At a 30 FPS render cadence (33 ms / tick), 10 ticks ≈ 330 ms —
/// long enough to ignore single coughs / chair squeaks, short enough
/// that the avatar reacts within a single spoken word.
pub const WAKE_SUSTAIN_TICKS: u8 = 10;

/// How long the wake hold pins `Happy` once set, in ms.
///
/// 5 s mirrors the other environmental holds. Within the window the
/// modifier short-circuits its own re-fire; once expired, a still-
/// loud audio stream rolls a fresh hold forward.
pub const WAKE_HOLD_MS: u64 = 5_000;

/// Modifier that watches `entity.perception.audio_rms` and writes
/// `Happy` when sustained voice is detected.
///
/// State is a small struct (counter only); the modifier holds no
/// allocation and is `Copy` so callers can stash it on the stack
/// alongside the other modifier instances.
#[derive(Debug, Clone, Copy)]
pub struct EmotionFromVoice {
    /// Linear-RMS threshold above which a tick counts as loud.
    pub threshold: f32,
    /// Consecutive-loud-ticks required to trigger the wake. Saturates
    /// at `u8::MAX` so an extremely-long sustained input doesn't wrap.
    pub sustain_ticks: u8,
    /// Running counter of consecutive loud ticks. Reset on any quiet
    /// tick or after the wake fires.
    consecutive_loud: u8,
}

impl EmotionFromVoice {
    /// Construct with the default thresholds ([`WAKE_RMS_THRESHOLD`] /
    /// [`WAKE_SUSTAIN_TICKS`]).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            threshold: WAKE_RMS_THRESHOLD,
            sustain_ticks: WAKE_SUSTAIN_TICKS,
            consecutive_loud: 0,
        }
    }

    /// Construct with custom threshold + sustain.
    #[must_use]
    pub const fn with_config(threshold: f32, sustain_ticks: u8) -> Self {
        Self {
            threshold,
            sustain_ticks,
            consecutive_loud: 0,
        }
    }

    /// Exposed for tests: current loud-tick counter.
    #[cfg(test)]
    const fn consecutive_loud(self) -> u8 {
        self.consecutive_loud
    }
}

impl Default for EmotionFromVoice {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for EmotionFromVoice {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "EmotionFromVoice",
            description: "Sustained perception.audio_rms above threshold flips emotion to Happy \
                          (wake from Sleepy). Sets voice.chirp_request = Wake on the rising edge.",
            phase: Phase::Affect,
            priority: -70,
            reads: &[Field::AudioRms, Field::Autonomy],
            writes: &[Field::Emotion, Field::Autonomy, Field::ChirpRequest],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;

        let Some(rms) = entity.perception.audio_rms else {
            // No reading yet — treat as silent, reset counter.
            self.consecutive_loud = 0;
            return;
        };

        if rms > self.threshold {
            self.consecutive_loud = self.consecutive_loud.saturating_add(1);
        } else {
            // Any quiet tick resets — we want sustained loudness, not
            // cumulative.
            self.consecutive_loud = 0;
            return;
        }

        if self.consecutive_loud < self.sustain_ticks {
            return;
        }

        // Threshold-and-sustain met. If a hold is already active,
        // don't extend it on every tick (mirrors EmotionFromAmbient's
        // rolling-hold behaviour); once it expires, the next still-
        // loud tick rolls a new one.
        if let Some(until) = entity.mind.autonomy.manual_until
            && now < until
        {
            return;
        }

        entity.mind.affect.emotion = Emotion::Happy;
        entity.mind.autonomy.manual_until = Some(now + WAKE_HOLD_MS);
        entity.mind.autonomy.source = Some(crate::mind::OverrideSource::Voice);
        entity.voice.chirp_request = Some(ChirpKind::Wake);
        // Reset so the next wake requires a fresh sustained period
        // rather than firing every tick within a single loud burst.
        self.consecutive_loud = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Instant;

    fn loud_avatar() -> Entity {
        let mut e = Entity::default();
        e.perception.audio_rms = Some(0.3);
        e
    }

    fn quiet_avatar() -> Entity {
        let mut e = Entity::default();
        e.perception.audio_rms = Some(0.001);
        e
    }

    #[test]
    fn no_reading_keeps_counter_zero() {
        let mut wake = EmotionFromVoice::new();
        let mut entity = Entity::default(); // audio_rms = None
        for t in 0..20 {
            entity.tick.now = Instant::from_millis(t * 33);
            wake.update(&mut entity);
        }
        assert_eq!(wake.consecutive_loud(), 0);
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
    }

    #[test]
    fn quiet_keeps_counter_zero() {
        let mut wake = EmotionFromVoice::new();
        let mut entity = quiet_avatar();
        for t in 0..20 {
            entity.tick.now = Instant::from_millis(t * 33);
            wake.update(&mut entity);
        }
        assert_eq!(wake.consecutive_loud(), 0);
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
    }

    #[test]
    fn loud_below_sustain_does_not_fire() {
        // sustain_ticks - 1 loud ticks should NOT trigger.
        let mut wake = EmotionFromVoice::new();
        let mut entity = loud_avatar();
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) - 1) {
            entity.tick.now = Instant::from_millis(t * 33);
            wake.update(&mut entity);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert!(entity.mind.autonomy.manual_until.is_none());
    }

    #[test]
    fn sustained_loud_triggers_happy() {
        let mut wake = EmotionFromVoice::new();
        let mut entity = loud_avatar();
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS)) {
            entity.tick.now = Instant::from_millis(t * 33);
            wake.update(&mut entity);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        // Hold deadline = now + WAKE_HOLD_MS at the firing tick.
        let firing_now = Instant::from_millis((u64::from(WAKE_SUSTAIN_TICKS) - 1) * 33);
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(firing_now + WAKE_HOLD_MS)
        );
        assert_eq!(entity.voice.chirp_request, Some(ChirpKind::Wake));
    }

    #[test]
    fn quiet_tick_resets_counter_mid_burst() {
        // 9 loud ticks, one quiet (resets), then 9 more loud — should
        // not yet fire because each run is below sustain.
        let mut wake = EmotionFromVoice::new();
        let mut entity = loud_avatar();
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) - 1) {
            entity.tick.now = Instant::from_millis(t * 33);
            wake.update(&mut entity);
        }
        entity.perception.audio_rms = Some(0.001); // quiet
        entity.tick.now = Instant::from_millis((u64::from(WAKE_SUSTAIN_TICKS) - 1) * 33);
        wake.update(&mut entity);
        assert_eq!(wake.consecutive_loud(), 0);
        entity.perception.audio_rms = Some(0.3); // loud again
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) - 1) {
            entity.tick.now = Instant::from_millis((u64::from(WAKE_SUSTAIN_TICKS) + t) * 33);
            wake.update(&mut entity);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
    }

    #[test]
    fn fires_only_once_per_sustained_burst() {
        // After firing, the modifier resets its counter. Continued
        // loudness within the hold window short-circuits at the
        // manual_until check; only after the hold expires AND a
        // fresh sustain accumulates does it fire again.
        let mut wake = EmotionFromVoice::new();
        let mut entity = loud_avatar();
        // First fire.
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS)) {
            entity.tick.now = Instant::from_millis(t * 33);
            wake.update(&mut entity);
        }
        let first_deadline = entity.mind.autonomy.manual_until;
        // Continue loud through the hold window. Counter should
        // accumulate but the hold should suppress re-firing.
        for t in (u64::from(WAKE_SUSTAIN_TICKS))..(u64::from(WAKE_SUSTAIN_TICKS) + 30) {
            entity.tick.now = Instant::from_millis(t * 33);
            wake.update(&mut entity);
        }
        assert_eq!(entity.mind.autonomy.manual_until, first_deadline);
    }

    #[test]
    fn manual_hold_from_other_modifier_blocks_wake() {
        // EmotionFromIntent has already claimed the avatar with Surprised
        // and a long hold. EmotionFromVoice still sees sustained loud
        // ticks but stands down — explicit input wins.
        let mut wake = EmotionFromVoice::new();
        let pickup_deadline = Instant::from_millis(30_000);
        let mut entity = {
            let mut e = loud_avatar();
            e.mind.affect.emotion = Emotion::Surprised;
            e.mind.autonomy.manual_until = Some(pickup_deadline);
            e
        };
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) + 5) {
            entity.tick.now = Instant::from_millis(t * 33);
            wake.update(&mut entity);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
        assert_eq!(entity.mind.autonomy.manual_until, Some(pickup_deadline));
        assert!(
            entity.voice.chirp_request.is_none(),
            "blocked wake must not request a chirp"
        );
    }

    #[test]
    fn voice_overrides_sleepy_when_hold_expired() {
        // EmotionFromAmbient set Sleepy + a hold; the hold has expired.
        // EmotionFromVoice should fire on sustained loud audio and write
        // Happy with a fresh hold.
        let mut wake = EmotionFromVoice::new();
        let expired = Instant::from_millis(0);
        let mut entity = {
            let mut e = loud_avatar();
            e.mind.affect.emotion = Emotion::Sleepy;
            e.mind.autonomy.manual_until = Some(expired);
            e
        };
        // Tick at well past the expiry.
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS)) {
            entity.tick.now = Instant::from_millis(10_000 + t * 33);
            wake.update(&mut entity);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert!(entity.mind.autonomy.manual_until > Some(expired));
    }

    #[test]
    fn counter_saturates_does_not_wrap() {
        // 300 loud ticks (well past u8::MAX) should not panic and
        // should keep the modifier in a consistent state.
        let mut wake = EmotionFromVoice::new();
        let mut entity = loud_avatar();
        for t in 0..300_u64 {
            entity.tick.now = Instant::from_millis(t * 33);
            wake.update(&mut entity);
        }
        // Either fired and reset, or held in saturating state.
        // The contract: no panic and the avatar emotion is one of the
        // expected values (Happy after fire, default Neutral never).
        assert!(matches!(entity.mind.affect.emotion, Emotion::Happy));
    }

    #[test]
    fn custom_config_takes_effect() {
        // Lower sustain count fires faster.
        let mut wake = EmotionFromVoice::with_config(0.05, 3);
        let mut entity = loud_avatar();
        for t in 0..3 {
            entity.tick.now = Instant::from_millis(t * 33);
            wake.update(&mut entity);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
    }

    #[test]
    fn chirp_request_set_on_trigger_tick_only() {
        // Sustained loudness fires on the Nth tick. chirp_request is
        // Some on that tick; firmware drains it. On the (N+1)th tick
        // (still-loud, still-held), no fresh request.
        let mut wake = EmotionFromVoice::new();
        let mut entity = loud_avatar();
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) - 1) {
            entity.tick.now = Instant::from_millis(t * 33);
            wake.update(&mut entity);
            assert!(
                entity.voice.chirp_request.is_none(),
                "fired early on tick {t}"
            );
        }
        // Trigger tick.
        entity.tick.now = Instant::from_millis((u64::from(WAKE_SUSTAIN_TICKS) - 1) * 33);
        wake.update(&mut entity);
        assert_eq!(entity.voice.chirp_request, Some(ChirpKind::Wake));
        // Simulate firmware draining the request.
        entity.voice.chirp_request = None;
        // Next tick: still loud, but inside hold window — no fresh fire.
        entity.tick.now = Instant::from_millis(u64::from(WAKE_SUSTAIN_TICKS) * 33);
        wake.update(&mut entity);
        assert!(
            entity.voice.chirp_request.is_none(),
            "stuck-fired across consecutive ticks"
        );
    }

    #[test]
    fn quiet_tick_after_fire_resets_counter() {
        let mut wake = EmotionFromVoice::new();
        let mut entity = loud_avatar();
        entity.tick.now = Instant::from_millis(0);
        wake.update(&mut entity);
        entity.voice.chirp_request = None;
        entity.perception.audio_rms = Some(0.001);
        entity.tick.now = Instant::from_millis(33);
        wake.update(&mut entity);
        assert!(entity.voice.chirp_request.is_none());
        assert_eq!(wake.consecutive_loud(), 0);
    }

    #[test]
    fn at_threshold_does_not_count_as_loud() {
        // The check is `rms > threshold`, so exactly-at-threshold is
        // treated as quiet. Pin this so it doesn't drift to `>=`.
        let mut wake = EmotionFromVoice::new();
        let mut entity = {
            let mut e = Entity::default();
            e.perception.audio_rms = Some(WAKE_RMS_THRESHOLD);
            e
        };
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) + 5) {
            entity.tick.now = Instant::from_millis(t * 33);
            wake.update(&mut entity);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert_eq!(wake.consecutive_loud(), 0);
    }
}
