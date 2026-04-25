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
/// Modifiers that request negative tilt (e.g. `EmotionHead::Sad`/
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
    reason = "tests compare bit-exact outputs of our own clamp/const code, \
              not results of floating-point arithmetic where epsilon matters"
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
}
