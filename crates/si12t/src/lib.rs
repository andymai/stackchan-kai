//! # si12t
//!
//! Scaffold for a `no_std` async I²C driver for the `Si12T` three-zone
//! capacitive touch controller. On the M5Stack Stack-chan body the
//! `Si12T` exposes three touch pads (left / centre / right) on the back
//! of the head.
//!
//! ## Status
//!
//! Scaffold only. The vendor datasheet is not publicly available; the
//! driver's register surface, I²C address, chip-ID location, and init
//! sequence must be extracted from M5Stack's reference firmware before
//! this crate can do anything useful.
//!
//! What this scaffold does give you:
//!
//! - The crate shell (Cargo.toml + workspace membership) so the whole
//!   workspace `cargo check`s.
//! - An `Si12t` struct + constructor so the firmware board-init module
//!   can wire the crate into place with a `TODO` marker instead of a
//!   bare stub.
//! - An [`Error`] + `init` shape that matches the other driver crates,
//!   so once the register surface lands the API stays consistent.
//!
//! Fill in [`ADDRESS`], register constants, and the `init()` body from
//! M5Stack's Stack-chan C++ source (`stackchan/main/hal/board/`).
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::{i2c::I2c, delay::DelayNs};
//! # async fn demo<B, D>(bus: B, mut delay: D) -> Result<(), si12t::Error<B::Error>>
//! # where B: I2c, D: DelayNs {
//! let mut touch = si12t::Si12t::new(bus);
//! touch.init(&mut delay).await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::{delay::DelayNs, i2c::I2c};

/// 7-bit I²C address.
///
/// **TODO:** confirm from M5Stack's reference firmware. Placeholder
/// value `0x50` is a guess and will likely change.
pub const ADDRESS: u8 = 0x50;

/// Decoded touch state for the three zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Touch {
    /// `true` when the left zone is being touched.
    pub left: bool,
    /// `true` when the centre zone is being touched.
    pub centre: bool,
    /// `true` when the right zone is being touched.
    pub right: bool,
}

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
    /// Chip did not respond with an expected identity value. The
    /// scaffold does not know what that value is yet; reserved for when
    /// the register surface is filled in.
    BadIdentity,
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// `Si12T` driver handle.
pub struct Si12t<B> {
    /// Underlying I²C bus. Unused until the register surface is in
    /// place; held so the handle can't be constructed without a bus
    /// and the eventual `init()` body has it ready.
    #[allow(
        dead_code,
        reason = "scaffold — bus wakes up once the register map is extracted"
    )]
    bus: B,
    /// Resolved 7-bit I²C address.
    address: u8,
}

impl<B: I2c> Si12t<B> {
    /// Wrap an I²C bus with the default (provisional) [`ADDRESS`].
    #[must_use]
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            address: ADDRESS,
        }
    }

    /// Wrap an I²C bus with a specific address.
    #[must_use]
    pub const fn with_address(bus: B, address: u8) -> Self {
        Self { bus, address }
    }

    /// Resolved 7-bit I²C address. Useful for logging.
    #[must_use]
    pub const fn address(&self) -> u8 {
        self.address
    }

    /// Scaffold init. Returns `Ok(())` without touching the bus.
    ///
    /// Fill in the register / init sequence once the M5Stack reference
    /// is on hand.
    ///
    /// # Errors
    ///
    /// Currently infallible; the signature matches peer driver crates
    /// so the eventual implementation can add `BadIdentity` /
    /// transport errors without an API break.
    #[allow(
        clippy::unused_async,
        reason = "scaffold — async surface matches peer driver crates"
    )]
    pub async fn init<D: DelayNs>(&mut self, _delay: &mut D) -> Result<(), Error<B::Error>> {
        // TODO: reset, chip-ID probe, zone calibration, sensitivity,
        // interrupt enable.
        Ok(())
    }

    /// Placeholder touch read. Returns the default `Touch` (all
    /// zones idle) until a real register map is in place.
    ///
    /// # Errors
    ///
    /// Currently infallible; the signature matches peer driver crates
    /// so the eventual implementation can return transport errors
    /// without an API break.
    #[allow(
        clippy::unused_async,
        reason = "scaffold — async surface matches peer driver crates"
    )]
    pub async fn read_touch(&mut self) -> Result<Touch, Error<B::Error>> {
        // TODO: read the zone-status register, decode each zone bit.
        Ok(Touch::default())
    }
}
