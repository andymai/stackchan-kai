//! `PickupReaction`: motion-reactive modifier that flips
//! `entity.mind.affect.emotion` to `Surprised` when a pickup / drop is detected
//! via the accelerometer.
//!
//! ## Detection shape
//!
//! Each tick reads `entity.perception.accel_g` (written by the firmware IMU
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
//! `entity.mind.autonomy.manual_until` is set to a future instant), `PickupReaction`
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

use super::MANUAL_HOLD_MS;
use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::modifier::Modifier;

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
/// the clearest "unexpected physical event" reaction for this entity.
const PICKUP_EMOTION: Emotion = Emotion::Surprised;

/// Modifier that watches `entity.perception.accel_g` for pickup events.
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
}

impl PickupReaction {
    /// Construct a modifier with no in-flight above-threshold state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            above_since: None,
            fired_this_run: false,
        }
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
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "PickupReaction",
            description: "Detects pickup edges from perception.accel_g (out-of-rest-band debounced \
                          for ~PICKUP_DEBOUNCE_MS), flips emotion to Surprised, and sets \
                          voice.chirp_request = Pickup for firmware audio.",
            phase: Phase::Affect,
            priority: -80,
            reads: &[Field::AccelG, Field::Autonomy, Field::Emotion],
            writes: &[Field::Emotion, Field::Autonomy, Field::ChirpRequest],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;

        let m2 = magnitude_squared(entity.perception.accel_g);
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
        let started = *self.above_since.get_or_insert(now);

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
        if let Some(until) = entity.mind.autonomy.manual_until
            && now < until
        {
            // Mark the run as "handled" so we don't keep checking the
            // hold on every tick during a prolonged pickup.
            self.fired_this_run = true;
            return;
        }

        entity.mind.affect.emotion = PICKUP_EMOTION;
        entity.mind.autonomy.manual_until = Some(now + MANUAL_HOLD_MS);
        entity.mind.autonomy.source = Some(crate::mind::OverrideSource::Pickup);
        entity.voice.chirp_request = Some(crate::voice::ChirpKind::Pickup);
        self.fired_this_run = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: put the avatar at rest (1 g on Z, zero gyro).
    fn at_rest() -> Entity {
        Entity::default()
    }

    /// Helper: advance accel so `|accel| = 1 + delta` in `g`.
    ///
    /// Pure Z-axis shift keeps the math predictable in tests.
    fn set_accel(entity: &mut Entity, magnitude_g: f32) {
        entity.perception.accel_g = (0.0, 0.0, magnitude_g);
    }

    #[test]
    fn resting_input_never_fires() {
        let mut entity = at_rest();
        let mut pickup = PickupReaction::new();

        for t in (0..500).step_by(10) {
            entity.tick.now = Instant::from_millis(t);
            pickup.update(&mut entity);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert!(entity.mind.autonomy.manual_until.is_none());
    }

    #[test]
    fn single_spike_under_debounce_does_not_fire() {
        let mut entity = at_rest();
        let mut pickup = PickupReaction::new();

        set_accel(&mut entity, 2.0);
        entity.tick.now = Instant::from_millis(0);
        pickup.update(&mut entity);
        // Only 30 ms of above-threshold before returning to rest —
        // under the 50 ms debounce.
        entity.tick.now = Instant::from_millis(30);
        pickup.update(&mut entity);
        set_accel(&mut entity, 1.0);
        entity.tick.now = Instant::from_millis(40);
        pickup.update(&mut entity);

        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert!(entity.mind.autonomy.manual_until.is_none());
    }

    #[test]
    fn sustained_lift_fires_after_debounce() {
        let mut entity = at_rest();
        let mut pickup = PickupReaction::new();

        set_accel(&mut entity, 1.8);
        entity.tick.now = Instant::from_millis(0);
        pickup.update(&mut entity);
        entity.tick.now = Instant::from_millis(30);
        pickup.update(&mut entity);
        // 60 ms of sustained pickup crosses the 50 ms debounce.
        entity.tick.now = Instant::from_millis(60);
        pickup.update(&mut entity);

        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(60 + MANUAL_HOLD_MS)),
        );
    }

    #[test]
    fn drop_fires_same_as_lift() {
        // Freefall reads |accel| ≈ 0 g, i.e. deviation ≈ -1 g.
        let mut entity = at_rest();
        let mut pickup = PickupReaction::new();

        set_accel(&mut entity, 0.1);
        entity.tick.now = Instant::from_millis(0);
        pickup.update(&mut entity);
        entity.tick.now = Instant::from_millis(30);
        pickup.update(&mut entity);
        entity.tick.now = Instant::from_millis(60);
        pickup.update(&mut entity);

        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
    }

    #[test]
    fn prolonged_pickup_does_not_refire() {
        let mut entity = at_rest();
        let mut pickup = PickupReaction::new();

        set_accel(&mut entity, 1.8);
        for t in (0..500).step_by(10) {
            entity.tick.now = Instant::from_millis(t);
            pickup.update(&mut entity);
        }

        // Capture the manual_until at first fire so we can confirm it
        // doesn't get pushed forward on every subsequent tick.
        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
        assert!(
            entity.mind.autonomy.manual_until.is_some(),
            "fire must set hold"
        );
        let first_until = entity.mind.autonomy.manual_until;

        // Another 200 ms of sustained motion — still no re-fire.
        for t in (500..700).step_by(10) {
            entity.tick.now = Instant::from_millis(t);
            pickup.update(&mut entity);
        }
        assert_eq!(
            entity.mind.autonomy.manual_until, first_until,
            "prolonged motion must not keep extending the hold",
        );
    }

    #[test]
    fn second_pickup_after_rest_and_expiry_fires_again() {
        let mut entity = at_rest();
        let mut pickup = PickupReaction::new();

        // First pickup fires.
        set_accel(&mut entity, 1.8);
        entity.tick.now = Instant::from_millis(0);
        pickup.update(&mut entity);
        entity.tick.now = Instant::from_millis(60);
        pickup.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);

        // Settle, clear hold manually (in real code `EmotionTouch`
        // clears it on expiry).
        set_accel(&mut entity, 1.0);
        entity.tick.now = Instant::from_millis(5_000);
        pickup.update(&mut entity);
        entity.mind.autonomy.manual_until = None;
        entity.mind.affect.emotion = Emotion::Neutral;

        // Second pickup — run the full debounce again.
        set_accel(&mut entity, 1.8);
        entity.tick.now = Instant::from_millis(10_000);
        pickup.update(&mut entity);
        entity.tick.now = Instant::from_millis(10_060);
        pickup.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
    }

    #[test]
    fn touch_hold_blocks_pickup() {
        let mut entity = at_rest();
        // Simulate touch having pinned `Happy` — same shape
        // `EmotionTouch` would produce.
        entity.mind.affect.emotion = Emotion::Happy;
        entity.mind.autonomy.manual_until = Some(Instant::from_millis(MANUAL_HOLD_MS));
        let mut pickup = PickupReaction::new();

        set_accel(&mut entity, 1.8);
        entity.tick.now = Instant::from_millis(0);
        pickup.update(&mut entity);
        entity.tick.now = Instant::from_millis(60);
        pickup.update(&mut entity);

        assert_eq!(
            entity.mind.affect.emotion,
            Emotion::Happy,
            "touch-set emotion must survive a concurrent pickup",
        );
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(MANUAL_HOLD_MS)),
            "touch-set hold deadline must not be overwritten",
        );
    }

    #[test]
    fn pickup_fires_after_touch_hold_expires() {
        let mut entity = at_rest();
        entity.mind.affect.emotion = Emotion::Happy;
        entity.mind.autonomy.manual_until = Some(Instant::from_millis(1_000));
        let mut pickup = PickupReaction::new();

        // Pickup begins mid-hold — suppressed.
        set_accel(&mut entity, 1.8);
        entity.tick.now = Instant::from_millis(0);
        pickup.update(&mut entity);
        entity.tick.now = Instant::from_millis(60);
        pickup.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);

        // Hold expires (in real code, `EmotionTouch` clears the
        // field). Re-arm by dropping back to rest so the detector
        // sees a fresh rising edge.
        set_accel(&mut entity, 1.0);
        entity.tick.now = Instant::from_millis(1_500);
        pickup.update(&mut entity);
        entity.mind.autonomy.manual_until = None;

        set_accel(&mut entity, 1.8);
        entity.tick.now = Instant::from_millis(2_000);
        pickup.update(&mut entity);
        entity.tick.now = Instant::from_millis(2_060);
        pickup.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
    }

    #[test]
    fn chirp_request_set_only_on_trigger_tick() {
        let mut entity = at_rest();
        let mut pickup = PickupReaction::new();

        // Pre-trigger: nothing fired yet.
        set_accel(&mut entity, 1.8);
        entity.tick.now = Instant::from_millis(0);
        pickup.update(&mut entity);
        assert!(entity.voice.chirp_request.is_none());
        entity.tick.now = Instant::from_millis(30);
        pickup.update(&mut entity);
        assert!(entity.voice.chirp_request.is_none());

        // Trigger tick (60 ms past the 50 ms debounce).
        entity.tick.now = Instant::from_millis(60);
        pickup.update(&mut entity);
        assert_eq!(
            entity.voice.chirp_request,
            Some(crate::voice::ChirpKind::Pickup),
            "expected pickup chirp on trigger tick"
        );

        // Subsequent above-threshold ticks within the same run do not
        // re-fire. Firmware drains the request after dispatch; we
        // simulate that here.
        entity.voice.chirp_request = None;
        entity.tick.now = Instant::from_millis(100);
        pickup.update(&mut entity);
        assert!(entity.voice.chirp_request.is_none());
    }

    #[test]
    fn no_chirp_when_blocked_by_existing_hold() {
        // EmotionTouch has already pinned the avatar with a long hold.
        // PickupReaction sees the threshold cross + debounce + active
        // hold, marks fired_this_run as handled, but does not write a
        // new emotion or request a chirp.
        let mut entity = {
            let mut e = at_rest();
            e.mind.affect.emotion = Emotion::Happy;
            e.mind.autonomy.manual_until = Some(Instant::from_millis(30_000));
            e
        };
        let mut pickup = PickupReaction::new();

        set_accel(&mut entity, 1.8);
        entity.tick.now = Instant::from_millis(0);
        pickup.update(&mut entity);
        entity.tick.now = Instant::from_millis(60);
        pickup.update(&mut entity);

        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert!(entity.voice.chirp_request.is_none());
    }
}
