//! Feetech SCServo-backed [`HeadDriver`] for the StackChan pan/tilt servos.
//!
//! Two smart servos share a half-duplex TTL UART bus (UART1 on CoreS3 at
//! 1 Mbaud, TX=GPIO6, RX=GPIO7). Each servo is addressable by a 1-byte
//! ID — [`YAW_SERVO_ID`] for pan and [`PITCH_SERVO_ID`] for tilt. The
//! physical assembly is pre-wired by M5Stack's base, so this is a
//! solderless plug-in for standard Stack-chan units.
//!
//! This module owns the servo driver + per-axis servo math + a boot-time
//! slow-ramp + the inter-task [`Signal`] used to hand poses from the
//! render task (which runs the Modifier pipeline) to the 50 Hz head
//! task.
//!
//! ## Wiring
//!
//! | Axis  | Servo ID | Mapping |
//! |-------|----------|---------|
//! | Pan   | 1 (yaw)     | `+pan_deg` → bigger step count |
//! | Tilt  | 2 (pitch)   | `+tilt_deg` → bigger step count |
//!
//! Invert a direction by flipping [`PAN_DIRECTION`] / [`TILT_DIRECTION`]
//! to `-1.0`. The sign-discovery step is the first bench-test with a
//! real unit; until then both default to `+1.0`.
//!
//! ## Calibration
//!
//! `PAN_TRIM_DEG` / `TILT_TRIM_DEG` are per-unit const trims applied at
//! the driver edge: `commanded_pos = POSITION_CENTER + direction *
//! (pose.axis + axis_trim) * POSITION_PER_DEGREE`. Rebuild after
//! physical trim.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embedded_io_async::Write;
use scservo::{POSITION_CENTER, POSITION_PER_DEGREE, Scservo};
use stackchan_core::{HeadDriver, Instant, Pose};

/// `SCServo` ID wired to the pan (yaw) servo, matching the old C++
/// firmware's convention.
pub const YAW_SERVO_ID: u8 = 1;
/// `SCServo` ID wired to the tilt (pitch) servo.
pub const PITCH_SERVO_ID: u8 = 2;

/// Pan direction sign: +1.0 if the servo turns the head right for
/// positive step counts; -1.0 if it's wired inverted.
const PAN_DIRECTION: f32 = 1.0;
/// Tilt direction sign: +1.0 if the servo nods the head up for positive
/// step counts; -1.0 otherwise.
const TILT_DIRECTION: f32 = 1.0;

/// Per-unit pan trim, in degrees. Positive = head points rightward at
/// commanded NEUTRAL. Zero until bench-measured on a real unit.
const PAN_TRIM_DEG: f32 = 0.0;
/// Per-unit tilt trim, in degrees. Positive = head aims up at NEUTRAL.
const TILT_TRIM_DEG: f32 = 0.0;

/// Boot-time slow-ramp window: how long to linearly lerp the commanded
/// pose from NEUTRAL to the first requested pose. Keeps the servos
/// from snapping on power-up.
pub const BOOT_RAMP_MS: u64 = 500;

/// Move-time sent with every `WritePos`. `SCServo` servos interpolate
/// internally over this many milliseconds, smoothing out the 50 Hz
/// step commands we send.
const MOVE_TIME_MS: u16 = 20;

/// Move-speed parameter sent with every `WritePos`. `0` means "use time
/// control" (see [`MOVE_TIME_MS`]).
const MOVE_SPEED: u16 = 0;

/// Single-producer / single-consumer latest-pose signal.
///
/// The render task calls [`Signal::signal`] with the latest
/// `avatar.head_pose` after each modifier pass; the head task drains
/// it via [`Signal::try_take`] on every tick (and holds the prior pose
/// if nothing new is pending).
pub static POSE_SIGNAL: Signal<CriticalSectionRawMutex, Pose> = Signal::new();

/// Feetech SCServo-backed head driver.
pub struct ScsHead<W: Write> {
    /// Underlying `SCServo` protocol driver on the UART bus.
    bus: Scservo<W>,
    /// Timestamp of the very first `set_pose` call. Used by the boot
    /// ramp. `None` until the first call anchors it.
    first_call_at: Option<Instant>,
}

impl<W: Write> ScsHead<W> {
    /// Wrap an [`Scservo`] bus driver. The caller is responsible for
    /// configuring the UART baud rate (1 Mbaud for SCS defaults).
    #[must_use]
    pub const fn new(bus: Scservo<W>) -> Self {
        Self {
            bus,
            first_call_at: None,
        }
    }

    /// Convert one axis angle (deg) into a servo step count, applying
    /// trim and direction. Clamps defensively to 0..=1023.
    fn position_for(angle_deg: f32, trim_deg: f32, direction: f32) -> u16 {
        let effective = (angle_deg + trim_deg) * direction;
        let offset = effective * POSITION_PER_DEGREE;
        let raw = f32::from(POSITION_CENTER) + offset;
        let clamped = raw.clamp(0.0, 1023.0);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to [0, 1023] above, safely fits u16"
        )]
        let pos = clamped as u16;
        pos
    }

    /// Produce the effective pose to command, applying the boot ramp.
    ///
    /// The first call captures `now` as the ramp start; subsequent
    /// calls linearly interpolate from `NEUTRAL` toward `target` over
    /// [`BOOT_RAMP_MS`]. After the window elapses, `target` passes
    /// through unchanged.
    fn ramped_pose(&mut self, target: Pose, now: Instant) -> Pose {
        let start = *self.first_call_at.get_or_insert(now);
        let elapsed = now.saturating_duration_since(start);
        if elapsed >= BOOT_RAMP_MS {
            return target;
        }
        #[allow(
            clippy::cast_precision_loss,
            reason = "elapsed + BOOT_RAMP_MS are < 2^32, well under the mantissa limit"
        )]
        let t = elapsed as f32 / BOOT_RAMP_MS as f32;
        Pose::new(target.pan_deg * t, target.tilt_deg * t)
    }
}

impl<W: Write> HeadDriver for ScsHead<W> {
    type Error = scservo::Error<W::Error>;

    async fn set_pose(&mut self, pose: Pose, now: Instant) -> Result<(), Self::Error> {
        let effective = self.ramped_pose(pose, now);
        let pan_pos = Self::position_for(effective.pan_deg, PAN_TRIM_DEG, PAN_DIRECTION);
        let tilt_pos = Self::position_for(effective.tilt_deg, TILT_TRIM_DEG, TILT_DIRECTION);
        self.bus
            .write_position(YAW_SERVO_ID, pan_pos, MOVE_TIME_MS, MOVE_SPEED)
            .await?;
        self.bus
            .write_position(PITCH_SERVO_ID, tilt_pos, MOVE_TIME_MS, MOVE_SPEED)
            .await?;
        Ok(())
    }
}
