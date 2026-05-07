//! [`HeadFromBodyGesture`] — applies a randomized head-pose nudge
//! when the back-of-head touch strip fires a reaction-eligible
//! gesture (`Press` / `SwipeForward` / `SwipeBackward`).
//!
//! Mirrors the upstream `m5stack/StackChan/firmware/main/stackchan/modifiers/head_pet.h`
//! reaction shape: a brief randomized head lift / tilt, then a slower
//! decay back to whatever the upstream modifier stack wanted. Reads
//! `mind.last_gesture` (set by [`super::IntentFromBodyTouch`] on every
//! gesture transition) and runs an attack/decay envelope over a
//! reaction window; subsequent gestures inside the window restart
//! the envelope from the new firing instant.
//!
//! ## Composition
//!
//! Phase `Motion`, priority `35` — runs *after* the rest of the head
//! stack (`HeadFromEmotion` 10, `HeadFromAttention` 20, `HeadFromIntent`
//! 30) so the pet reaction lands on top of any concurrent attention
//! follow / emotion bias / startle recoil. Composes additively via
//! diff-and-undo: the modifier subtracts its previous contribution
//! to recover the upstream pose, adds its new contribution, clamps
//! to mechanical range, then stores the post-clamp effective delta
//! for the next tick.
//!
//! ## Randomisation
//!
//! No RNG dependency — the randomized pan/tilt direction is derived
//! deterministically from the firing instant's millisecond bits via a
//! splitmix-style hash. Same instant produces the same nudge so the
//! sim's golden assertions stay stable.
//!
//! ## Why not in `HeadFromIntent`
//!
//! `HeadFromIntent` reacts to `Intent::Startled` (single edge) with a
//! fixed asymmetric recoil. The pet reaction has different timing
//! (longer hold, slower decay), different triggering events
//! (gesture transitions, not intent change), and a randomized nudge
//! direction — different envelope, different write trigger. Sharing
//! a modifier would force one kind of reaction to fight the other's
//! state machine.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::head::Pose;
use crate::mind::BodyGesture;
use crate::modifier::Modifier;

/// Total reaction window, in milliseconds.
///
/// 1500 ms covers the upstream factory firmware's "head lift / tilt
/// for ~1.5–2.5 s" beat at the lower end. Long enough to read as a
/// reaction without dragging the head off whatever it was tracking.
pub const HEADPET_REACTION_TOTAL_MS: u64 = 1_500;

/// Attack duration, in milliseconds — how fast the envelope ramps in.
///
/// 80 ms is roughly two render frames, fast enough that the reaction
/// reads as instantaneous to the audience.
pub const HEADPET_REACTION_ATTACK_MS: u64 = 80;

/// Maximum pan offset, in degrees. Applied with random sign.
pub const HEADPET_PAN_DEG: f32 = 6.0;

/// Maximum tilt offset, in degrees. Applied with random sign.
pub const HEADPET_TILT_DEG: f32 = 5.0;

/// Modifier that applies a randomized head-pose nudge on body-touch
/// gesture transitions. See module docs.
#[derive(Debug, Clone, Copy)]
pub struct HeadFromBodyGesture {
    /// Total reaction window, in milliseconds.
    pub total_ms: u64,
    /// Attack duration, in milliseconds.
    pub attack_ms: u64,
    /// Maximum pan magnitude, in degrees.
    pub pan_deg: f32,
    /// Maximum tilt magnitude, in degrees.
    pub tilt_deg: f32,
    /// Wall-clock instant the active reaction was anchored. `None`
    /// once the envelope has fully decayed.
    started_at: Option<Instant>,
    /// Pan offset of the active reaction, sign-randomized. Cached
    /// because re-deriving it every tick from the anchor would couple
    /// the random output to per-tick interpolation.
    target_pan_deg: f32,
    /// Tilt offset of the active reaction, sign-randomized.
    target_tilt_deg: f32,
    /// Last gesture timestamp consumed; used as the rising-edge gate
    /// against [`crate::mind::Mind::last_gesture`].
    last_consumed_at: Option<Instant>,
    /// Pan delta this modifier applied on the previous tick — needed
    /// for the diff-and-undo composition.
    last_applied_pan_deg: f32,
    /// Tilt delta this modifier applied on the previous tick.
    last_applied_tilt_deg: f32,
}

impl HeadFromBodyGesture {
    /// Construct with the documented default constants.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            total_ms: HEADPET_REACTION_TOTAL_MS,
            attack_ms: HEADPET_REACTION_ATTACK_MS,
            pan_deg: HEADPET_PAN_DEG,
            tilt_deg: HEADPET_TILT_DEG,
            started_at: None,
            target_pan_deg: 0.0,
            target_tilt_deg: 0.0,
            last_consumed_at: None,
            last_applied_pan_deg: 0.0,
            last_applied_tilt_deg: 0.0,
        }
    }
}

impl Default for HeadFromBodyGesture {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for HeadFromBodyGesture {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "HeadFromBodyGesture",
            description: "On Press / SwipeForward / SwipeBackward edges (mind.last_gesture), \
                          applies a randomized head-pose nudge with attack/decay envelope. \
                          Composes additively after the rest of the head stack via diff-and-undo. \
                          Mirrors m5stack/StackChan head_pet.h's reaction shape.",
            phase: Phase::Motion,
            priority: 35,
            reads: &[Field::Gesture, Field::HeadPose],
            writes: &[Field::HeadPose],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;

        // Check for a fresh, reaction-eligible gesture and re-anchor.
        if let Some((gesture, at)) = entity.mind.last_gesture
            && self.last_consumed_at != Some(at)
            && reaction_eligible(gesture)
        {
            self.last_consumed_at = Some(at);
            self.started_at = Some(at);
            let (pan_sign, tilt_sign) = random_signs(at);
            self.target_pan_deg = pan_sign * self.pan_deg;
            self.target_tilt_deg = tilt_sign * self.tilt_deg;
        }

        // Compute the current envelope amplitude (0..=1) for the
        // active reaction, if any. Clear the anchor once the envelope
        // has fully decayed.
        let amplitude = match self.started_at {
            Some(start) => {
                let elapsed = now.saturating_duration_since(start);
                if elapsed >= self.total_ms {
                    self.started_at = None;
                    0.0
                } else {
                    envelope(elapsed, self.attack_ms, self.total_ms)
                }
            }
            None => 0.0,
        };

        let target_pan = self.target_pan_deg * amplitude;
        let target_tilt = self.target_tilt_deg * amplitude;

        // Diff-and-undo: subtract our previous applied contribution
        // to recover the upstream pose, add the new contribution,
        // clamp, then store the post-clamp effective delta so next
        // tick's diff-and-undo lands on the right reference point.
        let upstream_pan = entity.motor.head_pose.pan_deg - self.last_applied_pan_deg;
        let upstream_tilt = entity.motor.head_pose.tilt_deg - self.last_applied_tilt_deg;
        let combined = Pose::new(upstream_pan + target_pan, upstream_tilt + target_tilt).clamped();
        self.last_applied_pan_deg = combined.pan_deg - upstream_pan;
        self.last_applied_tilt_deg = combined.tilt_deg - upstream_tilt;
        entity.motor.head_pose = combined;
    }
}

/// Which gestures earn a head-pose reaction. Release is excluded
/// because the pose is already on its way back to the post-touch
/// resting state via the rest of the modifier stack — adding a nudge
/// at release just chatters the servo.
const fn reaction_eligible(gesture: BodyGesture) -> bool {
    matches!(
        gesture,
        BodyGesture::Press { .. } | BodyGesture::SwipeForward | BodyGesture::SwipeBackward
    )
}

/// Compute the envelope amplitude `[0, 1]` for `elapsed` into a
/// reaction of length `total_ms` with attack `attack_ms`.
///
/// Linear ramp up over the attack, then linear decay back to zero
/// over the remainder. Deliberately simple — the audience reads the
/// silhouette of the motion, not the easing curve, and a non-linear
/// curve would buy nothing visible at this duration.
#[allow(
    clippy::cast_precision_loss,
    reason = "u64→f32 over time-window magnitudes well under 2^24"
)]
fn envelope(elapsed: u64, attack_ms: u64, total_ms: u64) -> f32 {
    if elapsed >= total_ms {
        return 0.0;
    }
    if elapsed < attack_ms {
        return (elapsed as f32) / (attack_ms.max(1) as f32);
    }
    let decay_ms = total_ms - attack_ms;
    let decay_elapsed = elapsed - attack_ms;
    if decay_ms == 0 {
        return 0.0;
    }
    1.0 - (decay_elapsed as f32 / decay_ms as f32)
}

/// Derive `(pan_sign, tilt_sign)` from a firing instant via a
/// splitmix-style hash on the millisecond count. Output values are
/// `±1.0`; same `at` always produces the same pair so the sim's
/// golden assertions stay stable.
const fn random_signs(at: Instant) -> (f32, f32) {
    let ms = at.as_millis();
    // SplitMix64 finalizer — cheap, well-distributed bit mixer.
    let mut z = ms.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    let pan_sign = if z & 1 == 0 { 1.0 } else { -1.0 };
    let tilt_sign = if (z >> 1) & 1 == 0 { 1.0 } else { -1.0 };
    (pan_sign, tilt_sign)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test-only: bit-exact assertions on our own envelope math"
)]
mod tests {
    use super::*;

    fn entity_with_gesture(gesture: BodyGesture, at_ms: u64) -> Entity {
        let mut e = Entity::default();
        e.mind.last_gesture = Some((gesture, Instant::from_millis(at_ms)));
        e.tick.now = Instant::from_millis(at_ms);
        e
    }

    #[test]
    fn idle_with_no_gesture_writes_no_pose() {
        let mut m = HeadFromBodyGesture::new();
        let mut entity = Entity::default();
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose, Pose::default());
    }

    #[test]
    fn release_does_not_anchor_reaction() {
        let mut m = HeadFromBodyGesture::new();
        let mut entity = entity_with_gesture(BodyGesture::Release, 100);
        m.update(&mut entity);
        // No envelope was started, so nothing should be applied.
        assert_eq!(entity.motor.head_pose, Pose::default());
        assert!(m.started_at.is_none());
    }

    #[test]
    fn press_anchors_envelope() {
        let mut m = HeadFromBodyGesture::new();
        let mut entity = entity_with_gesture(
            BodyGesture::Press {
                left: 0,
                centre: 3,
                right: 0,
            },
            100,
        );
        m.update(&mut entity);
        assert_eq!(m.started_at, Some(Instant::from_millis(100)));
        // Envelope at elapsed=0 is exactly 0 → no head displacement
        // on the entry tick. Displacement appears next tick.
        assert_eq!(entity.motor.head_pose.pan_deg, 0.0);
    }

    #[test]
    fn envelope_peaks_at_attack_then_decays() {
        let mut m = HeadFromBodyGesture::new();
        let mut entity = entity_with_gesture(BodyGesture::SwipeForward, 0);
        m.update(&mut entity);

        // At elapsed = attack_ms, amplitude should be ≈ 1.0 →
        // |head_pose| ≈ pan_deg / tilt_deg.
        entity.tick.now = Instant::from_millis(HEADPET_REACTION_ATTACK_MS);
        // Same gesture timestamp; the modifier shouldn't re-anchor.
        m.update(&mut entity);
        let pan_at_peak = entity.motor.head_pose.pan_deg.abs();
        assert!(
            (pan_at_peak - HEADPET_PAN_DEG).abs() < 0.01,
            "expected peak pan ≈ {HEADPET_PAN_DEG}, got {pan_at_peak}"
        );

        // Halfway through decay → roughly half amplitude.
        let half_decay_ms = HEADPET_REACTION_ATTACK_MS
            + (HEADPET_REACTION_TOTAL_MS - HEADPET_REACTION_ATTACK_MS) / 2;
        entity.tick.now = Instant::from_millis(half_decay_ms);
        m.update(&mut entity);
        let pan_at_half = entity.motor.head_pose.pan_deg.abs();
        assert!(
            pan_at_half < pan_at_peak,
            "expected pan to decay below peak ({pan_at_half} vs {pan_at_peak})"
        );
        assert!(
            pan_at_half > 0.5 * pan_at_peak * 0.5,
            "expected pan still meaningful at half-decay ({pan_at_half})"
        );
    }

    #[test]
    fn envelope_returns_to_zero_after_total() {
        let mut m = HeadFromBodyGesture::new();
        let mut entity = entity_with_gesture(BodyGesture::SwipeForward, 0);
        m.update(&mut entity);

        // Past total_ms, amplitude is 0 and the anchor has cleared.
        entity.tick.now = Instant::from_millis(HEADPET_REACTION_TOTAL_MS + 100);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.pan_deg, 0.0);
        assert_eq!(entity.motor.head_pose.tilt_deg, 0.0);
        assert!(m.started_at.is_none());
    }

    #[test]
    fn second_gesture_within_window_restarts_envelope() {
        let mut m = HeadFromBodyGesture::new();
        let mut entity = entity_with_gesture(BodyGesture::SwipeForward, 0);
        m.update(&mut entity);
        let first_anchor = m.started_at;

        // A second gesture at t=400, well inside the 1500 ms window —
        // the modifier should re-anchor to the new instant.
        entity.mind.last_gesture = Some((BodyGesture::SwipeBackward, Instant::from_millis(400)));
        entity.tick.now = Instant::from_millis(400);
        m.update(&mut entity);
        assert_eq!(m.started_at, Some(Instant::from_millis(400)));
        assert_ne!(m.started_at, first_anchor);
    }

    #[test]
    fn random_signs_are_deterministic() {
        // Same instant → same signs. Locks the determinism contract
        // the sim's golden assertions rely on.
        let a = random_signs(Instant::from_millis(100));
        let b = random_signs(Instant::from_millis(100));
        assert_eq!(a, b);
    }

    #[test]
    fn random_signs_are_pm_one() {
        // Sweep a few timestamps; outputs are always ±1.
        for ms in [0u64, 1, 2, 3, 100, 12_345, 999_999] {
            let (pan, tilt) = random_signs(Instant::from_millis(ms));
            assert!(pan.abs() == 1.0);
            assert!(tilt.abs() == 1.0);
        }
    }

    #[test]
    fn diff_and_undo_composes_with_upstream() {
        // Pick an upstream pose that lives well inside the
        // mechanical range so the diff-and-undo math doesn't have
        // its delta absorbed by `Pose::clamped`. Tilt is asymmetric
        // (`MIN_TILT_DEG = 0.0`, `MAX_TILT_DEG = 30.0`) so a
        // negative tilt would be clamped to 0 and the test would
        // fail spuriously.
        let upstream = Pose::new(3.0, 10.0);

        let mut m = HeadFromBodyGesture::new();
        let mut entity = Entity::default();
        // Upstream wrote a pose first. With no gesture, our delta is
        // zero and the upstream pose passes through unchanged.
        entity.motor.head_pose = upstream;
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose, upstream);

        // Anchor a gesture, advance to peak; the modifier should add
        // its randomized offset on top of the upstream pose.
        entity.mind.last_gesture = Some((BodyGesture::SwipeForward, Instant::from_millis(0)));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        // At elapsed=0 amplitude is 0; pose still equals upstream.
        assert_eq!(entity.motor.head_pose.pan_deg, upstream.pan_deg);

        // Tick to peak — re-apply the upstream pose first (this
        // models the firmware's render loop where upstream modifiers
        // run before this one each frame), then expect our nudge on
        // top.
        entity.motor.head_pose = upstream;
        entity.tick.now = Instant::from_millis(HEADPET_REACTION_ATTACK_MS);
        m.update(&mut entity);
        let dx = entity.motor.head_pose.pan_deg - upstream.pan_deg;
        let dy = entity.motor.head_pose.tilt_deg - upstream.tilt_deg;
        assert!(
            dx.abs() > 0.5 && dy.abs() > 0.5,
            "expected non-zero nudge layered on upstream pose, got dx={dx} dy={dy}"
        );
    }
}
