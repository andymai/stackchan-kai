//! # gc0308
//!
//! Scaffold for a `no_std` async I²C control-path driver for the
//! `GalaxyCore` GC0308 VGA CMOS image sensor.
//!
//! Covers SCCB-style (I²C-compatible) register access for resolution,
//! output format, and streaming control. The actual pixel transport
//! happens over a parallel DVP bus handled by the MCU (ESP32-S3
//! `LCD_CAM` peripheral on the CoreS3) — that is deliberately out of
//! scope for this driver.
//!
//! ## Status
//!
//! Scaffold only. [`Gc0308::init`] currently validates the chip ID and
//! returns [`Ok`] without applying a register sequence. Fill in the
//! reset + window + format + stream-enable sequence before using.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::{i2c::I2c, delay::DelayNs};
//! # async fn demo<B, D>(bus: B, mut delay: D) -> Result<(), gc0308::Error<B::Error>>
//! # where B: I2c, D: DelayNs {
//! let mut cam = gc0308::Gc0308::new(bus);
//! cam.init(&mut delay).await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::{delay::DelayNs, i2c::I2c};

/// 7-bit I²C address. GC0308 has a fixed address on the SCCB bus.
pub const ADDRESS: u8 = 0x21;

/// Expected value of the `CHIP_ID` register (`0xF0`) on a genuine GC0308.
pub const CHIP_ID: u8 = 0x9B;

/// `CHIP_ID` register. Read-only. Expected to return [`CHIP_ID`].
const REG_CHIP_ID: u8 = 0xF0;

/// Post-power-on settle time before the first I²C read, in milliseconds.
///
/// Datasheet specifies ≥50 ms between PWDN release and the first valid
/// register access; 60 ms is the robust default.
const POWER_ON_DELAY_MS: u32 = 60;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
    /// `CHIP_ID` register did not return [`CHIP_ID`]. Contains the byte
    /// that was read.
    BadChipId(u8),
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// GC0308 driver handle. Owns the bus + address.
pub struct Gc0308<B> {
    /// Underlying I²C bus.
    bus: B,
    /// Resolved 7-bit I²C address.
    address: u8,
}

impl<B: I2c> Gc0308<B> {
    /// Wrap an I²C bus with the fixed GC0308 address.
    ///
    /// Does not touch the bus. Call [`Gc0308::init`] before any other
    /// register access.
    #[must_use]
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            address: ADDRESS,
        }
    }

    /// Resolved 7-bit I²C address. Useful for logging.
    #[must_use]
    pub const fn address(&self) -> u8 {
        self.address
    }

    /// Read the `CHIP_ID` register.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_chip_id(&mut self) -> Result<u8, Error<B::Error>> {
        let mut buf = [0u8; 1];
        self.bus
            .write_read(self.address, &[REG_CHIP_ID], &mut buf)
            .await?;
        Ok(buf[0])
    }

    /// Scaffold init. Waits for power-on, reads and validates the chip
    /// ID, and returns without applying a resolution / format / stream
    /// sequence yet.
    ///
    /// # Errors
    ///
    /// - [`Error::BadChipId`] if the chip does not report [`CHIP_ID`].
    /// - [`Error::I2c`] on any bus failure.
    pub async fn init<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<B::Error>> {
        delay.delay_ms(POWER_ON_DELAY_MS).await;
        let id = self.read_chip_id().await?;
        if id != CHIP_ID {
            return Err(Error::BadChipId(id));
        }
        // TODO: select page, soft-reset, configure output window,
        // YCbCr / RGB565 format, PCLK / HSYNC / VSYNC polarity,
        // enable streaming.
        Ok(())
    }
}
