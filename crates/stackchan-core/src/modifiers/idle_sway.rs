//! Idle head sway: a slow pan/tilt wander that keeps the head from freezing.
//!
//! Produces a [`Pose`] on `entity.motor.head_pose` using two independent
//! triangle waves at incommensurable periods — pan at ~11 s, tilt at ~7 s
//! by default.
//! The mismatched periods make the head trace a non-repeating Lissajous-ish
//! path that reads as "alive" without looking like a preprogrammed sweep.
//!
//! Triangle waves (rather than `sin`) keep `stackchan-core` dependency-free:
//! no `libm`, no `micromath`, same philosophy as [`Breath`](super::Breath).
//! At the low amplitudes used here (~4° pan, ~2.5° tilt), the triangle
//! corners are well below the servo's own mechanical smoothing; a follow-up
//! release could swap in smoothstep or real trig for larger sweeps.
//!
//! ## Composition
//!
//! Contributes additively to `entity.motor.head_pose` using the same
//! diff-and-undo pattern [`Breath`](super::Breath) uses for vertical
//! offset: each tick subtracts the previous contribution and adds the
//! new one. That way modifiers running *before* `IdleSway` (e.g. a
//! future head-pose source that sets an absolute target) are not
//! silently clobbered, and modifiers running *after* (e.g.
//! [`HeadFromEmotion`](super::HeadFromEmotion), which biases on top) see the
//! already-swayed pose without `IdleSway` overwriting their work on the
//! next tick.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::head::Pose;
use crate::modifier::Modifier;

/// Default pan wander period, in milliseconds (~11 s).
pub const DEFAULT_PAN_PERIOD_MS: u64 = 11_000;
/// Default tilt wander period, in milliseconds (~7 s).
///
/// Chosen coprime-ish with the pan period so pan+tilt don't re-align on
/// a short cycle; the LCM is roughly 77 s, long enough to read as
/// non-repeating.
pub const DEFAULT_TILT_PERIOD_MS: u64 = 7_000;
/// Default pan amplitude in degrees.
///
/// Tests composing [`IdleSway`] with other head-pose modifiers can
/// use this to bound combined output honestly instead of hardcoding
/// the literal.
pub const DEFAULT_PAN_AMPLITUDE_DEG: f32 = 4.0;
/// Default tilt amplitude in degrees.
///
/// Smaller than pan because most StackChan bases have tighter mechanical
/// headroom on the tilt axis (pan servo sits under the tilt linkage).
pub const DEFAULT_TILT_AMPLITUDE_DEG: f32 = 2.5;

/// Modifier that contributes a slow, two-axis triangle sway to
/// `entity.motor.head_pose`.
///
/// Composition is additive via diff-and-undo: each tick subtracts the
/// previous contribution before adding the new one, so upstream pose
/// writes survive and downstream modifiers can bias without the sway
/// overwriting them on the next tick.
#[derive(Debug, Clone, Copy)]
pub struct IdleSway {
    /// Milliseconds per full pan sweep (left → right → left).
    pan_period_ms: u64,
    /// Milliseconds per full tilt sweep.
    tilt_period_ms: u64,
    /// Peak pan amplitude in degrees.
    pan_amplitude_deg: f32,
    /// Peak tilt amplitude in degrees.
    tilt_amplitude_deg: f32,
    /// Pan contribution **as actually applied** on the previous tick
    /// (post-clamp), subtracted before writing the new contribution.
    /// Storing the *effective* contribution rather than the *intended*
    /// one keeps diff-and-undo accurate when `Pose::clamped` truncates
    /// our request — without this, a clamped negative half-cycle would
    /// leak into the next positive half as a permanent bias, doubling
    /// the apparent amplitude.
    last_pan_deg: f32,
    /// Tilt contribution as actually applied on the previous tick. See
    /// [`Self::last_pan_deg`] — same diff-and-undo correctness reason.
    last_tilt_deg: f32,
}

impl IdleSway {
    /// Default sway parameters: ±4° pan over ~11 s, ±2.5° tilt over ~7 s.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pan_period_ms: DEFAULT_PAN_PERIOD_MS,
            tilt_period_ms: DEFAULT_TILT_PERIOD_MS,
            pan_amplitude_deg: DEFAULT_PAN_AMPLITUDE_DEG,
            tilt_amplitude_deg: DEFAULT_TILT_AMPLITUDE_DEG,
            last_pan_deg: 0.0,
            last_tilt_deg: 0.0,
        }
    }

    /// Construct with custom periods + amplitudes. Amplitudes greater than
    /// `MAX_PAN_DEG` / `MAX_TILT_DEG` will be clipped by [`Pose::clamped`]
    /// each tick; setting them larger than safe range has no effect on
    /// pose output but still costs compute.
    #[must_use]
    pub const fn with_params(
        pan_period_ms: u64,
        tilt_period_ms: u64,
        pan_amplitude_deg: f32,
        tilt_amplitude_deg: f32,
    ) -> Self {
        Self {
            pan_period_ms,
            tilt_period_ms,
            pan_amplitude_deg,
            tilt_amplitude_deg,
            last_pan_deg: 0.0,
            last_tilt_deg: 0.0,
        }
    }

    /// Sample a unity-amplitude triangle wave at time `now`.
    ///
    /// Returns a value in `[-1.0, +1.0]` that rises from -1 to +1 across
    /// the first half of `period_ms`, then falls back. Returns `0.0` if
    /// `period_ms == 0` (wave is undefined).
    fn unit_triangle(period_ms: u64, now: Instant) -> f32 {
        if period_ms == 0 {
            return 0.0;
        }
        let phase_ms = now.as_millis() % period_ms;
        // Both values are in [0, period_ms], which fits f32 cleanly for any
        // realistic wander period (< 2^24 ms ≈ 4.6 hours).
        #[allow(
            clippy::cast_precision_loss,
            reason = "period_ms stays well under 2^24, the f32 mantissa limit"
        )]
        let phase = phase_ms as f32 / period_ms as f32;
        // `mul_add` is the pedantic suggestion but it routes through the
        // `fma` intrinsic, which in `no_std` needs libm. A plain multiply
        // + add is accurate to within one ULP at these magnitudes and
        // keeps the crate dep-free.
        #[allow(
            clippy::suboptimal_flops,
            reason = "avoiding libm dep — precision is ample for ±MAX_*_DEG servo output"
        )]
        if phase < 0.5 {
            phase * 4.0 - 1.0
        } else {
            3.0 - phase * 4.0
        }
    }
}

impl Default for IdleSway {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for IdleSway {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "IdleSway",
            description: "Slow two-axis triangle-wave wander on motor.head_pose so the head \
                          looks alive at rest. Composes additively with upstream pose writes.",
            phase: Phase::Motion,
            priority: 0,
            reads: &[Field::HeadPose],
            writes: &[Field::HeadPose],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let pan = Self::unit_triangle(self.pan_period_ms, now) * self.pan_amplitude_deg;
        let tilt = Self::unit_triangle(self.tilt_period_ms, now) * self.tilt_amplitude_deg;
        let upstream_pan = entity.motor.head_pose.pan_deg - self.last_pan_deg;
        let upstream_tilt = entity.motor.head_pose.tilt_deg - self.last_tilt_deg;
        let new_pose = Pose::new(upstream_pan + pan, upstream_tilt + tilt).clamped();
        self.last_pan_deg = new_pose.pan_deg - upstream_pan;
        self.last_tilt_deg = new_pose.tilt_deg - upstream_tilt;
        entity.motor.head_pose = new_pose;
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare bit-exact outputs of our own triangle math, \
              not results of accumulated FP arithmetic"
)]
mod tests {
    use super::*;
    use crate::Entity;
    use crate::head::{MAX_PAN_DEG, MAX_TILT_DEG};

    /// Advance an `IdleSway` across `duration_ms` at `step_ms` granularity,
    /// returning the sequence of poses observed.
    fn sample(sway: &mut IdleSway, duration_ms: u64, step_ms: u64) -> Vec<Pose> {
        let mut entity = Entity::default();
        let steps = duration_ms / step_ms.max(1);
        (0..=steps)
            .map(|i| {
                entity.tick.now = Instant::from_millis(i * step_ms);
                sway.update(&mut entity);
                entity.motor.head_pose
            })
            .collect()
    }

    #[test]
    fn triangle_valley_at_phase_zero() {
        assert_eq!(IdleSway::unit_triangle(1000, Instant::from_millis(0)), -1.0);
    }

    #[test]
    fn triangle_peak_at_half_phase() {
        assert_eq!(
            IdleSway::unit_triangle(1000, Instant::from_millis(500)),
            1.0
        );
    }

    #[test]
    fn triangle_returns_to_valley_after_full_period() {
        assert_eq!(
            IdleSway::unit_triangle(1000, Instant::from_millis(1000)),
            -1.0
        );
    }

    #[test]
    fn zero_period_returns_zero() {
        assert_eq!(IdleSway::unit_triangle(0, Instant::from_millis(123)), 0.0);
    }

    #[test]
    fn pose_stays_within_configured_amplitude() {
        let mut sway = IdleSway::with_params(1_000, 700, 5.0, 3.0);
        // Sample every 10 ms across three full pan cycles.
        for pose in sample(&mut sway, 3_000, 10) {
            assert!(
                pose.pan_deg.abs() <= 5.0 + 0.01,
                "pan {} exceeded amplitude",
                pose.pan_deg
            );
            assert!(
                pose.tilt_deg.abs() <= 3.0 + 0.01,
                "tilt {} exceeded amplitude",
                pose.tilt_deg
            );
        }
    }

    #[test]
    fn pose_is_clamped_to_safe_range_even_with_huge_amplitudes() {
        // User asked for ±90° pan — Pose::clamped should hold us to ±MAX.
        let mut sway = IdleSway::with_params(1_000, 700, 90.0, 90.0);
        for pose in sample(&mut sway, 1_000, 1) {
            assert!(pose.pan_deg.abs() <= MAX_PAN_DEG);
            assert!(pose.tilt_deg.abs() <= MAX_TILT_DEG);
        }
    }

    #[test]
    fn default_sway_crosses_zero_in_each_axis() {
        // Over enough time, the head must pass through centered pose on
        // each axis (proof it isn't stuck at an extremum).
        let mut sway = IdleSway::new();
        let poses = sample(&mut sway, 12_000, 50); // 12 s covers pan+tilt
        let pan_crossed =
            poses.iter().any(|p| p.pan_deg >= 0.0) && poses.iter().any(|p| p.pan_deg <= 0.0);
        let tilt_crossed =
            poses.iter().any(|p| p.tilt_deg >= 0.0) && poses.iter().any(|p| p.tilt_deg <= 0.0);
        assert!(pan_crossed, "pan never crossed zero");
        assert!(tilt_crossed, "tilt never crossed zero");
    }

    #[test]
    fn trajectory_is_smooth_between_ticks() {
        // At 30 FPS (~33 ms steps), deltas between successive poses should
        // be bounded — no teleports. Bound is (amplitude/half_period)*step.
        let mut sway = IdleSway::new();
        let poses = sample(&mut sway, 5_000, 33);
        // Compute the slope bound via integer arithmetic, then cast once.
        #[allow(
            clippy::cast_precision_loss,
            reason = "DEFAULT_PAN_PERIOD_MS = 11_000 is well under the f32 mantissa limit"
        )]
        let max_pan_delta_per_step =
            DEFAULT_PAN_AMPLITUDE_DEG * 2.0 / (DEFAULT_PAN_PERIOD_MS as f32 / 2.0) * 33.0;
        for window in poses.windows(2) {
            let delta = (window[1].pan_deg - window[0].pan_deg).abs();
            // Double the bound to allow for the corner-of-triangle tick
            // that crosses phase 0.5.
            assert!(
                delta <= max_pan_delta_per_step * 2.0,
                "pan delta {delta}° between ticks exceeds bound"
            );
        }
    }

    #[test]
    fn sway_composes_additively_with_upstream_writes() {
        let mut sway = IdleSway::new();
        let mut entity = Entity::default();
        let upstream_pan = 1.5;
        let upstream_tilt = -0.5;

        for i in 0..100 {
            entity.motor.head_pose = Pose::new(upstream_pan, upstream_tilt);
            entity.tick.now = Instant::from_millis(i * 33);
            sway.update(&mut entity);

            let sway_pan = entity.motor.head_pose.pan_deg - upstream_pan;
            let sway_tilt = entity.motor.head_pose.tilt_deg - upstream_tilt;
            assert!(
                sway_pan.abs() <= DEFAULT_PAN_AMPLITUDE_DEG + 0.01,
                "sway contribution {sway_pan}° exceeds amplitude"
            );
            assert!(sway_tilt.abs() <= DEFAULT_TILT_AMPLITUDE_DEG + 0.01);
        }
    }

    #[test]
    fn pose_matches_state_read_back_from_entity() {
        let mut sway = IdleSway::new();
        let mut entity = Entity::default();
        entity.tick.now = Instant::from_millis(2_750);
        sway.update(&mut entity);
        let direct_pan =
            IdleSway::unit_triangle(DEFAULT_PAN_PERIOD_MS, Instant::from_millis(2_750))
                * DEFAULT_PAN_AMPLITUDE_DEG;
        assert_eq!(entity.motor.head_pose.pan_deg, direct_pan);
    }
}
