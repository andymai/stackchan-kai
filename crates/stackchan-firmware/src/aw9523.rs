//! Minimal AW9523B helper: release the CoreS3 LCD's reset line.
//!
//! The AW9523B is a 16-pin I²C IO expander on the CoreS3. Pin P0_0 drives
//! the ILI9342C's `RESX` line — it's held low at cold boot, so the LCD
//! stays in reset until we configure the pin as a push-pull output and
//! drive it high.
//!
//! This module is intentionally narrow: one function that touches only
//! P0_0. Other P0 pins (touch reset, bus enable) stay in their power-on
//! default (input / high-impedance), so the write here cannot disturb
//! peripherals we don't own.
//!
//! Register layout comes from the Awinic AW9523B datasheet (English),
//! Table 4.1 "Register Map".

use embassy_time::{Duration, Timer};
use embedded_hal_async::i2c::I2c;

/// 7-bit I²C address of the AW9523B on CoreS3 (`AD1=AD0=GND`).
const ADDRESS: u8 = 0x58;
/// Output-value register for port 0 (one bit per pin).
const REG_OUTPUT_P0: u8 = 0x02;
/// Direction register for port 0 (`0 = output`, `1 = input`).
const REG_DIR_P0: u8 = 0x04;
/// Global control: bit 4 selects push-pull (`1`) vs open-drain (`0`) on P0.
const REG_CONTROL: u8 = 0x11;
/// `REG_CONTROL` value that switches port 0 to push-pull, leaves the LED
/// current-scale bits at their reset default of `00`.
const CONTROL_P0_PUSH_PULL: u8 = 0x10;
/// Bitmask for P0_0 with all other pins left as inputs (`0b1111_1110`).
const DIR_P0_0_OUTPUT: u8 = 0xFE;
/// Output register: P0_0 high, other bits don't matter (they're inputs).
const OUTPUT_P0_0_HIGH: u8 = 0x01;
/// Output register: P0_0 low, other bits don't matter (they're inputs).
const OUTPUT_P0_0_LOW: u8 = 0x00;

/// Minimum reset-low time, in milliseconds. ILI9342C datasheet requires ≥10 µs;
/// 5 ms is a safe, vendor-example-matched margin.
const RESET_PULSE_MS: u64 = 5;
/// Wait after releasing reset before issuing the first SPI command. The
/// ILI9342C internal init sequence needs ≥120 ms per its datasheet.
const POST_RESET_SETTLE_MS: u64 = 120;

/// Pulse the LCD reset line low then high, leaving the LCD ready for
/// SPI configuration. Must be called after AXP2101 rails are up.
///
/// # Errors
///
/// Returns the underlying I²C error if any AW9523B register access fails.
pub async fn release_lcd_reset<B>(bus: &mut B) -> Result<(), B::Error>
where
    B: I2c,
{
    // Port 0 in push-pull mode (otherwise driving a logic high needs an
    // external pull-up, which the CoreS3 board doesn't provide on P0_0).
    bus.write(ADDRESS, &[REG_CONTROL, CONTROL_P0_PUSH_PULL])
        .await?;
    // Pre-load the output register with P0_0 low before flipping the
    // direction bit, so the first edge out of the pin is cleanly asserted.
    bus.write(ADDRESS, &[REG_OUTPUT_P0, OUTPUT_P0_0_LOW])
        .await?;
    // Flip P0_0 to output — P0_1..P0_7 stay input (high-Z) so peripherals
    // we don't control (touch reset, bus enable) keep their default state.
    bus.write(ADDRESS, &[REG_DIR_P0, DIR_P0_0_OUTPUT]).await?;

    Timer::after(Duration::from_millis(RESET_PULSE_MS)).await;

    bus.write(ADDRESS, &[REG_OUTPUT_P0, OUTPUT_P0_0_HIGH])
        .await?;
    Timer::after(Duration::from_millis(POST_RESET_SETTLE_MS)).await;
    Ok(())
}
