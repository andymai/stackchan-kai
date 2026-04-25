//! # si12t
//!
//! `no_std` async I²C driver for the `Si12T` three-zone capacitive
//! touch controller. On the M5Stack Stack-chan body the chip exposes
//! three pads (left / centre / right) on the back of the head and the
//! host polls them at ~50 ms cadence — there is no interrupt line.
//!
//! ## Source
//!
//! The chip's datasheet is proprietary; this driver mirrors the
//! upstream M5Stack reference C++ implementation:
//!
//! - <https://github.com/m5stack/StackChan/blob/main/firmware/main/hal/drivers/Si12T/Si12T.h>
//! - <https://github.com/m5stack/StackChan/blob/main/firmware/main/hal/drivers/Si12T/Si12T.cpp>
//! - <https://github.com/m5stack/StackChan/blob/main/firmware/main/hal/hal_head_touch.cpp>
//!
//! ## Address gotcha
//!
//! Upstream's `SI12T_GND_ADDRESS = 0x68` macro does **not** match the
//! address the chip ACKs at on Andy's CoreS3 + Stack-chan body unit
//! (verified `0x50` via `just i2c-probe`). The provisional `0x50` in
//! [`ADDRESS`] matches the bus probe; the upstream macro is presumably
//! a different variant or strap option. Override via
//! [`Si12t::with_address`] if your unit straps differently.
//!
//! ## Usage
//!
//! ```no_run
//! # use embedded_hal_async::{i2c::I2c, delay::DelayNs};
//! # async fn demo<B, D>(bus: B, mut delay: D) -> Result<(), si12t::Error<B::Error>>
//! # where B: I2c, D: DelayNs {
//! let mut touch = si12t::Si12t::new(bus);
//! touch.init(&mut delay).await?;
//! loop {
//!     let zones = touch.read_touch().await?;
//!     if zones.left() { /* … */ }
//! #   break;
//! }
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::{delay::DelayNs, i2c::I2c};

/// Default 7-bit I²C address.
///
/// Matches the upstream M5Stack reference `SI12T_GND_ADDRESS = 0x68`.
/// Note that BMI270 also defaults to 0x68 — on the M5Stack body the
/// IMU is strapped to `0x69` (SDO high) so the two don't collide.
pub const ADDRESS: u8 = 0x68;

/// Sensitivity register — channel 1.
const REG_SENS1: u8 = 0x02;
/// Sensitivity register — channel 5 (last one written by upstream init;
/// SENS6 at 0x07 is intentionally left at reset).
const REG_SENS5: u8 = 0x06;
/// Control register 1 — operating mode + FTC.
const REG_CTRL1: u8 = 0x08;
/// Control register 2 — reset / sleep gate.
const REG_CTRL2: u8 = 0x09;
/// Reference-reset register 1 (start of the 6-register zero-init block).
const REG_REF_RST1: u8 = 0x0A;
/// Calibration-hold register 2 (end of the zero-init block; 6 regs total).
const REG_CAL_HOLD2: u8 = 0x0F;
/// Output register 1 — single byte holding all three zones, 2 bits each.
const REG_OUTPUT1: u8 = 0x10;

/// CTRL1 value used by the M5Stack reference: auto-mode + FTC=01.
const CTRL1_AUTO_MODE: u8 = 0x22;
/// CTRL2 reset pulse (write before [`CTRL2_SLEEP_DISABLE`]).
const CTRL2_RESET: u8 = 0x0F;
/// CTRL2 normal-operation value (sleep-disable, post-reset).
const CTRL2_SLEEP_DISABLE: u8 = 0x07;

/// Default sensitivity byte: `TYPE_LOW + LEVEL_3` per the upstream
/// reference. Lower nibble = level (0..7); upper nibble adds `0x80`
/// for `TYPE_HIGH`. Stack-chan's body ships tuned for this value.
pub const DEFAULT_SENSITIVITY: u8 = 0x33;

/// Settling delay between the CTRL2 reset pulse and the
/// sleep-disable write, in milliseconds. Upstream's blocking
/// implementation has no explicit delay here; on async transports a
/// small guard avoids transient NACKs.
const CTRL2_SETTLE_MS: u32 = 1;

/// Per-zone touch intensity, packed into 2 bits in the output register.
///
/// `None` is a true "no touch"; the higher levels are a rough
/// proximity gradient that upstream's UI threshold treats as "touched"
/// if non-zero. Not `#[non_exhaustive]` — the encoding is a fixed
/// 2-bit field so there will never be more variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Intensity {
    /// No touch detected.
    #[default]
    None,
    /// Lightest touch (intensity 1).
    Low,
    /// Medium touch (intensity 2).
    Mid,
    /// Strongest touch (intensity 3).
    High,
}

impl Intensity {
    /// Decode the 2-bit field from the packed output byte.
    const fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0 => Self::None,
            1 => Self::Low,
            2 => Self::Mid,
            _ => Self::High,
        }
    }

    /// `true` if any non-zero intensity is present. Mirrors upstream's
    /// `intensity >= 1` UI threshold.
    #[must_use]
    pub const fn is_touched(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Decoded touch state for the three zones. `bool` accessors collapse
/// the 4-level intensity to "any touch"; reach for [`Self::intensity`]
/// when the gradient matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Touch {
    /// Raw per-zone intensities, in `(left, centre, right)` order.
    pub intensity: (Intensity, Intensity, Intensity),
}

impl Touch {
    /// Decode a 3-zone touch report from the packed `OUTPUT1` byte.
    ///
    /// Layout: bits `0..1` = left, `2..3` = centre, `4..5` = right.
    /// Upper bits are reserved.
    #[must_use]
    pub const fn from_output_byte(byte: u8) -> Self {
        Self {
            intensity: (
                Intensity::from_bits(byte),
                Intensity::from_bits(byte >> 2),
                Intensity::from_bits(byte >> 4),
            ),
        }
    }

    /// `true` if the left pad is being touched.
    #[must_use]
    pub const fn left(&self) -> bool {
        self.intensity.0.is_touched()
    }

    /// `true` if the centre pad is being touched.
    #[must_use]
    pub const fn centre(&self) -> bool {
        self.intensity.1.is_touched()
    }

    /// `true` if the right pad is being touched.
    #[must_use]
    pub const fn right(&self) -> bool {
        self.intensity.2.is_touched()
    }
}

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

/// Driver handle. Owns the I²C bus reference + a configurable
/// sensitivity byte applied at [`Self::init`].
pub struct Si12t<B> {
    /// Underlying I²C bus.
    bus: B,
    /// Resolved 7-bit I²C address.
    address: u8,
    /// Sensitivity byte written to SENS1..SENS5 during init. Defaults
    /// to [`DEFAULT_SENSITIVITY`].
    sensitivity: u8,
}

impl<B: I2c> Si12t<B> {
    /// Wrap an I²C bus with the verified default [`ADDRESS`] and
    /// [`DEFAULT_SENSITIVITY`].
    #[must_use]
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            address: ADDRESS,
            sensitivity: DEFAULT_SENSITIVITY,
        }
    }

    /// Wrap an I²C bus with a custom address (use if your body uses a
    /// different `ADD_SEL` strap).
    #[must_use]
    pub const fn with_address(bus: B, address: u8) -> Self {
        Self {
            bus,
            address,
            sensitivity: DEFAULT_SENSITIVITY,
        }
    }

    /// Override the sensitivity byte applied at [`Self::init`].
    /// See [`DEFAULT_SENSITIVITY`] for the encoding.
    #[must_use]
    pub const fn with_sensitivity(mut self, sensitivity: u8) -> Self {
        self.sensitivity = sensitivity;
        self
    }

    /// Resolved 7-bit I²C address. Useful for logging.
    #[must_use]
    pub const fn address(&self) -> u8 {
        self.address
    }

    /// Run the chip's init sequence — zero out the six reference /
    /// hold registers, pulse CTRL2 reset, set CTRL1 to auto-mode, and
    /// write the sensitivity byte to SENS1..SENS5.
    ///
    /// Mirrors `si12t_setup()` from the M5Stack reference firmware.
    ///
    /// # Errors
    ///
    /// Returns [`Error::I2c`] on any bus failure.
    pub async fn init<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<B::Error>> {
        // Zero-init the 6-register reference / hold block in one
        // burst so the bus locks the slave once instead of six times.
        // Layout: [reg, REF_RST1, REF_RST2, CH_HOLD1, CH_HOLD2,
        //          CAL_HOLD1, CAL_HOLD2].
        let zero_block = [REG_REF_RST1, 0, 0, 0, 0, 0, 0];
        debug_assert_eq!(
            zero_block.len(),
            1 + (REG_CAL_HOLD2 - REG_REF_RST1 + 1) as usize
        );
        self.bus.write(self.address, &zero_block).await?;

        // CTRL2 reset pulse, settle, then sleep-disable.
        self.write_reg(REG_CTRL2, CTRL2_RESET).await?;
        delay.delay_ms(CTRL2_SETTLE_MS).await;
        self.write_reg(REG_CTRL2, CTRL2_SLEEP_DISABLE).await?;

        // CTRL1 = auto-mode + FTC.
        self.write_reg(REG_CTRL1, CTRL1_AUTO_MODE).await?;

        // Sensitivity to SENS1..SENS5 in one burst (5 channels;
        // upstream intentionally leaves SENS6 at reset).
        let sens_block = [
            REG_SENS1,
            self.sensitivity,
            self.sensitivity,
            self.sensitivity,
            self.sensitivity,
            self.sensitivity,
        ];
        debug_assert_eq!(sens_block.len(), 1 + (REG_SENS5 - REG_SENS1 + 1) as usize);
        self.bus.write(self.address, &sens_block).await?;

        Ok(())
    }

    /// Read the current touch state from `OUTPUT1`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::I2c`] on any bus failure.
    pub async fn read_touch(&mut self) -> Result<Touch, Error<B::Error>> {
        let mut buf = [0u8; 1];
        self.bus
            .write_read(self.address, &[REG_OUTPUT1], &mut buf)
            .await?;
        Ok(Touch::from_output_byte(buf[0]))
    }

    /// Write a single 8-bit register. Helper used by [`Self::init`].
    async fn write_reg(&mut self, reg: u8, value: u8) -> Result<(), Error<B::Error>> {
        self.bus.write(self.address, &[reg, value]).await?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test scaffolding: Infallible bus error makes unwrap() sound"
)]
#[allow(
    clippy::future_not_send,
    reason = "test mocks hold RefCell for event recording; single-threaded block_on runs them"
)]
#[allow(
    clippy::panic,
    reason = "mock harness panics on unexpected I²C operation patterns — matches the aw9523 / ltr553 test pattern"
)]
mod tests {
    use super::*;
    use core::cell::RefCell;
    use embedded_hal_async::i2c::{Operation, SevenBitAddress};

    /// Test-mock event recording I²C writes / reads + delays in order.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Event {
        /// `write(addr, payload)`.
        Write(u8, Vec<u8>),
        /// `write_read(addr, write_payload, read_len)`.
        WriteRead(u8, Vec<u8>, usize),
        /// `delay_ms(n)`.
        DelayMs(u32),
    }

    struct Harness {
        events: RefCell<Vec<Event>>,
        /// Bytes that the next `write_read` should return. Pop-front.
        read_queue: RefCell<Vec<u8>>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                events: RefCell::new(Vec::new()),
                read_queue: RefCell::new(Vec::new()),
            }
        }

        fn queue_read(&self, byte: u8) {
            self.read_queue.borrow_mut().push(byte);
        }

        fn events(&self) -> Vec<Event> {
            self.events.borrow().clone()
        }
    }

    struct MockI2c<'a> {
        harness: &'a Harness,
    }

    impl embedded_hal_async::i2c::ErrorType for MockI2c<'_> {
        type Error = core::convert::Infallible;
    }

    impl embedded_hal_async::i2c::I2c for MockI2c<'_> {
        async fn transaction(
            &mut self,
            address: SevenBitAddress,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Self::Error> {
            // The driver only uses `write` and `write_read`, which
            // map to one or two `Operation` entries respectively.
            // Detect both shapes; ignore other patterns we don't emit.
            let mut iter = operations.iter_mut();
            match (iter.next(), iter.next(), iter.next()) {
                (Some(Operation::Write(buf)), None, _) => {
                    self.harness
                        .events
                        .borrow_mut()
                        .push(Event::Write(address, buf.to_vec()));
                }
                (Some(Operation::Write(wbuf)), Some(Operation::Read(rbuf)), None) => {
                    let len = rbuf.len();
                    self.harness.events.borrow_mut().push(Event::WriteRead(
                        address,
                        wbuf.to_vec(),
                        len,
                    ));
                    let mut q = self.harness.read_queue.borrow_mut();
                    for slot in rbuf.iter_mut() {
                        *slot = q.remove(0);
                    }
                }
                _ => panic!("driver issued an unexpected I²C operation pattern"),
            }
            Ok(())
        }
    }

    struct MockDelay<'a> {
        harness: &'a Harness,
    }

    impl embedded_hal_async::delay::DelayNs for MockDelay<'_> {
        async fn delay_ns(&mut self, ns: u32) {
            self.harness
                .events
                .borrow_mut()
                .push(Event::DelayMs(ns / 1_000_000));
        }
    }

    fn block_on<F: core::future::Future>(future: F) -> F::Output {
        use core::pin::pin;
        use core::task::{Context, Poll, Waker};
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(future);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    #[test]
    fn intensity_decodes_two_bit_field() {
        assert_eq!(Intensity::from_bits(0b00), Intensity::None);
        assert_eq!(Intensity::from_bits(0b01), Intensity::Low);
        assert_eq!(Intensity::from_bits(0b10), Intensity::Mid);
        assert_eq!(Intensity::from_bits(0b11), Intensity::High);
        // Upper bits ignored.
        assert_eq!(Intensity::from_bits(0xFC), Intensity::None);
    }

    #[test]
    fn touch_unpacks_three_zones_from_output_byte() {
        // left=High(3), centre=None(0), right=Mid(2) → 0b10_00_11 = 0x23
        let t = Touch::from_output_byte(0b10_00_11);
        assert_eq!(t.intensity.0, Intensity::High);
        assert_eq!(t.intensity.1, Intensity::None);
        assert_eq!(t.intensity.2, Intensity::Mid);
        assert!(t.left());
        assert!(!t.centre());
        assert!(t.right());
    }

    #[test]
    fn default_touch_has_no_zones_active() {
        let t = Touch::default();
        assert!(!t.left() && !t.centre() && !t.right());
    }

    #[test]
    fn init_emits_upstream_register_sequence() {
        let harness = Harness::new();
        let mut bus = MockI2c { harness: &harness };
        let mut delay = MockDelay { harness: &harness };
        let mut chip = Si12t::new(&mut bus);
        block_on(chip.init(&mut delay)).unwrap();
        let events = harness.events();
        let expected = vec![
            // 6-register zero block: reg + 6 zeros.
            Event::Write(ADDRESS, vec![REG_REF_RST1, 0, 0, 0, 0, 0, 0]),
            // CTRL2 reset, settle, sleep-disable.
            Event::Write(ADDRESS, vec![REG_CTRL2, CTRL2_RESET]),
            Event::DelayMs(CTRL2_SETTLE_MS),
            Event::Write(ADDRESS, vec![REG_CTRL2, CTRL2_SLEEP_DISABLE]),
            // CTRL1 auto-mode.
            Event::Write(ADDRESS, vec![REG_CTRL1, CTRL1_AUTO_MODE]),
            // Sensitivity burst: reg + 5 bytes.
            Event::Write(
                ADDRESS,
                vec![
                    REG_SENS1,
                    DEFAULT_SENSITIVITY,
                    DEFAULT_SENSITIVITY,
                    DEFAULT_SENSITIVITY,
                    DEFAULT_SENSITIVITY,
                    DEFAULT_SENSITIVITY,
                ],
            ),
        ];
        assert_eq!(events, expected);
    }

    #[test]
    fn read_touch_issues_write_read_against_output1() {
        let harness = Harness::new();
        // centre+right touched at MID intensity: 0b10_10_00 = 0x28
        harness.queue_read(0b10_10_00);
        let mut bus = MockI2c { harness: &harness };
        let mut chip = Si12t::new(&mut bus);
        let touch = block_on(chip.read_touch()).unwrap();
        assert_eq!(touch.intensity.1, Intensity::Mid);
        assert_eq!(touch.intensity.2, Intensity::Mid);
        assert!(touch.centre() && touch.right());
        assert!(!touch.left());
        assert_eq!(
            harness.events(),
            vec![Event::WriteRead(ADDRESS, vec![REG_OUTPUT1], 1)],
        );
    }

    #[test]
    fn with_sensitivity_overrides_default_in_init_sequence() {
        let harness = Harness::new();
        let mut bus = MockI2c { harness: &harness };
        let mut delay = MockDelay { harness: &harness };
        let mut chip = Si12t::new(&mut bus).with_sensitivity(0x77);
        block_on(chip.init(&mut delay)).unwrap();
        let last = harness.events().pop().unwrap();
        assert_eq!(
            last,
            Event::Write(ADDRESS, vec![REG_SENS1, 0x77, 0x77, 0x77, 0x77, 0x77]),
        );
    }
}
