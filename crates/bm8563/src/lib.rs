//! # bm8563
//!
//! `no_std` async driver for the NXP BM8563 real-time clock (a
//! pin-compatible clone of the PCF8563) over I²C. Scope: set / read
//! the date-time registers. Alarm, timer, and CLKOUT features are
//! deliberately out of scope for this driver.
//!
//! ## Why a bespoke `DateTime` instead of `chrono`?
//!
//! `chrono` requires `alloc` and drags in a surprising amount of
//! date-arithmetic code. All we need from the RTC is "hour/minute/
//! second + calendar date" in a shape the firmware can format into a
//! log line; callers that actually need date arithmetic can convert
//! our [`DateTime`] into their calendar library of choice.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::i2c::I2c;
//! # async fn demo<B: I2c>(bus: B) -> Result<(), bm8563::Error<B::Error>> {
//! let mut rtc = bm8563::Bm8563::new(bus);
//! rtc.init().await?;
//! let now = rtc.read_datetime().await?;
//! let _ = now.hours;
//! # Ok(())
//! # }
//! ```

#![no_std]
#![deny(unsafe_code)]

use embedded_hal_async::i2c::I2c;

/// Fixed 7-bit I²C address of the BM8563 / PCF8563.
pub const ADDRESS: u8 = 0x51;

/// `Control_1` register. Top bit `STOP`: `0` = RTC running, `1` = stopped.
const REG_CONTROL_1: u8 = 0x00;
/// `Control_2` register. Holds alarm / timer interrupt flags; we zero
/// it at init to disable them.
const REG_CONTROL_2: u8 = 0x01;
/// First byte of the date-time block (seconds, then minutes, hours,
/// days, weekdays, months+century, years). Seven consecutive
/// registers read in one burst.
const REG_VL_SECONDS: u8 = 0x02;

/// Mask used to strip the "voltage low" flag from the seconds byte.
const VL_SECONDS_MASK: u8 = 0x7F;
/// Mask for the two-digit BCD fields (all calendar values except year).
const BCD_TWO_DIGIT_MASK: u8 = 0x7F;
/// Mask for the day register (bits 5-0; bit 7 is undefined).
const DAYS_MASK: u8 = 0x3F;
/// Mask for the weekday register (bits 2-0).
const WEEKDAYS_MASK: u8 = 0x07;
/// Bit 7 of the months+century register: `1` = 20th century (years
/// 1900-1999), `0` = 21st (years 2000-2099).
const CENTURY_FLAG: u8 = 0x80;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// I²C transport error.
    I2c(E),
    /// The RTC's `VL` (voltage low) flag was set, signalling that the
    /// on-chip battery backup dropped and the current time is
    /// unreliable. Read the time anyway if you like, but treat it as
    /// "unset."
    VoltageLow,
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// A calendar date + wall-clock time, matching the layout the BM8563
/// stores. No timezone — the RTC is timezone-agnostic; assume UTC
/// unless you set it to something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DateTime {
    /// Four-digit Gregorian year (1900..=2099). The RTC's two-digit
    /// "year" register is expanded using its `CENTURY` flag.
    pub year: u16,
    /// 1..=12
    pub month: u8,
    /// 1..=31
    pub day: u8,
    /// 0..=6; 0 = Sunday (convention matches many C libraries and the
    /// Linux kernel; the BM8563 itself doesn't care which weekday is
    /// "zero").
    pub weekday: u8,
    /// 0..=23
    pub hours: u8,
    /// 0..=59
    pub minutes: u8,
    /// 0..=59
    pub seconds: u8,
}

/// BM8563 driver.
pub struct Bm8563<B> {
    /// I²C bus.
    bus: B,
}

impl<B: I2c> Bm8563<B> {
    /// Wrap an I²C bus.
    #[must_use]
    pub const fn new(bus: B) -> Self {
        Self { bus }
    }

    /// Zero `Control_1` and `Control_2`: clears the STOP flag (so the
    /// RTC runs) and disables any lingering alarm/timer interrupts.
    ///
    /// Does **not** set the time — that's a separate operation (see
    /// [`Bm8563::write_datetime`]). A freshly-powered RTC will have
    /// the `VL` flag set and the date-time fields at arbitrary values.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn init(&mut self) -> Result<(), Error<B::Error>> {
        self.write_register(REG_CONTROL_1, 0x00).await?;
        self.write_register(REG_CONTROL_2, 0x00).await?;
        Ok(())
    }

    /// Read the current date-time.
    ///
    /// # Errors
    ///
    /// - [`Error::VoltageLow`] if the RTC's `VL` flag is set, meaning
    ///   the current time is unreliable. The caller can choose to
    ///   retry, re-set the time, or treat the returned value as a
    ///   best-effort reading.
    /// - [`Error::I2c`] on any bus failure.
    pub async fn read_datetime(&mut self) -> Result<DateTime, Error<B::Error>> {
        let mut buf = [0u8; 7];
        self.bus
            .write_read(ADDRESS, &[REG_VL_SECONDS], &mut buf)
            .await?;
        if buf[0] & !VL_SECONDS_MASK != 0 {
            return Err(Error::VoltageLow);
        }
        Ok(decode_datetime(buf))
    }

    /// Set the RTC's date-time. Does **not** clear the `VL` flag
    /// automatically; the next read that succeeds will clear it
    /// (writing a valid seconds byte implicitly acknowledges `VL`).
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error. No validation — callers are
    /// responsible for passing sensible values (hour ≤ 23 etc.).
    pub async fn write_datetime(&mut self, dt: DateTime) -> Result<(), Error<B::Error>> {
        let mut buf = [0u8; 8];
        buf[0] = REG_VL_SECONDS;
        buf[1..].copy_from_slice(&encode_datetime(dt));
        self.bus.write(ADDRESS, &buf).await?;
        Ok(())
    }

    /// Single-register write helper.
    async fn write_register(&mut self, reg: u8, value: u8) -> Result<(), Error<B::Error>> {
        self.bus.write(ADDRESS, &[reg, value]).await?;
        Ok(())
    }
}

/// Convert a BCD-encoded byte back to its u8 value.
///
/// Takes the mask of "valid bits" — the BM8563 reuses upper bits of
/// several registers for flags that must be stripped before decoding.
const fn bcd_to_u8(bcd: u8, mask: u8) -> u8 {
    let masked = bcd & mask;
    ((masked >> 4) * 10) + (masked & 0x0F)
}

/// Encode a u8 as two-digit BCD. Saturates above 99.
const fn u8_to_bcd(value: u8) -> u8 {
    let clamped = if value > 99 { 99 } else { value };
    ((clamped / 10) << 4) | (clamped % 10)
}

/// Decode the 7-byte date-time burst into a [`DateTime`].
fn decode_datetime(buf: [u8; 7]) -> DateTime {
    let seconds = bcd_to_u8(buf[0], VL_SECONDS_MASK);
    let minutes = bcd_to_u8(buf[1], BCD_TWO_DIGIT_MASK);
    let hours = bcd_to_u8(buf[2], 0x3F);
    let day = bcd_to_u8(buf[3], DAYS_MASK);
    let weekday = buf[4] & WEEKDAYS_MASK;
    let month_century = buf[5];
    let month = bcd_to_u8(month_century, 0x1F);
    let year_two_digit = u16::from(bcd_to_u8(buf[6], 0xFF));
    // `CENTURY` bit = 1 means 20th century (1900-1999).
    let century_base: u16 = if month_century & CENTURY_FLAG != 0 {
        1900
    } else {
        2000
    };
    DateTime {
        year: century_base + year_two_digit,
        month,
        day,
        weekday,
        hours,
        minutes,
        seconds,
    }
}

/// Format a [`DateTime`] into `YYYY-MM-DD HH:MM:SS` (19 ASCII bytes)
/// in the caller's buffer.
///
/// Returns the formatted slice as a `&str`, which is always valid
/// UTF-8 because every byte is ASCII. Useful for writing one-off
/// wall-clock strings into log lines without pulling in `alloc`.
///
/// Values that exceed the normal calendar range (e.g. a glitched
/// sensor read producing `month = 99`) are clamped to two digits
/// via saturation, so the output length is always exactly 19 bytes.
pub fn format_datetime(dt: DateTime, out: &mut [u8; 19]) -> &str {
    // ASCII '0' = 0x30.
    let y = dt.year;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "year / 1000 and intermediates fit in 0..=9 after `% 10`"
    )]
    {
        out[0] = b'0' + (y / 1000) as u8;
        out[1] = b'0' + ((y / 100) % 10) as u8;
        out[2] = b'0' + ((y / 10) % 10) as u8;
        out[3] = b'0' + (y % 10) as u8;
    }
    out[4] = b'-';
    write_two_digits(&mut out[5..=6], dt.month);
    out[7] = b'-';
    write_two_digits(&mut out[8..=9], dt.day);
    out[10] = b' ';
    write_two_digits(&mut out[11..=12], dt.hours);
    out[13] = b':';
    write_two_digits(&mut out[14..=15], dt.minutes);
    out[16] = b':';
    write_two_digits(&mut out[17..=18], dt.seconds);
    // All 19 bytes are ASCII; conversion is infallible.
    core::str::from_utf8(out).unwrap_or("")
}

/// Write a two-digit decimal (0..=99) into a 2-byte slot as ASCII.
/// Values > 99 saturate to "99".
fn write_two_digits(slot: &mut [u8], value: u8) {
    let v = if value > 99 { 99 } else { value };
    slot[0] = b'0' + (v / 10);
    slot[1] = b'0' + (v % 10);
}

/// Encode a [`DateTime`] as the 7-byte register block the BM8563
/// writes to.
const fn encode_datetime(dt: DateTime) -> [u8; 7] {
    // Century: 1 if year in 1900s, 0 for 2000s+.
    let (century_bit, year_two_digit) = if dt.year < 2000 {
        (CENTURY_FLAG, dt.year.saturating_sub(1900))
    } else {
        (0, dt.year.saturating_sub(2000))
    };
    #[allow(
        clippy::cast_possible_truncation,
        reason = "year_two_digit was computed from a two-digit-range subtraction; fits in u8"
    )]
    let year_byte = u8_to_bcd(year_two_digit as u8);
    [
        u8_to_bcd(dt.seconds),
        u8_to_bcd(dt.minutes),
        u8_to_bcd(dt.hours),
        u8_to_bcd(dt.day),
        dt.weekday & WEEKDAYS_MASK,
        u8_to_bcd(dt.month) | century_bit,
        year_byte,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcd_roundtrip_common_values() {
        for v in [0_u8, 1, 9, 10, 29, 59, 99] {
            assert_eq!(bcd_to_u8(u8_to_bcd(v), 0xFF), v);
        }
    }

    #[test]
    fn bcd_saturates_above_99() {
        assert_eq!(u8_to_bcd(200), u8_to_bcd(99));
    }

    #[test]
    fn decode_2026_04_24_13_37_00_round_trips() {
        // Friday 2026-04-24 13:37:00.
        let dt = DateTime {
            year: 2026,
            month: 4,
            day: 24,
            weekday: 5, // Friday in "Sunday=0" convention
            hours: 13,
            minutes: 37,
            seconds: 0,
        };
        let bytes = encode_datetime(dt);
        let decoded = decode_datetime(bytes);
        assert_eq!(decoded, dt);
    }

    #[test]
    fn decode_century_flag_uses_20th_century() {
        // Century bit set → 1900s.
        let mut bytes = encode_datetime(DateTime {
            year: 1999,
            month: 12,
            day: 31,
            weekday: 5,
            hours: 23,
            minutes: 59,
            seconds: 59,
        });
        // Re-check the century bit survives the round-trip.
        assert!(bytes[5] & CENTURY_FLAG != 0);
        // Poke the month byte to simulate a raw register read.
        let _ = &mut bytes;
        assert_eq!(decode_datetime(bytes).year, 1999);
    }

    #[test]
    fn format_datetime_round_trips_example() {
        let dt = DateTime {
            year: 2026,
            month: 4,
            day: 24,
            weekday: 5,
            hours: 13,
            minutes: 37,
            seconds: 5,
        };
        let mut buf = [0u8; 19];
        let s = format_datetime(dt, &mut buf);
        assert_eq!(s, "2026-04-24 13:37:05");
    }

    #[test]
    fn format_datetime_saturates_pathological_input() {
        let dt = DateTime {
            year: 2099,
            month: 99,
            day: 99,
            weekday: 7,
            hours: 99,
            minutes: 99,
            seconds: 99,
        };
        let mut buf = [0u8; 19];
        let s = format_datetime(dt, &mut buf);
        assert_eq!(s, "2099-99-99 99:99:99");
    }

    #[test]
    fn vl_flag_stripped_from_seconds() {
        // Seconds register with VL bit set: raw bytes have the bit,
        // but the decoded value shouldn't. Manual wire-level check.
        let bytes = [0x80 | u8_to_bcd(42), 0, 0, 1, 0, u8_to_bcd(1), 0];
        let decoded = decode_datetime(bytes);
        assert_eq!(decoded.seconds, 42);
    }
}
