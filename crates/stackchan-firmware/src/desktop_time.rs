//! BM8563 RTC writer for Claude Desktop time-sync messages.
//!
//! The Hardware Buddy protocol's `{"time":[epoch_secs,
//! tz_offset_secs]}` message is the desktop telling the device "set
//! your wall clock to this." The reference protocol guarantees one
//! such message per connect, and the desktop's time authority is
//! independent of SNTP — operators may choose to point the device
//! at Claude Desktop as the only time source when the LAN doesn't
//! have a routable SNTP server.
//!
//! [`desktop_time_task`] owns its own [`Bm8563`] handle off the
//! shared I²C bus and waits on [`DESKTOP_RTC_WRITE_REQUEST`];
//! producers (currently just `desktop_control`) call
//! [`Signal::signal`] with the epoch seconds and the writer pushes
//! the resulting [`DateTime`] into the chip. Two-writer coexistence
//! with the SNTP task is safe — the shared [`SharedI2cBus`] mutex
//! serialises bus access and both tasks treat the RTC as
//! overwrite-on-write with no read-back locking.
//!
//! The timezone offset that rides along with the epoch is currently
//! logged only. The BM8563 stores UTC; tz-offset is a render-time
//! detail handled by [`crate::wallclock::apply_offset`] downstream.

use bm8563::{Bm8563, DateTime};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

use crate::board::SharedI2cBus;

/// Request the RTC be set to the supplied Unix epoch seconds.
///
/// Single-writer (`desktop_control`), single-reader
/// ([`desktop_time_task`]) — latest-wins semantics are acceptable
/// because the desktop sends at most one time-sync per connect and
/// retransmits would just re-write the same epoch.
pub static DESKTOP_RTC_WRITE_REQUEST: Signal<CriticalSectionRawMutex, i64> = Signal::new();

/// Embassy task: drain [`DESKTOP_RTC_WRITE_REQUEST`] forever,
/// writing each received epoch into the BM8563. `i2c_bus` is the
/// same shared handle the SNTP / chime / sensor tasks use.
#[embassy_executor::task]
pub async fn desktop_time_task(i2c_bus: &'static SharedI2cBus) -> ! {
    let mut bus = I2cDevice::new(i2c_bus);
    let mut rtc = Bm8563::new(&mut bus);
    if let Err(e) = rtc.init().await {
        defmt::warn!(
            "desktop_time: BM8563 init failed ({}); RTC writes will retry per request",
            defmt::Debug2Format(&e),
        );
    }
    loop {
        let epoch = DESKTOP_RTC_WRITE_REQUEST.wait().await;
        let Ok(unix_secs) = u32::try_from(epoch) else {
            defmt::warn!(
                "desktop_time: epoch {=i64} out of u32 range; skipping RTC write",
                epoch
            );
            continue;
        };
        let dt = unix_to_datetime(unix_secs);
        match rtc.write_datetime(dt).await {
            Ok(()) => defmt::info!(
                "desktop_time: RTC set from desktop ({=u16}-{=u8:02}-{=u8:02} {=u8:02}:{=u8:02}:{=u8:02} UTC)",
                dt.year,
                dt.month,
                dt.day,
                dt.hours,
                dt.minutes,
                dt.seconds,
            ),
            Err(e) => defmt::warn!(
                "desktop_time: BM8563 write failed ({})",
                defmt::Debug2Format(&e)
            ),
        }
    }
}

/// Convert a Unix timestamp into a Gregorian UTC `DateTime` for the
/// BM8563 driver. Valid for `1970-01-01` through `2099-12-31` (the
/// BM8563 itself is constrained to the same window via its
/// `CENTURY` flag). Duplicated locally rather than shared with the
/// SNTP path's private copy — the two paths might diverge later
/// (SNTP might gain millisecond precision, desktop time-sync might
/// gain timezone-aware offsets) and a shared helper would invite
/// premature coupling.
fn unix_to_datetime(unix_secs: u32) -> DateTime {
    const SECS_PER_DAY: u32 = 86_400;
    let days = unix_secs / SECS_PER_DAY;
    let secs = unix_secs % SECS_PER_DAY;
    #[allow(clippy::cast_possible_truncation)]
    let hours = (secs / 3_600) as u8;
    #[allow(clippy::cast_possible_truncation)]
    let minutes = ((secs % 3_600) / 60) as u8;
    #[allow(clippy::cast_possible_truncation)]
    let seconds = (secs % 60) as u8;
    // 1970-01-01 was a Thursday.
    #[allow(clippy::cast_possible_truncation)]
    let weekday = ((days + 4) % 7) as u8;

    let (year, month, day) = days_to_ymd(days);
    DateTime {
        year,
        month,
        day,
        weekday,
        hours,
        minutes,
        seconds,
    }
}

/// Walk forward from 1970-01-01 by whole years then months until the
/// day-of-year remainder fits inside a single month.
fn days_to_ymd(mut days: u32) -> (u16, u8, u8) {
    let mut year: u16 = 1970;
    loop {
        let in_year = if is_leap(year) { 366 } else { 365 };
        if days < in_year {
            break;
        }
        days -= in_year;
        year += 1;
    }
    let mut month: u8 = 1;
    loop {
        let dim = u32::from(days_in_month(year, month));
        if days < dim {
            break;
        }
        days -= dim;
        month += 1;
    }
    #[allow(clippy::cast_possible_truncation)]
    let day = (days + 1) as u8;
    (year, month, day)
}

/// Standard Gregorian leap-year rule.
const fn is_leap(year: u16) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Days in the given Gregorian month. February honours `is_leap`.
const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests assert structural invariants; .expect / .unwrap are the standard test idiom"
)]
mod tests {
    use super::*;

    #[test]
    fn unix_epoch_is_thursday_1970_01_01() {
        let dt = unix_to_datetime(0);
        assert_eq!((dt.year, dt.month, dt.day), (1970, 1, 1));
        assert_eq!(dt.weekday, 4); // Thursday
    }

    #[test]
    fn unix_to_datetime_round_trip_known_epochs() {
        // 2000-03-01 00:00 UTC — first day after a leap-day boundary.
        let dt = unix_to_datetime(951_868_800);
        assert_eq!((dt.year, dt.month, dt.day), (2000, 3, 1));
        // 2026-05-18 12:00 UTC.
        let dt = unix_to_datetime(1_779_796_800);
        assert_eq!((dt.year, dt.month, dt.day), (2026, 5, 18));
        assert_eq!((dt.hours, dt.minutes, dt.seconds), (12, 0, 0));
    }

    #[test]
    fn days_in_month_handles_leap_february() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2025, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
    }
}
