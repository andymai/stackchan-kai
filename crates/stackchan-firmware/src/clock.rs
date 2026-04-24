//! Embassy-backed [`Clock`] implementation.
//!
//! `stackchan_core::Clock` is intentionally abstract so the same modifiers
//! run on real hardware and under the sim crate's `FakeClock`. On the
//! CoreS3 we read `embassy_time::Instant::now()`, which is driven by the
//! timer esp-rtos starts during boot.

use stackchan_core::{Clock, Instant};

/// Zero-sized [`Clock`] that reads the embassy-time monotonic timer.
///
/// Must only be instantiated after `esp_rtos::start(...)` has been called;
/// before that the underlying timer driver is inactive and `now()` would
/// return the same value every call.
#[derive(Clone, Copy, Default)]
pub struct HalClock;

impl Clock for HalClock {
    fn now(&self) -> Instant {
        // embassy-time exposes millisecond precision; both sides are u64 ms.
        Instant::from_millis(embassy_time::Instant::now().as_millis())
    }
}
