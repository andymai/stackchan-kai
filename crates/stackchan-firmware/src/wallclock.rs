//! BM8563 wall-clock read, used to timestamp a single boot log line.
//!
//! The `defmt` timestamp stays as `embassy_time::Instant::now()` in
//! milliseconds — that's what the embassy / defmt integration expects.
//! This module exists only to give the boot log an absolute time
//! ("boot @ 2026-04-24 13:37:05") so post-reboot logs can be
//! correlated without maintaining a side channel on the host.
//!
//! No attempt to keep a running wall-clock: the one caller today
//! pays for one I²C round-trip at boot. YAGNI until we grow a second
//! caller.

use bm8563::{Bm8563, format_datetime};
use embedded_hal_async::i2c::I2c as AsyncI2c;

/// Read + format the current wall time from the RTC.
///
/// Writes into `buffer` in `YYYY-MM-DD HH:MM:SS` form (19 ASCII bytes)
/// and returns the filled string slice, or `None` if the RTC was
/// unreachable / unreliable.
///
/// Using a caller-provided buffer keeps the allocator out of the hot
/// path.
pub async fn read_and_format<I: AsyncI2c>(bus: I, buffer: &mut [u8; 19]) -> Option<&str> {
    let mut rtc = Bm8563::new(bus);
    if let Err(e) = rtc.init().await {
        defmt::warn!(
            "BM8563: init failed ({}); boot log will omit wall-clock",
            defmt::Debug2Format(&e),
        );
        return None;
    }
    match rtc.read_datetime().await {
        Ok(dt) => Some(format_datetime(dt, buffer)),
        Err(e) => {
            defmt::warn!(
                "BM8563: read failed ({}); boot log will omit wall-clock",
                defmt::Debug2Format(&e),
            );
            None
        }
    }
}
