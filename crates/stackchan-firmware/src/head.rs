//! Feetech SCServo-backed [`HeadDriver`] for the StackChan pan/tilt servos.
//!
//! Two smart servos share a half-duplex TTL UART bus (UART1 on CoreS3 at
//! 1 Mbaud, TX=GPIO6, RX=GPIO7). Each servo is addressable by a 1-byte
//! ID — [`YAW_SERVO_ID`] for pan and [`PITCH_SERVO_ID`] for tilt. The
//! physical assembly is pre-wired by M5Stack's base, so this is a
//! solderless plug-in for standard Stack-chan units.
//!
//! This module owns the servo driver + per-axis servo math + the
//! inter-task [`Signal`] used to hand poses from the render task (which
//! runs the Modifier pipeline) to the 50 Hz head task. Smooth start-up
//! is handled by the firmware's boot-nod gesture in `main` rather than
//! an implicit ramp here — the gesture is the gentle-first-move.
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
use embassy_time::{Duration, with_timeout};
use embedded_io_async::{Read, Write};
use scservo::{POSITION_CENTER, POSITION_PER_DEGREE, Scservo};
use stackchan_core::{HeadDriver, Instant, Pose};

/// Time-bounded drain of the 6-byte Feetech status response.
/// Servos with Status Return Level = 2 send one after every write;
/// servos with Level = 0 stay silent, so an unbounded drain would
/// hang. 2 ms is comfortably longer than the ~60 µs transmission of
/// 6 bytes at 1 Mbaud plus the servo's own response latency.
const STATUS_DRAIN_TIMEOUT_MS: u64 = 2;

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

/// Move-time sent with every `WritePos`. `SCServo` servos interpolate
/// internally over this many milliseconds, smoothing out the 50 Hz
/// step commands we send.
const MOVE_TIME_MS: u16 = 20;

/// Move-speed parameter sent with every `WritePos`. `0` means "use time
/// control" (see [`MOVE_TIME_MS`]).
const MOVE_SPEED: u16 = 0;

/// Commanded-pose signal: render task → head task.
///
/// The render task calls [`Signal::signal`] with the latest
/// `avatar.head_pose` after each modifier pass; the head task drains
/// it via [`Signal::try_take`] on every tick (and holds the prior pose
/// if nothing new is pending).
pub static POSE_SIGNAL: Signal<CriticalSectionRawMutex, Pose> = Signal::new();

/// Observed-pose signal: head task → render task.
///
/// The head task reads the servos' live position at ~1 Hz and signals
/// the result back here; the render task consumes via `try_take` and
/// writes to `avatar.head_pose_actual`. Used for feedback logging +
/// future gaze-compensation work. Signal semantics: latest wins, no
/// backlog.
pub static HEAD_POSE_ACTUAL_SIGNAL: Signal<CriticalSectionRawMutex, Pose> = Signal::new();

/// Feetech SCServo-backed head driver.
pub struct ScsHead<W> {
    /// Underlying `SCServo` protocol driver on the UART bus.
    bus: Scservo<W>,
}

impl<W: Write> ScsHead<W> {
    /// Wrap an [`Scservo`] bus driver. The caller is responsible for
    /// configuring the UART baud rate (1 Mbaud for SCS defaults).
    #[must_use]
    pub const fn new(bus: Scservo<W>) -> Self {
        Self { bus }
    }

    /// Borrow the wrapped bus mutably. Needed by firmware `main` to
    /// issue `ping` + boot-nod commands before handing the driver to
    /// the head task.
    pub const fn bus_mut(&mut self) -> &mut Scservo<W> {
        &mut self.bus
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

    /// Inverse of [`Self::position_for`]: convert a live servo step
    /// count back into degrees in the same reference frame the caller
    /// commanded. Used by the 1 Hz position poll.
    fn deg_for(position: u16, trim_deg: f32, direction: f32) -> f32 {
        // `direction` is a ±1.0 const today; belt-and-suspenders check
        // against a future refactor that makes it runtime-configurable,
        // because a 0.0 would silently return ±inf here rather than an
        // out-of-range Pose that later clamps cleanly.
        debug_assert!(
            direction.is_finite() && direction != 0.0,
            "direction must be a non-zero finite sign; a 0.0 would make deg_for return ±inf"
        );
        let offset = f32::from(position) - f32::from(POSITION_CENTER);
        let effective = offset / POSITION_PER_DEGREE;
        // Inverse direction + trim from `position_for`.
        (effective / direction) - trim_deg
    }
}

impl<U: Read + Write> ScsHead<U> {
    /// Read the live position of both servos, return a `Pose` in the
    /// same commanded-reference-frame as `set_pose` inputs. Useful for
    /// feedback logging + the future `EyeGaze` modifier.
    ///
    /// # Errors
    /// Returns the transport error on the first failing read; the
    /// caller typically logs + continues rather than treating this as
    /// fatal.
    pub async fn read_pose(&mut self) -> Result<Pose, scservo::Error<U::Error>> {
        let pan_pos = self.bus.read_position(YAW_SERVO_ID).await?;
        let tilt_pos = self.bus.read_position(PITCH_SERVO_ID).await?;
        let pan_deg = Self::deg_for(pan_pos, PAN_TRIM_DEG, PAN_DIRECTION);
        let tilt_deg = Self::deg_for(tilt_pos, TILT_TRIM_DEG, TILT_DIRECTION);
        Ok(Pose::new(pan_deg, tilt_deg))
    }
}

impl<U: Read + Write> HeadDriver for ScsHead<U> {
    type Error = scservo::Error<U::Error>;

    async fn set_pose(&mut self, pose: Pose, _now: Instant) -> Result<(), Self::Error> {
        let pan_pos = Self::position_for(pose.pan_deg, PAN_TRIM_DEG, PAN_DIRECTION);
        let tilt_pos = Self::position_for(pose.tilt_deg, TILT_TRIM_DEG, TILT_DIRECTION);
        self.bus
            .write_position(YAW_SERVO_ID, pan_pos, MOVE_TIME_MS, MOVE_SPEED)
            .await?;
        // Drain the 6-byte status response before the next write so
        // it doesn't pile up in the UART RX FIFO and corrupt the
        // periodic `read_pose` readback. Silent servos (Status Return
        // Level = 0) make the drain hang, so it's time-bounded.
        let _ = with_timeout(
            Duration::from_millis(STATUS_DRAIN_TIMEOUT_MS),
            self.bus.drain_write_status(),
        )
        .await;
        self.bus
            .write_position(PITCH_SERVO_ID, tilt_pos, MOVE_TIME_MS, MOVE_SPEED)
            .await?;
        let _ = with_timeout(
            Duration::from_millis(STATUS_DRAIN_TIMEOUT_MS),
            self.bus.drain_write_status(),
        )
        .await;
        Ok(())
    }
}
