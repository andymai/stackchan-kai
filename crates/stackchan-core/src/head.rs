//! Head kinematics: pan/tilt pose + [`HeadDriver`] trait.
//!
//! The StackChan's head rotates on two servos: pan (left/right rotation) and
//! tilt (up/down nod). Core models this as a [`Pose`] carried on the
//! [`Entity`](crate::entity::Entity), so the same [`Modifier`](crate::Modifier)
//! pipeline that animates eyes, mouth, and emotion can also produce motion
//! trajectories. Firmware consumes the pose by calling [`HeadDriver::set_pose`]
//! on an async I²C driver (see `crates/pca9685`); the simulator uses a
//! recording driver that captures the trajectory for golden-test assertions.
//!
//! ## Conventions
//!
//! - **Units:** degrees. `f32`, because the ESP32-S3 has a single-precision
//!   FPU and angular smoothing/interpolation reads naturally as floats.
//! - **Sign:** positive pan = head turns right from the *viewer's* POV
//!   (the servo horn rotates clockwise looking down on the head). Positive
//!   tilt = head nods up (chin rises).
//! - **Range:** conservative pan range `±MAX_PAN_DEG` is symmetric, but
//!   tilt is asymmetric `[MIN_TILT_DEG, MAX_TILT_DEG]` because the
//!   Stack-chan chassis cutout permits upward but not downward head
//!   travel from horizontal. Firmware const-table trim is applied
//!   *after* Pose is produced, so the core-visible range is uniform.

// `F32Ext` provides `atan2`/`sqrt`/`to_degrees` on the `no_std`
// firmware target. On host (`cfg(test)`) these methods come from
// `std`, so the import looks unused — silence the lint rather than
// branch the import on `cfg`.
#[allow(unused_imports)]
use micromath::F32Ext as _;

use crate::clock::Instant;

/// Conservative upper bound on pan travel in degrees (±).
///
/// Well inside SG90 mechanical limits (~±80°) with margin for servo-horn
/// misalignment. Widen deliberately after per-unit calibration; do not
/// raise as a matter of course — the BOM of a StackChan base includes
/// hard plastic stops that will grind gear teeth if overshot.
pub const MAX_PAN_DEG: f32 = 45.0;

/// Lower bound on tilt travel in degrees.
///
/// Asymmetric with [`MAX_TILT_DEG`] because Stack-chan's chassis
/// cutout typically blocks downward head travel — the head's
/// mechanical rest position already sits at the lower stop.
/// Modifiers that request negative tilt (e.g. `HeadFromEmotion::Sad`/
/// `Sleepy`'s downcast bias) are silently clamped to `MIN_TILT_DEG`
/// by [`Pose::clamped`]; the emotion's other channels (eyes, mouth,
/// LEDs) still differentiate.
pub const MIN_TILT_DEG: f32 = 0.0;

/// Upper bound on tilt travel in degrees.
///
/// Tilt has tighter mechanical limits than pan on most StackChan bases
/// (the pan servo sits under the tilt linkage). Matches the 1000–2000 µs
/// pulse-width envelope the firmware exposes by default.
pub const MAX_TILT_DEG: f32 = 30.0;

/// Servo pan/tilt pose in degrees.
///
/// [`Pose::NEUTRAL`] is the rest position (head facing forward, level).
/// See module docs for sign conventions and the safe-range constants
/// [`MAX_PAN_DEG`] / [`MAX_TILT_DEG`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pose {
    /// Pan angle in degrees. Positive = turn right (viewer POV).
    pub pan_deg: f32,
    /// Tilt angle in degrees. Positive = nod up (chin rises).
    pub tilt_deg: f32,
}

impl Pose {
    /// The rest pose: head facing forward, level. Firmware boots into this
    /// via a slow ramp so power-up doesn't snap the servos.
    pub const NEUTRAL: Self = Self {
        pan_deg: 0.0,
        tilt_deg: 0.0,
    };

    /// Construct a [`Pose`] from explicit pan/tilt values. Does not clamp;
    /// callers that need the safe range should use [`Pose::clamped`].
    #[must_use]
    pub const fn new(pan_deg: f32, tilt_deg: f32) -> Self {
        Self { pan_deg, tilt_deg }
    }

    /// Return this pose clamped to `±MAX_PAN_DEG` for pan and
    /// `[MIN_TILT_DEG, MAX_TILT_DEG]` for tilt (asymmetric — see the
    /// per-axis const docs).
    ///
    /// NaN inputs collapse to `NEUTRAL` for that axis — servos cannot
    /// honour a non-number command, and silently passing NaN into a
    /// pulse-width computation upstream is a latent bug. Using the
    /// neutral fallback instead of panicking keeps the modifier pipeline
    /// robust under arithmetic mishaps.
    #[must_use]
    pub const fn clamped(self) -> Self {
        Self {
            pan_deg: clamp_symmetric_or_zero(self.pan_deg, MAX_PAN_DEG),
            tilt_deg: clamp_range_or_zero(self.tilt_deg, MIN_TILT_DEG, MAX_TILT_DEG),
        }
    }

    /// 3D inverse kinematics: convert a Cartesian world point into the
    /// pose that aims the head at it. Right-handed coordinates with
    /// `+Z` forward (head's natural gaze axis), `+X` right, `+Y` up.
    ///
    /// - Pan (yaw) = `atan2(x, z)` — rotation around the vertical axis.
    /// - Tilt (pitch) = `atan2(y, sqrt(x² + z²))` — elevation above
    ///   the horizontal plane.
    ///
    /// Returns `None` when the target lies at the origin (or so close
    /// that the math is undefined): there's no direction to aim at.
    /// The factory firmware's `motion.h::lookAtPoint` documents this
    /// same singularity.
    ///
    /// Inputs must be finite — NaN or Inf in any axis returns `None`.
    /// The returned pose is *unclamped*; callers that need the
    /// mechanical-safe range should pipe the result through
    /// [`Self::clamped`].
    #[must_use]
    pub fn from_xyz_lookat(x: f32, y: f32, z: f32) -> Option<Self> {
        // Tolerance keeps us from amplifying float noise into wild
        // angles when the point is "essentially the origin." 1 mm-ish
        // in any sensible application unit.
        const ORIGIN_EPSILON_SQ: f32 = 1e-6;

        if !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return None;
        }
        // Squared planar distance avoids a sqrt for the singularity
        // check. `mul_add` would need libm on no_std for f32; the
        // straightforward form keeps the math platform-uniform.
        #[allow(
            clippy::suboptimal_flops,
            reason = "f32::mul_add needs libm on no_std; straight multiply-add is platform-uniform here"
        )]
        let planar_sq = x * x + z * z;
        #[allow(clippy::suboptimal_flops, reason = "see above")]
        let radial_sq = planar_sq + y * y;
        if radial_sq < ORIGIN_EPSILON_SQ {
            return None;
        }
        // `atan2` + `sqrt` come from `micromath`'s `F32Ext` polyfill;
        // matches the rest of the workspace's no_std math stance.
        let pan_rad = x.atan2(z);
        // sqrt(planar_sq) is fine even at x=z=0 because radial_sq >=
        // ORIGIN_EPSILON_SQ guarantees y is large enough that the
        // tilt branch dominates.
        let planar = planar_sq.sqrt();
        let tilt_rad = y.atan2(planar);
        Some(Self {
            pan_deg: pan_rad.to_degrees(),
            tilt_deg: tilt_rad.to_degrees(),
        })
    }
}

/// Clamp `value` into `[-max, +max]`, collapsing NaN to `0.0`.
const fn clamp_symmetric_or_zero(value: f32, max: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(-max, max)
    }
}

/// Clamp `value` into `[min, max]`, collapsing NaN to `0.0`.
const fn clamp_range_or_zero(value: f32, min: f32, max: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(min, max)
    }
}

/// Sink for head pose commands.
///
/// Implementations realize a [`Pose`] on hardware (PCA9685 → SG90 on the
/// firmware side) or record it for test assertions (sim side). The trait
/// is async to match the I²C transport: PCA9685 writes are awaited over
/// `embedded-hal-async`.
///
/// Errors are surfaced as the associated `Error` type so callers can
/// choose their policy — the firmware's 50 Hz head task logs warnings
/// and keeps going; a stricter embedded host could halt instead.
pub trait HeadDriver {
    /// Transport or driver error.
    type Error;

    /// Command the head to `pose` as of `now`. Implementations may clamp,
    /// smooth, or ignore updates (e.g. during a boot ramp); callers must
    /// not assume the servos have actually reached `pose` on return.
    fn set_pose(
        &mut self,
        pose: Pose,
        now: Instant,
    ) -> impl core::future::Future<Output = Result<(), Self::Error>>;
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests compare bit-exact outputs of our own clamp/const code, \
              and unwrap on values our own helper just produced"
)]
mod tests {
    use super::*;

    #[test]
    fn neutral_is_zero() {
        let n = Pose::NEUTRAL;
        assert_eq!(n.pan_deg, 0.0);
        assert_eq!(n.tilt_deg, 0.0);
    }

    #[test]
    fn clamped_respects_safe_range() {
        let p = Pose::new(100.0, -100.0).clamped();
        assert_eq!(p.pan_deg, MAX_PAN_DEG);
        // Tilt is asymmetric — negative inputs clamp up to MIN_TILT_DEG.
        assert_eq!(p.tilt_deg, MIN_TILT_DEG);
        let q = Pose::new(-100.0, 100.0).clamped();
        assert_eq!(q.pan_deg, -MAX_PAN_DEG);
        assert_eq!(q.tilt_deg, MAX_TILT_DEG);
    }

    #[test]
    fn clamped_preserves_in_range_values() {
        // Tilt of -5 is below MIN_TILT_DEG (0), so it'll clamp to 0.
        let p = Pose::new(10.0, 5.0).clamped();
        assert_eq!(p.pan_deg, 10.0);
        assert_eq!(p.tilt_deg, 5.0);
    }

    #[test]
    fn clamped_tilt_lower_bound_is_zero() {
        let p = Pose::new(0.0, -1.0).clamped();
        assert_eq!(p.tilt_deg, MIN_TILT_DEG);
    }

    #[test]
    fn nan_collapses_to_neutral() {
        let p = Pose::new(f32::NAN, f32::NAN).clamped();
        assert_eq!(p.pan_deg, 0.0);
        assert_eq!(p.tilt_deg, 0.0);
    }

    #[test]
    fn lookat_forward_is_neutral() {
        // Straight ahead on +Z, level → pan = 0, tilt = 0.
        let p = Pose::from_xyz_lookat(0.0, 0.0, 1.0).expect("forward target is well-defined");
        assert!(p.pan_deg.abs() < 0.5);
        assert!(p.tilt_deg.abs() < 0.5);
    }

    #[test]
    fn lookat_right_yields_positive_pan() {
        // Pure +X (one unit to the right, no Y, no Z) → pan = +90°.
        let p = Pose::from_xyz_lookat(1.0, 0.0, 0.0).expect("right target is well-defined");
        assert!(
            (p.pan_deg - 90.0).abs() < 0.5,
            "expected pan ≈ +90, got {}",
            p.pan_deg
        );
        assert!(p.tilt_deg.abs() < 0.5);
    }

    #[test]
    fn lookat_left_yields_negative_pan() {
        let p = Pose::from_xyz_lookat(-1.0, 0.0, 0.0).expect("left target is well-defined");
        assert!(
            (p.pan_deg - (-90.0)).abs() < 0.5,
            "expected pan ≈ -90, got {}",
            p.pan_deg
        );
    }

    #[test]
    fn lookat_above_yields_positive_tilt() {
        // Up (+Y), forward (+Z) → tilt ≈ +45°.
        let p = Pose::from_xyz_lookat(0.0, 1.0, 1.0).expect("up-forward target is well-defined");
        assert!(p.pan_deg.abs() < 0.5);
        assert!(
            (p.tilt_deg - 45.0).abs() < 1.0,
            "expected tilt ≈ +45, got {}",
            p.tilt_deg
        );
    }

    #[test]
    fn lookat_below_yields_negative_tilt() {
        let p = Pose::from_xyz_lookat(0.0, -1.0, 1.0).expect("down-forward target is well-defined");
        assert!(
            (p.tilt_deg - (-45.0)).abs() < 1.0,
            "expected tilt ≈ -45, got {}",
            p.tilt_deg
        );
    }

    #[test]
    fn lookat_origin_is_singularity() {
        assert!(Pose::from_xyz_lookat(0.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn lookat_rejects_nan_and_infinity() {
        assert!(Pose::from_xyz_lookat(f32::NAN, 0.0, 1.0).is_none());
        assert!(Pose::from_xyz_lookat(0.0, f32::INFINITY, 1.0).is_none());
        assert!(Pose::from_xyz_lookat(0.0, 0.0, f32::NEG_INFINITY).is_none());
    }

    #[test]
    fn lookat_independent_of_target_distance() {
        // Doubling all coordinates keeps the same direction → same pose.
        let near = Pose::from_xyz_lookat(0.5, 0.3, 1.0).expect("near target well-defined");
        let far = Pose::from_xyz_lookat(50.0, 30.0, 100.0).expect("far target well-defined");
        assert!(
            (near.pan_deg - far.pan_deg).abs() < 0.5,
            "pan should be distance-invariant: {} vs {}",
            near.pan_deg,
            far.pan_deg
        );
        assert!(
            (near.tilt_deg - far.tilt_deg).abs() < 0.5,
            "tilt should be distance-invariant: {} vs {}",
            near.tilt_deg,
            far.tilt_deg
        );
    }
}
