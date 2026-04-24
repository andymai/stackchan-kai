//! AW9523B helper: bring up the CoreS3 IO expander and reset the LCD.
//!
//! The AW9523B is a 16-pin I²C IO expander on the CoreS3 that gates several
//! peripherals behind software-controlled pins. For the LCD bring-up we need
//! two things from it:
//!
//! 1. **Full init** — both port-output registers, both port-direction
//!    registers, global-control (push-pull on P0), and the two LED-mode
//!    registers set to "all GPIO" so the pins drive a clean logic level
//!    rather than a limited LED-current source.
//! 2. **LCD reset pulse** on `P1_1` (= `ILI9342C.RESX`). The line must be
//!    pulsed low then high *after* AXP2101 LDOs are up, otherwise the
//!    panel latches bad state from mid-rising rails.
//!
//! Both the register values and the port-1 layout are copied verbatim from
//! M5Stack's CoreS3 reference init (`xiaozhi-esp32` +
//! `stackchan/main/hal/board/stackchan.cc`). Key bits on P1: bit 1 is
//! `LCD_RST`; bits 0, 3, 7 are other board-level enables that must latch
//! HIGH for the display module to be fully powered (the backlight boost
//! converter's enable sits on this port, which is why the screen stays
//! dark if we skip the P1 init and only drive P0).

use embassy_time::{Duration, Timer};
use embedded_hal_async::i2c::I2c;

/// 7-bit I²C address of the AW9523B on CoreS3 (`AD1 = AD0 = GND`).
const ADDRESS: u8 = 0x58;

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
const RESET_PULSE_MS: u64 = 20;
/// Wait after releasing reset before issuing the first SPI command. The
/// ILI9342C internal init sequence needs ≥120 ms per its datasheet.
const POST_RESET_SETTLE_MS: u64 = 120;

/// Apply the CoreS3 AW9523 defaults and pulse `LCD_RST`. Returns once the
/// LCD is ready for SPI init. Must be called after AXP2101 rails are up.
///
/// # Errors
///
/// Returns the underlying I²C error if any AW9523 register access fails.
pub async fn init_and_reset_lcd<B>(bus: &mut B) -> Result<(), B::Error>
where
    B: I2c,
{
    // M5Stack CoreS3 reference init order: output values first (so the
    // pins drive a known level the instant direction flips to output),
    // then direction, then global control / LED mode.
    bus.write(ADDRESS, &[REG_OUTPUT_P0, P0_OUTPUT_INIT]).await?;
    bus.write(ADDRESS, &[REG_OUTPUT_P1, P1_OUTPUT_INIT]).await?;
    bus.write(ADDRESS, &[REG_DIR_P0, P0_DIR_INIT]).await?;
    bus.write(ADDRESS, &[REG_DIR_P1, P1_DIR_INIT]).await?;
    bus.write(ADDRESS, &[REG_CONTROL, CONTROL_P0_PUSH_PULL])
        .await?;
    bus.write(ADDRESS, &[REG_LEDMODE_P0, LEDMODE_ALL_GPIO])
        .await?;
    bus.write(ADDRESS, &[REG_LEDMODE_P1, LEDMODE_ALL_GPIO])
        .await?;

    // LCD reset pulse: drop P1_1 only, keep boost-enable (bit 7) HIGH.
    bus.write(ADDRESS, &[REG_OUTPUT_P1, P1_OUTPUT_LCD_RESET])
        .await?;
    Timer::after(Duration::from_millis(RESET_PULSE_MS)).await;
    bus.write(ADDRESS, &[REG_OUTPUT_P1, P1_OUTPUT_INIT]).await?;
    Timer::after(Duration::from_millis(POST_RESET_SETTLE_MS)).await;
    Ok(())
}
