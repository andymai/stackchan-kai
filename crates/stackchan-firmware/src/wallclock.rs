//! BM8563 wall-clock read, used to timestamp a single boot log line.
//!
//! The `defmt` timestamp stays as `embassy_time::Instant::now()` in
//! milliseconds — that's what the embassy / defmt integration expects.
//! This module exists to give the boot log an absolute time ("boot @
//! 2026-04-24 13:37:05") so post-reboot logs can be correlated.
//!
//! The RTC stores UTC; [`apply_offset`] converts a UTC `DateTime` into
//! local time using a signed minutes-from-UTC offset (typically derived
//! from `Config::time::tz` via
//! [`stackchan_net::tz_offset_minutes`]).
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

/// Days in each month (non-leap year). Indexed by month - 1.
const DAYS_IN_MONTH: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// `true` iff `year` is a Gregorian leap year (divisible by 4, except
/// century years not divisible by 400).
fn is_leap(year: u16) -> bool {
    let y = u32::from(year);
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

/// Days in a specific year/month. Returns 29 for February in leap years.
fn days_in_month(year: u16, month: u8) -> u32 {
    if month == 2 && is_leap(year) {
        return 29;
    }
    DAYS_IN_MONTH[(month as usize) - 1]
}

/// Apply a signed minutes-from-UTC offset to a UTC [`DateTime`].
///
/// `offset_minutes` may be negative (zones west of UTC) or positive
/// (east). The conversion handles month / year rollover in either
/// direction without bringing in `chrono` — Stack-chan's RTC range is
/// 2000-01-01..2099-12-31 (BM8563 chip limit), well inside `u16` year
/// arithmetic.
///
/// Returns the input unchanged when `offset_minutes` is zero so the
/// common `tz: "UTC"` path doesn't pay any conversion cost.
#[must_use]
pub fn apply_offset(utc: DateTime, offset_minutes: i32) -> DateTime {
    if offset_minutes == 0 {
        return utc;
    }
    let total_minutes =
        i64::from(utc.hours) * 60 + i64::from(utc.minutes) + i64::from(offset_minutes);
    // Floor-divide to handle the negative case correctly: for
    // `-90 minutes`, we want -2 day-rollover and +1350 min-of-day, not
    // -1 and -90.
    let day_offset = total_minutes.div_euclid(1440);
    let mins_of_day = total_minutes.rem_euclid(1440);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let new_hours = (mins_of_day / 60) as u8;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let new_minutes = (mins_of_day % 60) as u8;

    let mut year = utc.year;
    let mut month = utc.month;
    let mut day = i64::from(utc.day) + day_offset;

    // Walk forward / backward across month + year boundaries until
    // `day` falls inside the current month.
    while day < 1 {
        // Roll back to the previous month.
        if month == 1 {
            month = 12;
            year -= 1;
        } else {
            month -= 1;
        }
        day += i64::from(days_in_month(year, month));
    }
    loop {
        let dim = i64::from(days_in_month(year, month));
        if day <= dim {
            break;
        }
        day -= dim;
        if month == 12 {
            month = 1;
            year += 1;
        } else {
            month += 1;
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let new_day = day as u8;

    DateTime {
        year,
        month,
        day: new_day,
        hours: new_hours,
        minutes: new_minutes,
        seconds: utc.seconds,
        // Weekday rolls with day-of-week arithmetic which the BM8563
        // tracks itself; on a derived local-time `DateTime` we don't
        // try to recompute it (callers only care about year-second).
        weekday: utc.weekday,
    }
}

#[cfg(test)]
#[allow(clippy::missing_docs_in_private_items, reason = "test fixtures")]
mod tests {
    use super::*;

    fn dt(year: u16, month: u8, day: u8, hours: u8, minutes: u8, seconds: u8) -> DateTime {
        DateTime {
            year,
            month,
            day,
            hours,
            minutes,
            seconds,
            weekday: 0,
        }
    }

    #[test]
    fn apply_offset_zero_returns_input() {
        let d = dt(2026, 5, 7, 12, 0, 0);
        assert_eq!(apply_offset(d, 0), d);
    }

    #[test]
    fn apply_offset_positive_within_day() {
        // 09:00 UTC + 9 hours = 18:00 same day (Tokyo).
        let d = dt(2026, 5, 7, 9, 0, 0);
        let local = apply_offset(d, 9 * 60);
        assert_eq!(local.hours, 18);
        assert_eq!(local.day, 7);
    }

    #[test]
    fn apply_offset_positive_crosses_midnight_into_next_day() {
        // 23:30 UTC + 9 hours = 08:30 next day.
        let d = dt(2026, 5, 7, 23, 30, 0);
        let local = apply_offset(d, 9 * 60);
        assert_eq!(local.hours, 8);
        assert_eq!(local.minutes, 30);
        assert_eq!(local.day, 8);
    }

    #[test]
    fn apply_offset_negative_crosses_midnight_into_prior_day() {
        // 03:00 UTC - 8 hours = 19:00 prior day (Pacific).
        let d = dt(2026, 5, 7, 3, 0, 0);
        let local = apply_offset(d, -8 * 60);
        assert_eq!(local.hours, 19);
        assert_eq!(local.day, 6);
    }

    #[test]
    fn apply_offset_rolls_month_backward() {
        // 01:00 UTC on the 1st of May, -8 hours = 17:00 on 30 April.
        let d = dt(2026, 5, 1, 1, 0, 0);
        let local = apply_offset(d, -8 * 60);
        assert_eq!(local.month, 4);
        assert_eq!(local.day, 30);
        assert_eq!(local.hours, 17);
    }

    #[test]
    fn apply_offset_rolls_year_forward_at_midnight_dec_31() {
        // 23:00 UTC on Dec 31, +9 hours = 08:00 on Jan 1 next year.
        let d = dt(2026, 12, 31, 23, 0, 0);
        let local = apply_offset(d, 9 * 60);
        assert_eq!(local.year, 2027);
        assert_eq!(local.month, 1);
        assert_eq!(local.day, 1);
        assert_eq!(local.hours, 8);
    }

    #[test]
    fn apply_offset_rolls_year_backward_at_midnight_jan_1() {
        // 03:00 UTC on Jan 1, -8 hours = 19:00 on Dec 31 prior year.
        let d = dt(2026, 1, 1, 3, 0, 0);
        let local = apply_offset(d, -8 * 60);
        assert_eq!(local.year, 2025);
        assert_eq!(local.month, 12);
        assert_eq!(local.day, 31);
        assert_eq!(local.hours, 19);
    }

    #[test]
    fn apply_offset_handles_leap_day_february() {
        // 03:00 UTC on March 1 2024, -8 hours = 19:00 on Feb 29 2024.
        let d = dt(2024, 3, 1, 3, 0, 0);
        let local = apply_offset(d, -8 * 60);
        assert_eq!(local.month, 2);
        assert_eq!(local.day, 29);
    }

    #[test]
    fn apply_offset_handles_non_leap_february() {
        // Same as above but 2025 is not a leap year, so we land on Feb 28.
        let d = dt(2025, 3, 1, 3, 0, 0);
        let local = apply_offset(d, -8 * 60);
        assert_eq!(local.month, 2);
        assert_eq!(local.day, 28);
    }

    #[test]
    fn apply_offset_handles_half_hour_zone() {
        // India: +5h30. 18:00 UTC + 5h30 = 23:30 same day.
        let d = dt(2026, 5, 7, 18, 0, 0);
        let local = apply_offset(d, 5 * 60 + 30);
        assert_eq!(local.hours, 23);
        assert_eq!(local.minutes, 30);
        assert_eq!(local.day, 7);
    }
}
