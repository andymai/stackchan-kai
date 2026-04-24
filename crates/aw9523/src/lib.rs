//! # aw9523
//!
//! `no_std` driver for the AW9523B I²C IO expander, with a board-specific
//! bring-up helper for the M5Stack CoreS3.
//!
//! The AW9523B is a 16-pin I²C IO expander that, on CoreS3, gates several
//! peripherals behind software-controlled pins. The one thing the LCD
//! pipeline needs from it is a clean reset pulse on `P1_1` (=
//! `ILI9342C.RESX`) *after* the AXP2101 LDOs are up — plus the full
//! port-output / direction / LED-mode / global-control init that lets
//! the rest of the board-level enables latch HIGH (notably the backlight
//! boost-converter enable on `P1_7`).
//!
//! The driver is generic over any [`embedded_hal_async::i2c::I2c`] and
//! [`embedded_hal_async::delay::DelayNs`], so any async runtime (embassy,
//! rtic, ...) can drive it.
//!
//! Register values and port-1 layout are copied verbatim from M5Stack's
//! CoreS3 reference init (`xiaozhi-esp32` +
//! `stackchan/main/hal/board/stackchan.cc`).
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::{i2c::I2c, delay::DelayNs};
//! # async fn demo<B, D>(mut bus: B, mut delay: D) -> Result<(), aw9523::Error<B::Error>>
//! # where B: I2c, D: DelayNs {
//! aw9523::init_cores3(&mut bus, &mut delay).await?;
//! # Ok(())
//! # }
//! ```

#![no_std]
#![deny(unsafe_code)]

use embedded_hal_async::{delay::DelayNs, i2c::I2c};

/// 7-bit I²C address of the AW9523B on CoreS3 (`AD1 = AD0 = GND`).
pub const CORES3_ADDRESS: u8 = 0x58;

/// Port-0 output register (one bit per pin).
const REG_OUTPUT_P0: u8 = 0x02;
/// Port-1 output register.
const REG_OUTPUT_P1: u8 = 0x03;
/// Port-0 direction register (`0 = output`, `1 = input`).
const REG_DIR_P0: u8 = 0x04;
/// Port-1 direction register.
const REG_DIR_P1: u8 = 0x05;
/// Global control: bit 4 selects push-pull (`1`) vs open-drain (`0`) on P0.
const REG_CONTROL: u8 = 0x11;
/// Port-0 LED-mode register (`1 = GPIO`, `0 = LED current-sink`).
const REG_LEDMODE_P0: u8 = 0x12;
/// Port-1 LED-mode register.
const REG_LEDMODE_P1: u8 = 0x13;

/// P0 output value after init: `P0_0`..`P0_2` HIGH (`LCD_RST`, `AW88298_RST`,
/// `TP_RST` all released), rest LOW.
const P0_OUTPUT_INIT: u8 = 0b0000_0111;
/// P1 output value after init: bits 0, 1, 3, 7 HIGH. Bit 1 = `LCD_RST` (HIGH
/// = released). Bit 7 is the backlight-boost enable — must be HIGH or the
/// panel stays dark even with `BLDO1` up.
const P1_OUTPUT_INIT: u8 = 0b1000_1111;
/// P1 output with `LCD_RST` asserted (bit 1 LOW), boost-enable (bit 7)
/// kept HIGH so the backlight rail doesn't drop during the reset pulse.
const P1_OUTPUT_LCD_RESET: u8 = 0b1000_0001;
/// P0 direction: bits 3 and 4 inputs (unused board signals), rest outputs.
const P0_DIR_INIT: u8 = 0b0001_1000;
/// P1 direction: bits 2 and 3 inputs (touch interrupt, tear-effect), rest
/// outputs.
const P1_DIR_INIT: u8 = 0b0000_1100;
/// `REG_CONTROL` value that switches port 0 to push-pull; leaves LED
/// current-scale bits at reset default `00`.
const CONTROL_P0_PUSH_PULL: u8 = 0x10;
/// All pins in GPIO mode (not LED current-sink mode).
const LEDMODE_ALL_GPIO: u8 = 0xFF;

/// Minimum `LCD_RST` low-pulse width. The ILI9342C datasheet requires
/// ≥10 µs; 20 ms matches the M5Stack reference and is harmless.
const RESET_PULSE_MS: u32 = 20;
/// Wait after releasing reset before issuing the first SPI command. The
/// ILI9342C internal init sequence needs ≥120 ms per its datasheet.
const POST_RESET_SETTLE_MS: u32 = 120;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// Apply the M5Stack CoreS3 AW9523 defaults and pulse `LCD_RST`.
///
/// Returns once the LCD is ready for SPI init. Must be called **after**
/// AXP2101 rails are up, otherwise the panel latches bad state from
/// mid-rising rails.
///
/// The reference init order (output values first, then direction, then
/// global control / LED mode) makes each pin drive a known logic level
/// the instant its direction flips to output.
///
/// # Errors
///
/// Returns the underlying I²C error if any AW9523 register access fails.
pub async fn init_cores3<B, D>(bus: &mut B, delay: &mut D) -> Result<(), Error<B::Error>>
where
    B: I2c,
    D: DelayNs,
{
    write_reg(bus, REG_OUTPUT_P0, P0_OUTPUT_INIT).await?;
    write_reg(bus, REG_OUTPUT_P1, P1_OUTPUT_INIT).await?;
    write_reg(bus, REG_DIR_P0, P0_DIR_INIT).await?;
    write_reg(bus, REG_DIR_P1, P1_DIR_INIT).await?;
    write_reg(bus, REG_CONTROL, CONTROL_P0_PUSH_PULL).await?;
    write_reg(bus, REG_LEDMODE_P0, LEDMODE_ALL_GPIO).await?;
    write_reg(bus, REG_LEDMODE_P1, LEDMODE_ALL_GPIO).await?;

    // LCD reset pulse: drop P1_1 only, keep boost-enable (bit 7) HIGH.
    write_reg(bus, REG_OUTPUT_P1, P1_OUTPUT_LCD_RESET).await?;
    delay.delay_ms(RESET_PULSE_MS).await;
    write_reg(bus, REG_OUTPUT_P1, P1_OUTPUT_INIT).await?;
    delay.delay_ms(POST_RESET_SETTLE_MS).await;
    Ok(())
}

/// Write `value` to the register at `reg` on the CoreS3-addressed AW9523.
async fn write_reg<B: I2c>(bus: &mut B, reg: u8, value: u8) -> Result<(), Error<B::Error>> {
    bus.write(CORES3_ADDRESS, &[reg, value]).await?;
    Ok(())
}
