//! # aw88298
//!
//! Scaffold for a `no_std` async I²C control-path driver for the
//! Awinic AW88298 16-bit I2S "smart K" digital audio amplifier.
//!
//! On the CoreS3 Stack-chan the AW88298 drives the single 1 W speaker
//! from an integrated 10.25 V smart boost converter. The I²C side
//! handles configuration (I2S format, volume, fade, boost voltage,
//! thermal protection); audio data arrives over I2S from the MCU.
//!
//! ## Status
//!
//! Scaffold only. [`Aw88298::init`] releases the external reset,
//! verifies the chip ID, and returns without configuring any amplifier
//! register.
//!
//! ## External reset
//!
//! The AW88298's `RST` pin is wired to the AW9523 I/O expander on the
//! CoreS3 (port `P0_1`). Release it via the `aw9523` crate's CoreS3
//! bring-up helper *before* calling [`Aw88298::init`]; otherwise every
//! I²C transaction NACKs.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::{i2c::I2c, delay::DelayNs};
//! # async fn demo<B, D>(bus: B, mut delay: D) -> Result<(), aw88298::Error<B::Error>>
//! # where B: I2c, D: DelayNs {
//! let mut amp = aw88298::Aw88298::new(bus);
//! amp.init(&mut delay).await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::{delay::DelayNs, i2c::I2c};

/// 7-bit I²C address. `AD1 = AD2 = 0` yields `0x36` (the CoreS3 wiring).
///
/// Datasheet address format is `0b01101xx`; strap pins set the two
/// LSBs, giving the `0x34..=0x37` window.
pub const ADDRESS: u8 = 0x36;

/// Expected value of the `CHIPID` register on a genuine AW88298.
///
/// Datasheet specifies the two-byte chip ID as `0x1852` (MSB-first read
/// from `REG_CHIPID`).
pub const CHIP_ID: u16 = 0x1852;

/// `CHIPID` register. Read-only. Two-byte value, MSB at offset 0.
const REG_CHIP_ID: u8 = 0x00;

/// `SYSCTRL` register. Bits 0 (`PWDN`), 1 (`AMPPD`), 2 (`I2SEN`).
/// Writing `0x00` wakes the amplifier from power-down.
const REG_SYSCTRL: u8 = 0x04;

/// Post-reset settle delay, in milliseconds.
///
/// Datasheet requires ≥1 ms between `RST` release and the first I²C
/// transaction; 5 ms is conservative.
const RESET_SETTLE_MS: u32 = 5;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
    /// `CHIPID` register did not return [`CHIP_ID`]. Contains the raw
    /// 16-bit value that was read.
    BadChipId(u16),
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// AW88298 driver handle.
pub struct Aw88298<B> {
    /// Underlying I²C bus.
    bus: B,
    /// Resolved 7-bit I²C address.
    address: u8,
}

impl<B: I2c> Aw88298<B> {
    /// Wrap an I²C bus with the default [`ADDRESS`].
    #[must_use]
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            address: ADDRESS,
        }
    }

    /// Wrap an I²C bus with a specific address (for strap variants).
    #[must_use]
    pub const fn with_address(bus: B, address: u8) -> Self {
        Self { bus, address }
    }

    /// Resolved 7-bit I²C address. Useful for logging.
    #[must_use]
    pub const fn address(&self) -> u8 {
        self.address
    }

    /// Read the 16-bit `CHIPID` register (big-endian on the wire).
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_chip_id(&mut self) -> Result<u16, Error<B::Error>> {
        let mut buf = [0u8; 2];
        self.bus
            .write_read(self.address, &[REG_CHIP_ID], &mut buf)
            .await?;
        Ok(u16::from_be_bytes(buf))
    }

    /// Scaffold init. Waits post-reset, verifies the chip ID, and
    /// returns. Does not configure I2S format, volume, boost, or
    /// thermal protection.
    ///
    /// Caller is responsible for releasing the AW88298 `RST` pin via
    /// AW9523 before calling this function.
    ///
    /// # Errors
    ///
    /// - [`Error::BadChipId`] if `CHIPID` doesn't read `0x1852`.
    /// - [`Error::I2c`] on any bus failure.
    pub async fn init<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<B::Error>> {
        delay.delay_ms(RESET_SETTLE_MS).await;
        let id = self.read_chip_id().await?;
        if id != CHIP_ID {
            return Err(Error::BadChipId(id));
        }
        // TODO: SYSCTRL power-up, I2SCTRL format (16-bit, 48 kHz,
        // Philips I2S), BSTCTRL boost voltage, HAGCCFG thermal /
        // protection, VSNCTRL fade curve, PWMCTRL PWM frequency.
        Ok(())
    }

    /// Placeholder for the eventual mute/unmute API. Writes
    /// [`REG_SYSCTRL`] with `AMPPD` set or cleared.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn set_muted(&mut self, muted: bool) -> Result<(), Error<B::Error>> {
        let value = if muted { 0b0000_0010 } else { 0x00 };
        self.bus.write(self.address, &[REG_SYSCTRL, value]).await?;
        Ok(())
    }
}
