//! PCA9685-backed [`HeadDriver`] for the StackChan pan/tilt servos.
//!
//! The PCA9685 sits on the CoreS3's external I²C Port A (separate bus from
//! internal AXP2101/AW9523). This module owns that driver + per-channel
//! servo math + a boot-time slow-ramp + the inter-task [`Signal`] used to
//! hand poses from the render task (which runs the Modifier pipeline) to
//! the 50 Hz head task (which only consumes).
//!
//! ## Wiring
//!
//! | Axis  | PCA9685 channel |
//! |-------|-----------------|
//! | Pan   | 0               |
//! | Tilt  | 1               |
//!
//! ## Calibration
//!
//! `PAN_TRIM_DEG` / `TILT_TRIM_DEG` are per-unit const trims applied at the
//! driver edge: `commanded_pulse = CENTER_US + (pose.axis + axis_trim) * US_PER_DEG`.
//! Rebuild after physical trim; future work might move this into a RON
//! config so non-coders can tweak.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embedded_hal_async::i2c::I2c;
use pca9685::Pca9685;
use stackchan_core::{HeadDriver, Instant, Pose};

/// PCA9685 channel wired to the pan servo.
pub const PAN_CHANNEL: u8 = 0;
/// PCA9685 channel wired to the tilt servo.
pub const TILT_CHANNEL: u8 = 1;

/// Pulse width at mechanical center, in microseconds. Standard SG90 spec.
const CENTER_US: u16 = 1500;
/// Microseconds of pulse width per degree of commanded angle.
///
/// SG90 spec: 1000 µs → −45°, 2000 µs → +45°, linear in between.
/// `1000 µs / 90° ≈ 11.11 µs/°`.
const US_PER_DEG: f32 = 1000.0 / 90.0;
/// Lower clamp on commanded pulse width — below this, typical SG90 gears
/// hit mechanical stops and stall.
const MIN_PULSE_US: u16 = 500;
/// Upper clamp on commanded pulse width (symmetric to [`MIN_PULSE_US`]).
const MAX_PULSE_US: u16 = 2500;

/// Per-unit pan trim, in degrees. Positive = head points rightward at
/// commanded NEUTRAL. Zero until we've bench-measured a real unit.
const PAN_TRIM_DEG: f32 = 0.0;
/// Per-unit tilt trim, in degrees. Positive = head aims up at NEUTRAL.
const TILT_TRIM_DEG: f32 = 0.0;

/// Boot-time slow-ramp window: how long to linearly lerp the commanded
/// pose from NEUTRAL to the first requested pose. Keeps the servos from
/// snapping on power-up (audible thunk + mechanical stress).
pub const BOOT_RAMP_MS: u64 = 500;

/// Single-producer / single-consumer latest-pose signal.
///
/// The render task calls [`Signal::signal`] with the latest
/// `avatar.head_pose` after each modifier pass; the head task drains it
/// via [`Signal::try_take`] on every tick (and holds the prior pose if
/// nothing new is pending — servos hold position fine, so a few stale
/// ticks cost nothing).
pub static POSE_SIGNAL: Signal<CriticalSectionRawMutex, Pose> = Signal::new();

/// PCA9685-backed head driver. Owns the I²C bus dedicated to the servo
/// chip (separate from the internal AXP2101/AW9523 bus).
pub struct PcaHead<B: I2c> {
    /// The underlying PCA9685 driver.
    pwm: Pca9685<B>,
    /// Timestamp of the very first `set_pose` call. Used by the boot ramp.
    /// `None` until the first call commits the start-of-ramp anchor.
    first_call_at: Option<Instant>,
}

impl<B: I2c> PcaHead<B> {
    /// Wrap a freshly-initialized [`Pca9685`]. The chip must already be
    /// running — callers run [`Pca9685::init`] with the 50 Hz servo
    /// prescale before constructing this.
    #[must_use]
    pub const fn new(pwm: Pca9685<B>) -> Self {
        Self {
            pwm,
            first_call_at: None,
        }
    }

    /// Convert one axis angle (deg) into a pulse width (µs), applying the
    /// axis trim and clamping to the mechanical pulse-width window.
    fn pulse_for(angle_deg: f32, trim_deg: f32) -> u16 {
        let effective = angle_deg + trim_deg;
        let pulse_offset = effective * US_PER_DEG;
        let raw = f32::from(CENTER_US) + pulse_offset;
        let clamped = raw.clamp(f32::from(MIN_PULSE_US), f32::from(MAX_PULSE_US));
        // clamped is a finite positive f32 in [MIN_PULSE_US, MAX_PULSE_US],
        // well under u16::MAX, so the cast is lossless.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to [MIN_PULSE_US, MAX_PULSE_US] above"
        )]
        let us = clamped as u16;
        us
    }

    /// Produce the effective pose to command, applying the boot ramp.
    ///
    /// The first call captures `now` as the ramp start; subsequent calls
    /// linearly interpolate from `NEUTRAL` toward `target` over
    /// [`BOOT_RAMP_MS`]. After the window elapses, `target` passes through
    /// unchanged.
    fn ramped_pose(&mut self, target: Pose, now: Instant) -> Pose {
        let start = *self.first_call_at.get_or_insert(now);
        let elapsed = now.saturating_duration_since(start);
        if elapsed >= BOOT_RAMP_MS {
            return target;
        }
        // Lerp in f32 for readability; the ratio is well inside f32 range.
        #[allow(
            clippy::cast_precision_loss,
            reason = "elapsed + BOOT_RAMP_MS are both < 2^32, well under the mantissa limit"
        )]
        let t = elapsed as f32 / BOOT_RAMP_MS as f32;
        Pose::new(target.pan_deg * t, target.tilt_deg * t)
    }
}

impl<B: I2c> HeadDriver for PcaHead<B> {
    type Error = pca9685::Error<B::Error>;

    async fn set_pose(&mut self, pose: Pose, now: Instant) -> Result<(), Self::Error> {
        let effective = self.ramped_pose(pose, now);
        let pan_us = Self::pulse_for(effective.pan_deg, PAN_TRIM_DEG);
        let tilt_us = Self::pulse_for(effective.tilt_deg, TILT_TRIM_DEG);
        self.pwm.set_channel_pulse_us(PAN_CHANNEL, pan_us).await?;
        self.pwm.set_channel_pulse_us(TILT_CHANNEL, tilt_us).await?;
        Ok(())
    }
}
