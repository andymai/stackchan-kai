//! SD-card-side `embedded_hal::spi::SpiDevice` adapter for the shared
//! SPI2 bus on M5Stack CoreS3.
//!
//! ## Why this exists
//!
//! CoreS3 wires the SD card MISO line to **GPIO35** — the same physical
//! pin the LCD uses as DC (Data/Command). The LCD is write-only at the
//! peripheral level, so it never reads MISO; the SD client is read/write
//! and needs MISO. M5GFX (Arduino C++) handles this by toggling the
//! pin's output-enable bit per CS edge:
//!
//! - **CS LOW (LCD)** → OE on  → GPIO35 drives DC value (output)
//! - **CS LOW (SD)**  → OE off → GPIO35 floats; SPI MISO reads it (input)
//!
//! The fancier alternative — splitting the pin via `peripheral_input`
//! / `OutputSignal::connect_to` — relies on `#[doc(hidden)]` esp-hal
//! 1.0 surfaces (tracked in esp-rs/esp-hal#2876, currently blocked).
//! A direct register write to `GPIO_ENABLE1_W1TS_REG` /
//! `GPIO_ENABLE1_W1TC_REG` is what M5GFX actually does and is the
//! pattern this module follows.

// Direct register access for the GPIO35 OE flip. The unsafe surface is
// two `core::ptr::write_volatile` calls in `set_dc_drive_*` below, both
// guarded by:
//   - the register address comes from the ESP32-S3 TRM (chapter 5,
//     "GPIO and IO_MUX") and never changes;
//   - the bit position is statically computed (35 - 32 = 3);
//   - the W1TS / W1TC pattern is atomic in hardware (single-cycle
//     bit-set / bit-clear), so no read-modify-write races even across
//     interrupt context.
#![allow(unsafe_code)]

use core::cell::RefCell;

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;
use embedded_hal::spi::{ErrorType, Operation, SpiBus, SpiDevice};
use esp_hal::Blocking;
use esp_hal::spi::master::Spi;

/// `GPIO_ENABLE1_W1TS_REG` — write `1` to set output-enable bits for
/// pins 32–48. ESP32-S3 TRM, chapter 5.
const GPIO_ENABLE1_W1TS_REG: *mut u32 = 0x6000_4030 as *mut u32;
/// `GPIO_ENABLE1_W1TC_REG` — write `1` to clear output-enable bits.
const GPIO_ENABLE1_W1TC_REG: *mut u32 = 0x6000_4034 as *mut u32;
/// Bit position of GPIO35 in the ENABLE1 register (35 − 32 = 3).
const GPIO35_OE_MASK: u32 = 1 << 3;

/// Enable GPIO35's output driver (LCD DC mode).
///
/// Hardware-atomic: W1TS sets only the bits with a `1` in the written
/// value; other bits are untouched.
#[inline]
fn set_gpio35_oe_high() {
    // SAFETY: address from TRM, bit pattern statically derived,
    // W1TS register semantics atomic per TRM 5.2.5.
    unsafe {
        core::ptr::write_volatile(GPIO_ENABLE1_W1TS_REG, GPIO35_OE_MASK);
    }
}

/// Disable GPIO35's output driver (SD MISO mode — pin floats, input
/// buffer reads externally-driven level).
#[inline]
fn set_gpio35_oe_low() {
    // SAFETY: same as `set_gpio35_oe_high` — atomic W1TC write.
    unsafe {
        core::ptr::write_volatile(GPIO_ENABLE1_W1TC_REG, GPIO35_OE_MASK);
    }
}

/// `embedded-hal` `SpiDevice` for the SD-card client on the shared SPI2 bus.
///
/// Borrows the bus from a `RefCell` and flips GPIO35's OE bit around
/// each transaction so the SPI peripheral can read MISO while the
/// LCD's DC pin (the same physical GPIO35) is held floating.
pub struct SdSpiDevice<'a, CS, D> {
    /// Shared SPI2 bus (also used by the LCD via its own `RefCellDevice`).
    bus: &'a RefCell<Spi<'static, Blocking>>,
    /// SD chip-select line — GPIO4 on CoreS3, active low.
    cs: CS,
    /// Inter-byte delay used between bus operations and around CS edges.
    delay: D,
}

impl<'a, CS, D> SdSpiDevice<'a, CS, D>
where
    CS: OutputPin,
    D: DelayNs,
{
    /// Construct a new SD-side device against the shared SPI2 bus.
    /// Caller is responsible for keeping `bus` live for at least the
    /// lifetime of this device.
    #[must_use]
    pub const fn new(bus: &'a RefCell<Spi<'static, Blocking>>, cs: CS, delay: D) -> Self {
        Self { bus, cs, delay }
    }
}

/// Error from an SD-side SPI transaction.
///
/// Mirrors the variants `embedded_hal_bus::spi::DeviceError` exposes
/// for `RefCellDevice`, plus the CS-pin error path. Logged via
/// `defmt::Debug2Format` at the firmware boundary.
#[derive(Debug, defmt::Format)]
#[non_exhaustive]
pub enum SdSpiError {
    /// SPI bus reported an error (clocking, configuration).
    Spi,
    /// CS-pin write failed.
    Cs,
}

impl embedded_hal::spi::Error for SdSpiError {
    fn kind(&self) -> embedded_hal::spi::ErrorKind {
        embedded_hal::spi::ErrorKind::Other
    }
}

impl<CS, D> ErrorType for SdSpiDevice<'_, CS, D>
where
    CS: OutputPin,
    D: DelayNs,
{
    type Error = SdSpiError;
}

impl<CS, D> SpiDevice for SdSpiDevice<'_, CS, D>
where
    CS: OutputPin,
    D: DelayNs,
{
    fn transaction(&mut self, operations: &mut [Operation<'_, u8>]) -> Result<(), Self::Error> {
        // Borrow the shared bus for the entire transaction. Single-core
        // cooperative scheduling means no other task runs while we hold
        // this borrow — see the `RefCellDevice` rationale at
        // `crates/stackchan-firmware/src/main.rs` (LCD path).
        let mut bus = self.bus.borrow_mut();

        // Switch GPIO35 from "LCD DC output" to "SD MISO input" before
        // pulling CS low. Pin floats; the SPI peripheral's MISO matrix
        // signal reads the externally-driven level from the SD card.
        set_gpio35_oe_low();

        self.cs.set_low().map_err(|_| SdSpiError::Cs)?;

        // Run each operation against the shared bus. We map every
        // bus-error variant to `SdSpiError::Spi` because the
        // upstream esp-hal `Error` doesn't carry actionable detail
        // for this layer; the operator triages through the wider
        // boot log.
        let result = (|| -> Result<(), SdSpiError> {
            for op in operations.iter_mut() {
                match op {
                    Operation::Read(buf) => {
                        bus.read(buf).map_err(|_| SdSpiError::Spi)?;
                    }
                    Operation::Write(buf) => {
                        bus.write(buf).map_err(|_| SdSpiError::Spi)?;
                    }
                    Operation::Transfer(read, write) => {
                        // Disambiguate against esp-hal's inherent `Spi::transfer`
                        // (single-arg in-place form) — the `embedded-hal` trait
                        // method takes both buffers.
                        <Spi<'static, Blocking> as embedded_hal::spi::SpiBus>::transfer(
                            &mut bus, read, write,
                        )
                        .map_err(|_| SdSpiError::Spi)?;
                    }
                    Operation::TransferInPlace(buf) => {
                        bus.transfer_in_place(buf).map_err(|_| SdSpiError::Spi)?;
                    }
                    Operation::DelayNs(ns) => {
                        self.delay.delay_ns(*ns);
                    }
                }
            }
            Ok(())
        })();

        // CS high regardless of the result so we don't leave the SD
        // card selected on a partial failure.
        let cs_result = self.cs.set_high().map_err(|_| SdSpiError::Cs);

        // Restore GPIO35 to LCD DC output mode before any subsequent
        // LCD transaction reuses the bus.
        set_gpio35_oe_high();

        result.and(cs_result)
    }
}
