//! `WakeOnVoice`: flips `Avatar::emotion` to `Happy` when the
//! microphone observes sustained voice activity.
//!
//! ## Detection shape
//!
//! Watches `avatar.audio_rms` (linear, `0.0..=1.0`, normalised against
//! full-scale i16). A tick "counts as loud" when the RMS exceeds
//! [`WAKE_RMS_THRESHOLD`]. Once the consecutive loud-tick count
//! reaches [`WAKE_SUSTAIN_TICKS`] the modifier triggers: emotion goes
//! to `Happy` and `manual_until` is pinned forward by
//! [`WAKE_HOLD_MS`].
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
//! Unlike [`super::AmbientSleepy`] / [`super::LowBatteryEmotion`],
//! this modifier is intended to *override* a sleepy state on voice
//! activity — so it does **not** respect an existing
//! [`Avatar::manual_until`] hold. It runs early in the modifier chain
//! (right after the explicit-input modifiers but before the
//! environmental-override group) and writes its own `manual_until`
//! that the environmental modifiers will then respect.
//!
//! Touch / pickup / remote inputs still win because they run first
//! and set their own holds before this modifier sees the avatar.
//!
//! [`Avatar::manual_until`]: crate::avatar::Avatar::manual_until

use super::Modifier;
use crate::avatar::Avatar;
use crate::clock::Instant;
use crate::emotion::Emotion;

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

/// Modifier that watches [`Avatar::audio_rms`] and writes `Happy`
/// when sustained voice is detected.
///
/// State is a small struct (counter + edge flag); the modifier holds
/// no allocation and is `Copy` so callers can stash it on the stack
/// alongside the other modifier instances.
#[derive(Debug, Clone, Copy)]
pub struct WakeOnVoice {
    /// Linear-RMS threshold above which a tick counts as loud.
    pub threshold: f32,
    /// Consecutive-loud-ticks required to trigger the wake. Saturates
    /// at `u8::MAX` so an extremely-long sustained input doesn't wrap.
    pub sustain_ticks: u8,
    /// Running counter of consecutive loud ticks. Reset on any quiet
    /// tick or after the wake fires.
    consecutive_loud: u8,
    /// Set to `true` on the tick the wake transitions from
    /// not-firing → firing; cleared on the next `update` call.
    /// Callers (e.g. firmware enqueueing a wake chirp) read this
    /// once per tick after `update`.
    just_fired: bool,
}

impl WakeOnVoice {
    /// Construct with the default thresholds ([`WAKE_RMS_THRESHOLD`] /
    /// [`WAKE_SUSTAIN_TICKS`]).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            threshold: WAKE_RMS_THRESHOLD,
            sustain_ticks: WAKE_SUSTAIN_TICKS,
            consecutive_loud: 0,
            just_fired: false,
        }
    }

    /// Construct with custom threshold + sustain.
    #[must_use]
    pub const fn with_config(threshold: f32, sustain_ticks: u8) -> Self {
        Self {
            threshold,
            sustain_ticks,
            consecutive_loud: 0,
            just_fired: false,
        }
    }

    /// `true` on the tick this modifier just transitioned from
    /// not-firing → firing. Cleared at the start of every `update`
    /// call, so consumers should check it once per render tick after
    /// `update` runs.
    ///
    /// Use this to drive one-shot side effects (e.g. enqueueing an
    /// audio chirp) that should accompany the emotional change.
    #[must_use]
    pub const fn just_fired(self) -> bool {
        self.just_fired
    }

    /// Exposed for tests: current loud-tick counter.
    #[cfg(test)]
    const fn consecutive_loud(self) -> u8 {
        self.consecutive_loud
    }
}

impl Default for WakeOnVoice {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for WakeOnVoice {
    fn update(&mut self, avatar: &mut Avatar, now: Instant) {
        // Clear the edge flag at the start of every tick — it only
        // ever signals the *current* tick's transition.
        self.just_fired = false;

        let Some(rms) = avatar.audio_rms else {
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
        // don't extend it on every tick (mirrors AmbientSleepy's
        // rolling-hold behaviour); once it expires, the next still-
        // loud tick rolls a new one.
        if let Some(until) = avatar.manual_until
            && now < until
        {
            return;
        }

        avatar.emotion = Emotion::Happy;
        avatar.manual_until = Some(now + WAKE_HOLD_MS);
        // Reset so the next wake requires a fresh sustained period
        // rather than firing every tick within a single loud burst.
        self.consecutive_loud = 0;
        self.just_fired = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loud_avatar() -> Avatar {
        Avatar {
            audio_rms: Some(0.3),
            ..Avatar::default()
        }
    }

    fn quiet_avatar() -> Avatar {
        Avatar {
            audio_rms: Some(0.001),
            ..Avatar::default()
        }
    }

    #[test]
    fn no_reading_keeps_counter_zero() {
        let mut wake = WakeOnVoice::new();
        let mut avatar = Avatar::default(); // audio_rms = None
        for t in 0..20 {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
        }
        assert_eq!(wake.consecutive_loud(), 0);
        assert_eq!(avatar.emotion, Emotion::Neutral);
    }

    #[test]
    fn quiet_keeps_counter_zero() {
        let mut wake = WakeOnVoice::new();
        let mut avatar = quiet_avatar();
        for t in 0..20 {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
        }
        assert_eq!(wake.consecutive_loud(), 0);
        assert_eq!(avatar.emotion, Emotion::Neutral);
    }

    #[test]
    fn loud_below_sustain_does_not_fire() {
        // sustain_ticks - 1 loud ticks should NOT trigger.
        let mut wake = WakeOnVoice::new();
        let mut avatar = loud_avatar();
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) - 1) {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
        }
        assert_eq!(avatar.emotion, Emotion::Neutral);
        assert!(avatar.manual_until.is_none());
    }

    #[test]
    fn sustained_loud_triggers_happy() {
        let mut wake = WakeOnVoice::new();
        let mut avatar = loud_avatar();
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS)) {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
        }
        assert_eq!(avatar.emotion, Emotion::Happy);
        // Hold deadline = now + WAKE_HOLD_MS at the firing tick.
        let firing_now = Instant::from_millis((u64::from(WAKE_SUSTAIN_TICKS) - 1) * 33);
        assert_eq!(avatar.manual_until, Some(firing_now + WAKE_HOLD_MS));
    }

    #[test]
    fn quiet_tick_resets_counter_mid_burst() {
        // 9 loud ticks, one quiet (resets), then 9 more loud — should
        // not yet fire because each run is below sustain.
        let mut wake = WakeOnVoice::new();
        let mut avatar = loud_avatar();
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) - 1) {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
        }
        avatar.audio_rms = Some(0.001); // quiet
        wake.update(
            &mut avatar,
            Instant::from_millis((u64::from(WAKE_SUSTAIN_TICKS) - 1) * 33),
        );
        assert_eq!(wake.consecutive_loud(), 0);
        avatar.audio_rms = Some(0.3); // loud again
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) - 1) {
            wake.update(
                &mut avatar,
                Instant::from_millis((u64::from(WAKE_SUSTAIN_TICKS) + t) * 33),
            );
        }
        assert_eq!(avatar.emotion, Emotion::Neutral);
    }

    #[test]
    fn fires_only_once_per_sustained_burst() {
        // After firing, the modifier resets its counter. Continued
        // loudness within the hold window short-circuits at the
        // manual_until check; only after the hold expires AND a
        // fresh sustain accumulates does it fire again.
        let mut wake = WakeOnVoice::new();
        let mut avatar = loud_avatar();
        // First fire.
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS)) {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
        }
        let first_deadline = avatar.manual_until;
        // Continue loud through the hold window. Counter should
        // accumulate but the hold should suppress re-firing.
        for t in (u64::from(WAKE_SUSTAIN_TICKS))..(u64::from(WAKE_SUSTAIN_TICKS) + 30) {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
        }
        assert_eq!(avatar.manual_until, first_deadline);
    }

    #[test]
    fn manual_hold_from_other_modifier_blocks_wake() {
        // PickupReaction has already claimed the avatar with Surprised
        // and a long hold. WakeOnVoice still sees sustained loud
        // ticks but stands down — explicit input wins.
        let mut wake = WakeOnVoice::new();
        let pickup_deadline = Instant::from_millis(30_000);
        let mut avatar = Avatar {
            emotion: Emotion::Surprised,
            manual_until: Some(pickup_deadline),
            ..loud_avatar()
        };
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) + 5) {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
        }
        assert_eq!(avatar.emotion, Emotion::Surprised);
        assert_eq!(avatar.manual_until, Some(pickup_deadline));
    }

    #[test]
    fn voice_overrides_sleepy_when_hold_expired() {
        // AmbientSleepy set Sleepy + a hold; the hold has expired.
        // WakeOnVoice should fire on sustained loud audio and write
        // Happy with a fresh hold.
        let mut wake = WakeOnVoice::new();
        let expired = Instant::from_millis(0);
        let mut avatar = Avatar {
            emotion: Emotion::Sleepy,
            manual_until: Some(expired),
            ..loud_avatar()
        };
        // Tick at well past the expiry.
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS)) {
            wake.update(&mut avatar, Instant::from_millis(10_000 + t * 33));
        }
        assert_eq!(avatar.emotion, Emotion::Happy);
        assert!(avatar.manual_until > Some(expired));
    }

    #[test]
    fn counter_saturates_does_not_wrap() {
        // 300 loud ticks (well past u8::MAX) should not panic and
        // should keep the modifier in a consistent state.
        let mut wake = WakeOnVoice::new();
        let mut avatar = loud_avatar();
        for t in 0..300_u64 {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
        }
        // Either fired and reset, or held in saturating state.
        // The contract: no panic and the avatar emotion is one of the
        // expected values (Happy after fire, default Neutral never).
        assert!(matches!(avatar.emotion, Emotion::Happy));
    }

    #[test]
    fn custom_config_takes_effect() {
        // Lower sustain count fires faster.
        let mut wake = WakeOnVoice::with_config(0.05, 3);
        let mut avatar = loud_avatar();
        for t in 0..3 {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
        }
        assert_eq!(avatar.emotion, Emotion::Happy);
    }

    #[test]
    fn just_fired_set_on_trigger_tick_only() {
        // Sustained loudness fires on the Nth tick. just_fired is
        // true on that tick, false on the (N+1)th.
        let mut wake = WakeOnVoice::new();
        let mut avatar = loud_avatar();
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) - 1) {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
            assert!(!wake.just_fired(), "fired early on tick {t}");
        }
        // Trigger tick.
        wake.update(
            &mut avatar,
            Instant::from_millis((u64::from(WAKE_SUSTAIN_TICKS) - 1) * 33),
        );
        assert!(wake.just_fired(), "did not fire on threshold tick");
        // Next tick: still loud, but inside hold window — not a fresh
        // fire, so just_fired clears.
        wake.update(
            &mut avatar,
            Instant::from_millis(u64::from(WAKE_SUSTAIN_TICKS) * 33),
        );
        assert!(!wake.just_fired(), "stuck-fired across consecutive ticks");
    }

    #[test]
    fn just_fired_clears_when_quiet_tick_resets_counter() {
        // A quiet tick mid-burst resets the counter and zeroes
        // just_fired (which was already false).
        let mut wake = WakeOnVoice::new();
        let mut avatar = loud_avatar();
        wake.update(&mut avatar, Instant::from_millis(0));
        avatar.audio_rms = Some(0.001);
        wake.update(&mut avatar, Instant::from_millis(33));
        assert!(!wake.just_fired());
        assert_eq!(wake.consecutive_loud(), 0);
    }

    #[test]
    fn just_fired_not_set_when_blocked_by_manual_hold() {
        // Sustained loud audio under an existing manual hold should
        // *not* set just_fired — emotion / hold are unchanged so
        // there's no transition to chirp about.
        let mut wake = WakeOnVoice::new();
        let mut avatar = Avatar {
            emotion: Emotion::Surprised,
            manual_until: Some(Instant::from_millis(30_000)),
            ..loud_avatar()
        };
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) + 5) {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
        }
        assert!(!wake.just_fired());
        assert_eq!(avatar.emotion, Emotion::Surprised);
    }

    #[test]
    fn at_threshold_does_not_count_as_loud() {
        // The check is `rms > threshold`, so exactly-at-threshold is
        // treated as quiet. Pin this so it doesn't drift to `>=`.
        let mut wake = WakeOnVoice::new();
        let mut avatar = Avatar {
            audio_rms: Some(WAKE_RMS_THRESHOLD),
            ..Avatar::default()
        };
        for t in 0..(u64::from(WAKE_SUSTAIN_TICKS) + 5) {
            wake.update(&mut avatar, Instant::from_millis(t * 33));
        }
        assert_eq!(avatar.emotion, Emotion::Neutral);
        assert_eq!(wake.consecutive_loud(), 0);
    }
}
