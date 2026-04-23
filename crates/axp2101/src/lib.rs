//! # axp2101
//!
//! Minimal `no_std` driver for the X-Powers AXP2101 PMIC used on the M5Stack
//! CoreS3. v0.1.0 implements the **smallest** register set needed to bring up
//! the LCD and 3V3 rails:
//!
//! - ALDO1 -- 3V3 system rail
//! - BLDO1 -- LCD backlight enable
//! - BLDO2 -- LCD logic rail
//! - Power-on sequencing helpers
//!
//! Battery monitoring, charging configuration, and button handling are left
//! for future releases; adding them is a matter of wiring more register
//! accesses through the existing I²C surface.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::i2c::I2c;
//! # async fn demo<B: I2c>(bus: B) -> Result<(), axp2101::Error<B::Error>> {
//! let mut pmic = axp2101::Axp2101::new(bus);
//! pmic.enable_lcd_rails().await?;
//! # Ok(())
//! # }
//! ```

#![no_std]
#![deny(unsafe_code)]

use embedded_hal_async::i2c::I2c;

/// 7-bit I²C address of the AXP2101 on CoreS3.
pub const ADDRESS: u8 = 0x34;

/// AXP2101 register offsets (partial; extend as features are added).
mod reg {
    /// ALDO enable bitmap.
    pub const LDO_ONOFF_CTL0: u8 = 0x90;
    /// BLDO enable bitmap.
    pub const LDO_ONOFF_CTL1: u8 = 0x91;
    /// ALDO1 voltage register (0.5V base + 100mV steps).
    pub const ALDO1_VOLT: u8 = 0x92;
    /// BLDO1 voltage register.
    pub const BLDO1_VOLT: u8 = 0x96;
    /// BLDO2 voltage register.
    pub const BLDO2_VOLT: u8 = 0x97;
}

/// Error type for the driver.
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

/// AXP2101 driver. Holds the I²C bus and issues register reads/writes.
pub struct Axp2101<B> {
    /// Underlying I²C bus.
    bus: B,
}

impl<B> Axp2101<B>
where
    B: I2c,
{
    /// Construct a new driver bound to `bus` at the default address.
    pub const fn new(bus: B) -> Self {
        Self { bus }
    }

    /// Turn on the LDOs required for the CoreS3 LCD + 3V3 rails:
    /// ALDO1 @ 3.3V (system), BLDO1 @ 3.3V (backlight), BLDO2 @ 3.3V (logic).
    ///
    /// Must be called before any SPI transactions targeting the LCD.
    ///
    /// # Errors
    ///
    /// Returns [`Error::I2c`] if any underlying I²C transaction fails.
    pub async fn enable_lcd_rails(&mut self) -> Result<(), Error<B::Error>> {
        // ALDO voltage register encoding: value * 100 mV, offset 500 mV.
        // 3.3V -> (3300 - 500) / 100 = 28.
        self.write_reg(reg::ALDO1_VOLT, 28).await?;
        self.write_reg(reg::BLDO1_VOLT, 28).await?;
        self.write_reg(reg::BLDO2_VOLT, 28).await?;

        // Set enable bits. LDO_ONOFF_CTL0 bit 0 = ALDO1.
        // LDO_ONOFF_CTL1 bit 0 = BLDO1, bit 1 = BLDO2.
        let aldo_mask = self.read_reg(reg::LDO_ONOFF_CTL0).await?;
        self.write_reg(reg::LDO_ONOFF_CTL0, aldo_mask | 0x01)
            .await?;

        let bldo_mask = self.read_reg(reg::LDO_ONOFF_CTL1).await?;
        self.write_reg(reg::LDO_ONOFF_CTL1, bldo_mask | 0x03)
            .await?;

        Ok(())
    }

    /// Read a single byte from a register.
    ///
    /// # Errors
    ///
    /// Returns [`Error::I2c`] on bus errors.
    pub async fn read_reg(&mut self, reg: u8) -> Result<u8, Error<B::Error>> {
        let mut buf = [0u8];
        self.bus
            .write_read(ADDRESS, &[reg], &mut buf)
            .await
            .map_err(Error::I2c)?;
        Ok(buf[0])
    }

    /// Write a single byte to a register.
    ///
    /// # Errors
    ///
    /// Returns [`Error::I2c`] on bus errors.
    pub async fn write_reg(&mut self, reg: u8, value: u8) -> Result<(), Error<B::Error>> {
        self.bus
            .write(ADDRESS, &[reg, value])
            .await
            .map_err(Error::I2c)?;
        Ok(())
    }
}
