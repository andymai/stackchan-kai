//! # st25r3916
//!
//! Scaffold for a `no_std` async SPI driver for the
//! `STMicroelectronics` ST25R3916 — a full-featured NFC reader /
//! writer IC. On the CoreS3
//! Stack-chan the chip provides tag detection and card read / write
//! over its integrated antenna matching network.
//!
//! ## Scope
//!
//! This crate targets the low-level SPI transport: register reads /
//! writes, direct commands, and the ~4-byte startup sanity-check
//! (probe `IC_IDENTITY`, confirm this is a `3916`). Protocol stacks
//! (ISO 14443-A/B, ISO 15693, `FeliCa`) are intentionally left to a
//! higher layer or the vendor's RFAL library; the chip is too complex
//! to re-implement those cleanly.
//!
//! ## SPI framing
//!
//! The ST25R3916 uses a two-bit "mode" prefix on the first byte of
//! every transaction:
//!
//! - `00b` — register read (followed by the register address in bits 5..0)
//! - `01b` — register write
//! - `10b` — FIFO read
//! - `11b` — direct command
//!
//! The scaffold exposes register read / write helpers that encode this
//! prefix internally.
//!
//! ## SPI settings
//!
//! Clock polarity `low` (CPOL = 0), clock phase 1st edge (CPHA = 0),
//! MSB-first, 8-bit framing. Up to 6 MHz; the vendor driver defaults to
//! 1.5 MHz.
//!
//! ## Status
//!
//! Scaffold only. [`St25r3916::init`] issues the set-default-command
//! and verifies the chip identity; it does not configure the analog
//! front-end, antenna tuning, or protocol registers.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::{spi::SpiDevice, delay::DelayNs};
//! # async fn demo<S, D>(spi: S, mut delay: D) -> Result<(), st25r3916::Error<S::Error>>
//! # where S: SpiDevice, D: DelayNs {
//! let mut nfc = st25r3916::St25r3916::new(spi);
//! nfc.init(&mut delay).await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::{
    delay::DelayNs,
    spi::{Operation, SpiDevice},
};

/// `IC_IDENTITY` register (space-A, `0x3F`). Low 5 bits encode the IC
/// type; upper 3 bits are the silicon revision.
const REG_IC_IDENTITY: u8 = 0x3F;

/// IC-type field expected on a genuine ST25R3916 (low 5 bits of
/// `IC_IDENTITY`). `0b01010` = ST25R3916.
pub const IC_TYPE_ST25R3916: u8 = 0b0_1010;

/// Mask applied to `IC_IDENTITY` before comparing against
/// [`IC_TYPE_ST25R3916`].
const IC_IDENTITY_TYPE_MASK: u8 = 0b0001_1111;

/// SPI mode prefix bits (upper 2 bits of the first byte).
const MODE_REG_READ: u8 = 0b0100_0000;
/// Register-write mode prefix.
const MODE_REG_WRITE: u8 = 0b0000_0000;
/// Direct-command mode prefix.
const MODE_DIRECT_CMD: u8 = 0b1100_0000;

/// Direct command: `SET_DEFAULT` — reset all registers to datasheet
/// defaults. Opcode `0xC2`.
pub const CMD_SET_DEFAULT: u8 = 0xC2;

/// Post-`SET_DEFAULT` settle delay, in microseconds.
///
/// The datasheet's startup sequence waits "several hundred µs" after
/// `SET_DEFAULT` before issuing further commands; 500 µs is safe.
const SET_DEFAULT_SETTLE_US: u32 = 500;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying SPI bus.
    Spi(E),
    /// `IC_IDENTITY` did not match the expected ST25R3916 IC type.
    /// Contains the raw byte read from the register.
    BadIcIdentity(u8),
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::Spi(e)
    }
}

/// ST25R3916 driver handle.
pub struct St25r3916<S> {
    /// Underlying SPI device (handles CS for us).
    spi: S,
}

impl<S: SpiDevice> St25r3916<S> {
    /// Wrap an SPI device. Does not touch the bus.
    #[must_use]
    pub const fn new(spi: S) -> Self {
        Self { spi }
    }

    /// Read a register in address space A (`0x00..=0x3F`).
    ///
    /// Sends the register-read mode byte with the 6-bit register
    /// address in the low bits; the response byte arrives on the
    /// second beat of the transaction.
    ///
    /// # Errors
    ///
    /// Propagates any SPI bus error.
    pub async fn read_register(&mut self, reg: u8) -> Result<u8, Error<S::Error>> {
        let cmd = [MODE_REG_READ | (reg & 0x3F)];
        let mut rx = [0u8; 1];
        self.spi
            .transaction(&mut [Operation::Write(&cmd), Operation::Read(&mut rx)])
            .await?;
        Ok(rx[0])
    }

    /// Write a register in address space A (`0x00..=0x3F`).
    ///
    /// # Errors
    ///
    /// Propagates any SPI bus error.
    pub async fn write_register(&mut self, reg: u8, value: u8) -> Result<(), Error<S::Error>> {
        self.spi
            .write(&[MODE_REG_WRITE | (reg & 0x3F), value])
            .await?;
        Ok(())
    }

    /// Send a direct command.
    ///
    /// # Errors
    ///
    /// Propagates any SPI bus error.
    pub async fn direct_command(&mut self, opcode: u8) -> Result<(), Error<S::Error>> {
        self.spi.write(&[MODE_DIRECT_CMD | (opcode & 0x3F)]).await?;
        Ok(())
    }

    /// Scaffold init. Issues `SET_DEFAULT`, waits, reads and validates
    /// `IC_IDENTITY`. Does not configure the analog front-end, antenna
    /// tuning, or any protocol registers.
    ///
    /// # Errors
    ///
    /// - [`Error::BadIcIdentity`] if the chip does not report an
    ///   ST25R3916 IC type.
    /// - [`Error::Spi`] on any bus failure.
    pub async fn init<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<S::Error>> {
        self.direct_command(CMD_SET_DEFAULT).await?;
        delay.delay_us(SET_DEFAULT_SETTLE_US).await;

        let id = self.read_register(REG_IC_IDENTITY).await?;
        if id & IC_IDENTITY_TYPE_MASK != IC_TYPE_ST25R3916 {
            return Err(Error::BadIcIdentity(id));
        }
        // TODO: IO config (SPI vs I²C mode pin strap, MISO pull), oscillator
        // startup (`CMD_CLEAR_RXGAIN` + `CMD_ANALOG_PRESET`), AM / PM mode,
        // receiver / transmitter configuration, interrupt mask.
        Ok(())
    }
}
