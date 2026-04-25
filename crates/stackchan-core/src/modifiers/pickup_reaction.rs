//! `PickupReaction`: motion-reactive modifier that flips
//! `Avatar::emotion` to `Surprised` when a pickup / drop is detected
//! via the accelerometer.
//!
//! ## Detection shape
//!
//! Each tick reads `avatar.accel_g` (written by the firmware IMU
//! task at ~100 Hz). The pickup condition is:
//!
//! > `|accel_magnitude - 1.0|` (g units) exceeds
//! > [`PICKUP_DEVIATION_G`] and the condition holds for at least
//! > [`PICKUP_DEBOUNCE_MS`].
//!
//! "Deviation from 1 g" catches both a lift (magnitude > 1 g during
//! the upward acceleration phase) and a drop / freefall (magnitude
//! collapses toward 0 g). Slow, steady motion stays near 1 g and does
//! not trigger; perfect stillness also stays at 1 g and does not
//! trigger.
//!
//! ## Coordination with the rest of the emotion pipeline
//!
//! Pickup is a *reflex*; touch is *intentional*. When the user has
//! already tapped to pin an emotion via [`super::EmotionTouch`] (so
//! `avatar.manual_until` is set to a future instant), `PickupReaction`
//! stands down — the user's explicit choice wins. Once the manual
//! hold has expired (and [`super::EmotionTouch::update`] has cleared
//! the field), pickup is eligible to fire again.
//!
//! Within the modifier stack, `PickupReaction` runs immediately after
//! `EmotionTouch` and before `EmotionCycle` so the ordering is:
//!
//! 1. `EmotionTouch` consumes pending taps + clears expired holds.
//! 2. `PickupReaction` reads `accel_g`, fires if eligible + unheld.
//! 3. `EmotionCycle` advances only if `manual_until` is clear.

use super::{MANUAL_HOLD_MS, Modifier};
use crate::avatar::Avatar;
use crate::clock::Instant;
use crate::emotion::Emotion;

/// How far `|accel|` must deviate from the resting 1 g value, in g
/// units, to count as "in motion."
///
/// `0.5 g` catches hand lifts (typical peak ≈ 1.5–2 g), drops
/// (magnitude collapses toward 0 g during freefall), and firm taps,
/// while ignoring desk vibration and the `IdleSway` servo wobble.
pub const PICKUP_DEVIATION_G: f32 = 0.5;

/// How long the deviation must persist before a pickup fires, in
/// milliseconds.
///
/// 50 ms = 5 samples at the BMI270's default 100 Hz ODR. Long enough
/// to reject single-sample spikes (electrical noise, a fingertip
/// bump), short enough that a real pickup feels instantaneous.
pub const PICKUP_DEBOUNCE_MS: u64 = 50;

/// Emotion set on a pickup event. Hardcoded to `Surprised`: semantically
/// the clearest "unexpected physical event" reaction for this avatar.
const PICKUP_EMOTION: Emotion = Emotion::Surprised;

/// Modifier that watches [`Avatar::accel_g`] for pickup events.
#[derive(Debug, Clone, Copy, Default)]
pub struct PickupReaction {
    /// First time the magnitude-deviation threshold was crossed in
    /// the current above-threshold run. `None` when the reading is
    /// at or below threshold. Reset to `None` each time a sample
    /// falls back under the threshold or after a fire.
    above_since: Option<Instant>,
    /// `true` once the current above-threshold run has already fired
    /// a pickup event. Prevents re-fires while a single sustained
    /// pickup is still in progress; cleared when the reading returns
    /// below threshold.
    fired_this_run: bool,
    /// Set to `true` on the tick the modifier just transitioned from
    /// not-firing → firing (i.e. wrote `Surprised` and pinned
    /// `manual_until`). Cleared on the next `update` call.
    just_fired: bool,
}

impl PickupReaction {
    /// Construct a modifier with no in-flight above-threshold state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            above_since: None,
            fired_this_run: false,
            just_fired: false,
        }
    }

    /// `true` on the tick this modifier just transitioned from
    /// not-firing → firing. Cleared at the start of every `update`,
    /// so consumers should check it once per render tick after
    /// `update` runs.
    ///
    /// Use this to drive one-shot side effects (e.g. enqueueing a
    /// pickup chirp) that should accompany the emotional change.
    /// Note that a pickup blocked by an existing `manual_until` from
    /// touch / remote does *not* set this flag — there's no
    /// transition to chirp about.
    #[must_use]
    pub const fn just_fired(self) -> bool {
        self.just_fired
    }
}

/// Squared magnitude of a 3-axis g-scaled acceleration vector.
///
/// Avoids `sqrt` entirely: the pickup condition `||v| - 1| > k` is
/// equivalent to `|v|² > (1 + k)² ∨ |v|² < (1 − k)²` for any
/// `k ∈ (0, 1)`, which the caller evaluates against the
/// [`REST_BAND_SQUARED`] inclusive range. Keeps `stackchan-core`
/// `libm`-free like every other modifier in the crate.
///
/// Uses plain `x*x + y*y + z*z` (not `mul_add`) because `f32::mul_add`
/// requires libm on `no_std` targets. Clippy's `suboptimal_flops` lint
/// is silenced locally for this reason.
#[allow(
    clippy::suboptimal_flops,
    reason = "f32::mul_add needs libm on no_std; stackchan-core stays libm-free"
)]
fn magnitude_squared((x, y, z): (f32, f32, f32)) -> f32 {
    x * x + y * y + z * z
}

/// `(1 − k)²` (lower bound of rest). A `|accel|²` below this
/// corresponds to an acceleration magnitude ≤ `1 − k` g (a drop /
/// near-freefall).
const BELOW_SQUARED: f32 = (1.0 - PICKUP_DEVIATION_G) * (1.0 - PICKUP_DEVIATION_G);
/// `(1 + k)²` (upper bound of rest). A `|accel|²` above this
/// corresponds to an acceleration magnitude ≥ `1 + k` g (a lift / hit).
const ABOVE_SQUARED: f32 = (1.0 + PICKUP_DEVIATION_G) * (1.0 + PICKUP_DEVIATION_G);
/// Closed interval of squared magnitudes treated as "at rest."
/// A sample *outside* this range counts as in-motion.
const REST_BAND_SQUARED: core::ops::RangeInclusive<f32> = BELOW_SQUARED..=ABOVE_SQUARED;

impl Modifier for PickupReaction {
    fn update(&mut self, avatar: &mut Avatar, now: Instant) {
        // Clear the edge flag at the start of every tick — it only
        // ever signals the *current* tick's transition.
        self.just_fired = false;

        let m2 = magnitude_squared(avatar.accel_g);
        // Out-of-rest-band: magnitude outside `(1±k) g` squared.
        let above_threshold = !REST_BAND_SQUARED.contains(&m2);

        if !above_threshold {
            // Fell back to rest — arm for the next pickup.
            self.above_since = None;
            self.fired_this_run = false;
            return;
        }

        // Above threshold this tick. Anchor the run start if we don't
        // have one yet.
        let started = if let Some(t) = self.above_since {
            t
        } else {
            self.above_since = Some(now);
            now
        };

        // Debounce: must persist ≥ PICKUP_DEBOUNCE_MS.
        if now.saturating_duration_since(started) < PICKUP_DEBOUNCE_MS {
            return;
        }

        // Already fired for this run; wait for rest before re-arming.
        if self.fired_this_run {
            return;
        }

        // User input wins. If an `EmotionTouch`-set hold is active,
        // leave it alone. This respects "explicit beats reflexive"
        // without having to know who set the hold.
        if let Some(until) = avatar.manual_until
            && now < until
        {
            // Mark the run as "handled" so we don't keep checking the
            // hold on every tick during a prolonged pickup.
            self.fired_this_run = true;
            return;
        }

        avatar.emotion = PICKUP_EMOTION;
        avatar.manual_until = Some(now + MANUAL_HOLD_MS);
        self.fired_this_run = true;
        self.just_fired = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: put the avatar at rest (1 g on Z, zero gyro).
    fn at_rest() -> Avatar {
        Avatar::default()
    }

    /// Helper: advance accel so `|accel| = 1 + delta` in `g`.
    ///
    /// Pure Z-axis shift keeps the math predictable in tests.
    fn set_accel(avatar: &mut Avatar, magnitude_g: f32) {
        avatar.accel_g = (0.0, 0.0, magnitude_g);
    }

    #[test]
    fn resting_input_never_fires() {
        let mut avatar = at_rest();
        let mut pickup = PickupReaction::new();

        for t in (0..500).step_by(10) {
            pickup.update(&mut avatar, Instant::from_millis(t));
        }
        assert_eq!(avatar.emotion, Emotion::Neutral);
        assert!(avatar.manual_until.is_none());
    }

    #[test]
    fn single_spike_under_debounce_does_not_fire() {
        let mut avatar = at_rest();
        let mut pickup = PickupReaction::new();

        set_accel(&mut avatar, 2.0);
        pickup.update(&mut avatar, Instant::from_millis(0));
        // Only 30 ms of above-threshold before returning to rest —
        // under the 50 ms debounce.
        pickup.update(&mut avatar, Instant::from_millis(30));
        set_accel(&mut avatar, 1.0);
        pickup.update(&mut avatar, Instant::from_millis(40));

        assert_eq!(avatar.emotion, Emotion::Neutral);
        assert!(avatar.manual_until.is_none());
    }

    #[test]
    fn sustained_lift_fires_after_debounce() {
        let mut avatar = at_rest();
        let mut pickup = PickupReaction::new();

        set_accel(&mut avatar, 1.8);
        pickup.update(&mut avatar, Instant::from_millis(0));
        pickup.update(&mut avatar, Instant::from_millis(30));
        // 60 ms of sustained pickup crosses the 50 ms debounce.
        pickup.update(&mut avatar, Instant::from_millis(60));

        assert_eq!(avatar.emotion, Emotion::Surprised);
        assert_eq!(
            avatar.manual_until,
            Some(Instant::from_millis(60 + MANUAL_HOLD_MS)),
        );
    }

    #[test]
    fn drop_fires_same_as_lift() {
        // Freefall reads |accel| ≈ 0 g, i.e. deviation ≈ -1 g.
        let mut avatar = at_rest();
        let mut pickup = PickupReaction::new();

        set_accel(&mut avatar, 0.1);
        pickup.update(&mut avatar, Instant::from_millis(0));
        pickup.update(&mut avatar, Instant::from_millis(30));
        pickup.update(&mut avatar, Instant::from_millis(60));

        assert_eq!(avatar.emotion, Emotion::Surprised);
    }

    #[test]
    fn prolonged_pickup_does_not_refire() {
        let mut avatar = at_rest();
        let mut pickup = PickupReaction::new();

        set_accel(&mut avatar, 1.8);
        for t in (0..500).step_by(10) {
            pickup.update(&mut avatar, Instant::from_millis(t));
        }

        // Capture the manual_until at first fire so we can confirm it
        // doesn't get pushed forward on every subsequent tick.
        assert_eq!(avatar.emotion, Emotion::Surprised);
        assert!(avatar.manual_until.is_some(), "fire must set hold");
        let first_until = avatar.manual_until;

        // Another 200 ms of sustained motion — still no re-fire.
        for t in (500..700).step_by(10) {
            pickup.update(&mut avatar, Instant::from_millis(t));
        }
        assert_eq!(
            avatar.manual_until, first_until,
            "prolonged motion must not keep extending the hold",
        );
    }

    #[test]
    fn second_pickup_after_rest_and_expiry_fires_again() {
        let mut avatar = at_rest();
        let mut pickup = PickupReaction::new();

        // First pickup fires.
        set_accel(&mut avatar, 1.8);
        pickup.update(&mut avatar, Instant::from_millis(0));
        pickup.update(&mut avatar, Instant::from_millis(60));
        assert_eq!(avatar.emotion, Emotion::Surprised);

        // Settle, clear hold manually (in real code `EmotionTouch`
        // clears it on expiry).
        set_accel(&mut avatar, 1.0);
        pickup.update(&mut avatar, Instant::from_millis(5_000));
        avatar.manual_until = None;
        avatar.emotion = Emotion::Neutral;

        // Second pickup — run the full debounce again.
        set_accel(&mut avatar, 1.8);
        pickup.update(&mut avatar, Instant::from_millis(10_000));
        pickup.update(&mut avatar, Instant::from_millis(10_060));
        assert_eq!(avatar.emotion, Emotion::Surprised);
    }

    #[test]
    fn touch_hold_blocks_pickup() {
        let mut avatar = at_rest();
        // Simulate touch having pinned `Happy` — same shape
        // `EmotionTouch` would produce.
        avatar.emotion = Emotion::Happy;
        avatar.manual_until = Some(Instant::from_millis(MANUAL_HOLD_MS));
        let mut pickup = PickupReaction::new();

        set_accel(&mut avatar, 1.8);
        pickup.update(&mut avatar, Instant::from_millis(0));
        pickup.update(&mut avatar, Instant::from_millis(60));

        assert_eq!(
            avatar.emotion,
            Emotion::Happy,
            "touch-set emotion must survive a concurrent pickup",
        );
        assert_eq!(
            avatar.manual_until,
            Some(Instant::from_millis(MANUAL_HOLD_MS)),
            "touch-set hold deadline must not be overwritten",
        );
    }

    #[test]
    fn pickup_fires_after_touch_hold_expires() {
        let mut avatar = at_rest();
        avatar.emotion = Emotion::Happy;
        avatar.manual_until = Some(Instant::from_millis(1_000));
        let mut pickup = PickupReaction::new();

        // Pickup begins mid-hold — suppressed.
        set_accel(&mut avatar, 1.8);
        pickup.update(&mut avatar, Instant::from_millis(0));
        pickup.update(&mut avatar, Instant::from_millis(60));
        assert_eq!(avatar.emotion, Emotion::Happy);

        // Hold expires (in real code, `EmotionTouch` clears the
        // field). Re-arm by dropping back to rest so the detector
        // sees a fresh rising edge.
        set_accel(&mut avatar, 1.0);
        pickup.update(&mut avatar, Instant::from_millis(1_500));
        avatar.manual_until = None;

        set_accel(&mut avatar, 1.8);
        pickup.update(&mut avatar, Instant::from_millis(2_000));
        pickup.update(&mut avatar, Instant::from_millis(2_060));
        assert_eq!(avatar.emotion, Emotion::Surprised);
    }

    #[test]
    fn just_fired_set_only_on_trigger_tick() {
        let mut avatar = at_rest();
        let mut pickup = PickupReaction::new();

        // Pre-trigger: nothing fired yet.
        set_accel(&mut avatar, 1.8);
        pickup.update(&mut avatar, Instant::from_millis(0));
        assert!(!pickup.just_fired());
        pickup.update(&mut avatar, Instant::from_millis(30));
        assert!(!pickup.just_fired());

        // Trigger tick (60 ms past the 50 ms debounce).
        pickup.update(&mut avatar, Instant::from_millis(60));
        assert!(pickup.just_fired(), "expected fire on trigger tick");

        // Subsequent above-threshold ticks within the same run do
        // *not* re-fire (fired_this_run is set), so just_fired clears.
        pickup.update(&mut avatar, Instant::from_millis(100));
        assert!(!pickup.just_fired());
    }

    #[test]
    fn just_fired_not_set_when_blocked_by_existing_hold() {
        // EmotionTouch has already pinned the avatar with a long hold.
        // PickupReaction sees the threshold cross + debounce + active
        // hold, marks fired_this_run as handled, but does not write
        // a new emotion or set just_fired — there's no transition to
        // chirp about.
        let mut avatar = Avatar {
            emotion: Emotion::Happy,
            manual_until: Some(Instant::from_millis(30_000)),
            ..at_rest()
        };
        let mut pickup = PickupReaction::new();

        set_accel(&mut avatar, 1.8);
        pickup.update(&mut avatar, Instant::from_millis(0));
        pickup.update(&mut avatar, Instant::from_millis(60));

        assert_eq!(avatar.emotion, Emotion::Happy);
        assert!(!pickup.just_fired());
    }
}
