//! Boot-time servo-power-rail status, mirrored for HTTP read-out.
//!
//! [`board::bringup`](crate::board) drives the PY32's servo-power gate
//! HIGH at boot. That step is non-fatal: a failure leaves the rail off
//! and the head unpowered, but the firmware still boots a working face.
//! The only on-device evidence of the failure is a one-shot defmt
//! `warn` — gone after the next reset, and invisible to an operator
//! debugging over the LAN.
//!
//! This module records the outcome of the rail-enable attempt into a
//! single mirror static so `GET /hardware/status` can report whether
//! the servos are powered without re-touching the I²C bus. Mirrors the
//! `watchdog::TASKS_SNAPSHOT` read-side pattern.

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

/// Outcome of the boot-time servo-power-rail enable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServoPowerStatus {
    /// Whether the rail-enable write to the PY32 ultimately succeeded.
    /// `false` means the head is unpowered — `ping_servo` will time out
    /// and commanded poses produce no motion.
    pub enabled: bool,
    /// Number of enable attempts made (1 on first-try success, up to
    /// the boot retry cap on failure).
    pub attempts: u8,
    /// Whether the post-enable settle delay completed. Distinguishes a
    /// rail that came up cleanly from one whose enable returned `Ok`
    /// but never finished settling (only `false` before boot reaches
    /// the settle step).
    pub settled: bool,
}

impl ServoPowerStatus {
    /// Pre-boot placeholder: rail not yet enabled. Distinguishable from
    /// a real failure by `attempts == 0`.
    const fn empty() -> Self {
        Self {
            enabled: false,
            attempts: 0,
            settled: false,
        }
    }
}

/// Read-side mirror of the boot-time enable outcome. Written once by
/// [`record`] during `board::bringup`; read by the HTTP
/// `GET /hardware/status` handler.
static SERVO_POWER_STATUS: Mutex<CriticalSectionRawMutex, core::cell::Cell<ServoPowerStatus>> =
    Mutex::new(core::cell::Cell::new(ServoPowerStatus::empty()));

/// Cheap snapshot read for the HTTP `GET /hardware/status` handler.
#[must_use]
pub fn read_status() -> ServoPowerStatus {
    SERVO_POWER_STATUS.lock(core::cell::Cell::get)
}

/// Record the outcome of the boot-time servo-power-rail enable.
pub(crate) fn record(status: ServoPowerStatus) {
    SERVO_POWER_STATUS.lock(|cell| cell.set(status));
}
