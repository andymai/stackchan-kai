//! # es7210
//!
//! Scaffold for a `no_std` async I²C control-path driver for the
//! Everest ES7210 four-channel 24-bit audio ADC.
//!
//! On the CoreS3 Stack-chan the ES7210 captures the two on-board
//! microphones. The remaining two ADC channels are unused. Audio data
//! is clocked out over I2S/TDM; the I²C side only handles register
//! configuration (sample rate, MCLK divisor, gain, channel enable,
//! TDM format).
//!
//! ## Status
//!
//! Scaffold only. [`Es7210::init`] validates the chip ID and returns
//! without writing the rate / gain / enable registers.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::{i2c::I2c, delay::DelayNs};
//! # async fn demo<B, D>(bus: B, mut delay: D) -> Result<(), es7210::Error<B::Error>>
//! # where B: I2c, D: DelayNs {
//! let mut adc = es7210::Es7210::new(bus);
//! adc.init(&mut delay).await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::{delay::DelayNs, i2c::I2c};

/// Default 7-bit I²C address with `AD1 = AD0 = GND`.
///
/// The chip's two strap pins let AD move across `0x40..=0x43`; the
/// CoreS3 Stack-chan wires both low, giving `0x40`.
pub const ADDRESS: u8 = 0x40;

/// Expected chip-ID low byte at [`REG_CHIP_ID1`].
///
/// ES7210 datasheet specifies `CHIP_ID1 = 0x72, CHIP_ID2 = 0x10` for
/// the production stepping.
pub const CHIP_ID1: u8 = 0x72;
/// Expected chip-ID high byte at [`REG_CHIP_ID2`].
pub const CHIP_ID2: u8 = 0x10;

/// `CHIP_ID1` register (low byte of the two-byte ID).
const REG_CHIP_ID1: u8 = 0xFD;
/// `CHIP_ID2` register (high byte).
const REG_CHIP_ID2: u8 = 0xFE;
/// `RESET` register. Writing `0x3F` asserts a soft-reset; `0x00` releases.
const REG_RESET: u8 = 0x00;

/// Post-soft-reset settle delay, in milliseconds.
///
/// ES7210 datasheet asks for "a few ms" after releasing reset before
/// configuration writes; 5 ms is conservative.
const RESET_SETTLE_MS: u32 = 5;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
    /// Chip-ID registers did not match the expected `(0x72, 0x10)`.
    /// Contains the raw bytes read from (`CHIP_ID1`, `CHIP_ID2`).
    BadChipId(u8, u8),
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// ES7210 driver handle.
pub struct Es7210<B> {
    /// Underlying I²C bus.
    bus: B,
    /// Resolved 7-bit I²C address.
    address: u8,
}

impl<B: I2c> Es7210<B> {
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

    /// Read the two chip-ID bytes.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_chip_id(&mut self) -> Result<(u8, u8), Error<B::Error>> {
        let mut lo = [0u8; 1];
        let mut hi = [0u8; 1];
        self.bus
            .write_read(self.address, &[REG_CHIP_ID1], &mut lo)
            .await?;
        self.bus
            .write_read(self.address, &[REG_CHIP_ID2], &mut hi)
            .await?;
        Ok((lo[0], hi[0]))
    }

    /// Scaffold init. Soft-resets, verifies chip ID, and returns. Does
    /// not configure sample rate, MCLK divisor, channel enable, TDM
    /// format, or gain.
    ///
    /// # Errors
    ///
    /// - [`Error::BadChipId`] if the chip ID bytes don't match.
    /// - [`Error::I2c`] on any bus failure.
    pub async fn init<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<B::Error>> {
        self.write_register(REG_RESET, 0x3F).await?;
        delay.delay_ms(RESET_SETTLE_MS).await;
        self.write_register(REG_RESET, 0x00).await?;
        delay.delay_ms(RESET_SETTLE_MS).await;

        let (lo, hi) = self.read_chip_id().await?;
        if lo != CHIP_ID1 || hi != CHIP_ID2 {
            return Err(Error::BadChipId(lo, hi));
        }

        // TODO: clock manager, MCLK divisor, ADC sample rate, ADC gain
        // per channel, channel enable, TDM / I2S format.
        Ok(())
    }

    /// Single-register write. Private because the scaffold does not
    /// expose arbitrary register access yet.
    async fn write_register(&mut self, reg: u8, value: u8) -> Result<(), Error<B::Error>> {
        self.bus.write(self.address, &[reg, value]).await?;
        Ok(())
    }
}
