//! BM8563 wall-clock read, used to timestamp a single boot log line
//! and select the time-of-day boot greeting.
//!
//! The `defmt` timestamp stays as `embassy_time::Instant::now()` in
//! milliseconds — that's what the embassy / defmt integration expects.
//! This module exists to give the boot log an absolute time ("boot @
//! 2026-04-24 13:37:05") so post-reboot logs can be correlated, and
//! to surface the current hour so the audio task can pick a
//! morning / day / evening / night greeting (see
//! [`crate::audio::boot_greeting_for_hour`]).
//!
//! No attempt to keep a running wall-clock: callers today pay for
//! one I²C round-trip at boot. YAGNI until we grow a regular polling
//! consumer.

use bm8563::{Bm8563, DateTime};
use embedded_hal_async::i2c::I2c as AsyncI2c;

/// Read the current wall time from the RTC.
///
/// Returns `None` if the RTC was unreachable / unreliable. Errors are
/// logged at `warn` so boot diagnostics surface without coupling
/// callers to the BM8563 error type.
pub async fn read_datetime<I: AsyncI2c>(bus: I) -> Option<DateTime> {
    let mut rtc = Bm8563::new(bus);
    if let Err(e) = rtc.init().await {
        defmt::warn!(
            "BM8563: init failed ({}); boot log will omit wall-clock",
            defmt::Debug2Format(&e),
        );
        return None;
    }
    rtc.read_datetime()
        .await
        .inspect_err(|e| {
            defmt::warn!(
                "BM8563: read failed ({}); boot log will omit wall-clock",
                defmt::Debug2Format(e),
            );
        })
        .ok()
}
