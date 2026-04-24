//! # py32
//!
//! `no_std` async driver for the Stack-chan CoreS3's PY32 IO expander
//! at I²C address `0x6F`.
//!
//! On this board the PY32 is a custom-firmware co-processor — it is *not*
//! a standard GPIO expander part. M5Stack ships it with a small MCU
//! firmware (the Puya PY32 family) that exposes, over a single I²C
//! target, GPIO / pull / drive-mode control plus a WS2812-family fan-out
//! buffer on the pin wired to the 12-LED ring. This crate implements the
//! subset this firmware needs:
//!
//! - Configure one GPIO pin as a push-pull output with pull-up
//!   (read-modify-write, so previously configured pins stay put). This
//!   is how the servo-power rail gate on pin 0 is raised.
//! - Load a pixel frame into the on-chip LED RAM and latch it to the
//!   WS2812 chain on pin 13 (the dedicated LED-fan-out pin). Pixel data
//!   is little-endian RGB565; the PY32 firmware handles all WS2812 bit
//!   timing internally.
//!
//! The PWM / ADC / IRQ / UID surfaces that the M5Stack C++ class also
//! exposes are intentionally omitted — add them when a caller needs them.
//!
//! Register layout + LED-RAM semantics are lifted from
//! `m5stack/StackChan` — `firmware/main/hal/drivers/PY32IOExpander_Class/`
//! (MIT-licensed upstream).
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::i2c::I2c;
//! # async fn demo<B: I2c>(bus: B) -> Result<(), py32::Error<B::Error>> {
//! let mut py = py32::Py32::new(bus);
//! // Raise the servo-power rail (pin 0 = push-pull, pull-up, HIGH).
//! py.configure_output_pin(0, true).await?;
//! // Drive the 12-LED ring with a single warm-white frame.
//! py.set_led_count(12).await?;
//! let frame: [u16; 12] = [0xFFE0; 12]; // RGB565: yellowish white.
//! py.write_led_pixels(&frame).await?;
//! py.refresh_leds().await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::i2c::I2c;

/// 7-bit I²C address of the PY32 on the Stack-chan CoreS3.
pub const ADDRESS: u8 = 0x6F;

/// Maximum number of WS2812 pixels the on-chip LED RAM can hold.
/// The LED-count field in the LED-config register is 6 bits wide.
pub const MAX_LEDS: usize = 32;

/// Highest GPIO pin index exposed by the PY32 firmware (pins `0..=13`).
const PIN_MAX: u8 = 13;

// --- Register map (from m5stack/StackChan PY32IOExpander_Class.cpp) ---

/// GPIO direction register, low byte (pins `0..=7`). Bit set = output.
const REG_DIR_LO: u8 = 0x03;
/// GPIO direction register, high byte (pins `8..=13`).
const REG_DIR_HI: u8 = 0x04;
/// GPIO output-level register, low byte (pins `0..=7`). Bit set = HIGH.
const REG_OUT_LO: u8 = 0x05;
/// GPIO output-level register, high byte (pins `8..=13`).
const REG_OUT_HI: u8 = 0x06;
/// Pull-up enable register, low byte (pins `0..=7`). Bit set = enabled.
const REG_PULLUP_LO: u8 = 0x09;
/// Pull-up enable register, high byte (pins `8..=13`).
const REG_PULLUP_HI: u8 = 0x0A;
/// LED configuration. Bits `0..=5` hold the live LED count (max 32).
/// Bit 6 is a refresh-trigger: the PY32 latches the LED RAM contents
/// onto the WS2812 chain when bit 6 is set.
const REG_LED_CFG: u8 = 0x24;
/// First byte of LED RAM. Each pixel occupies 2 bytes (LE RGB565), so
/// pixel `i` lives at `REG_LED_RAM + 2 * i`. Bulk writes auto-increment.
const REG_LED_RAM: u8 = 0x30;

/// LED count field mask within [`REG_LED_CFG`].
const LED_COUNT_MASK: u8 = 0x3F;
/// Refresh-trigger bit within [`REG_LED_CFG`].
const LED_REFRESH_BIT: u8 = 1 << 6;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
    /// Caller passed a pin index outside the `0..=13` range the PY32
    /// firmware exposes.
    InvalidPin(u8),
    /// Caller passed more than [`MAX_LEDS`] pixels.
    TooManyLeds(usize),
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// PY32 IO-expander driver.
///
/// Holds the I²C bus handle and issues register reads / writes to
/// [`ADDRESS`]. The driver is state-free: every public call that touches
/// GPIO registers does a read-modify-write, so interleaving calls on
/// different pins is safe.
pub struct Py32<B> {
    /// Async I²C bus handle used for every transaction.
    bus: B,
}

impl<B: I2c> Py32<B> {
    /// Construct a driver over an async I²C bus.
    #[must_use = "holds the I²C bus; drop it to release"]
    pub const fn new(bus: B) -> Self {
        Self { bus }
    }

    /// Consume the driver and return the underlying bus.
    #[must_use = "dropping the bus drops any pending transactions"]
    pub fn release(self) -> B {
        self.bus
    }

    /// Configure `pin` as a push-pull output with pull-up enabled, and
    /// drive it to `level`.
    ///
    /// Uses read-modify-write on each affected register so previously
    /// configured pins keep their state. The PY32 firmware's reset
    /// default is all-zeros for every GPIO register, so the very first
    /// call on a fresh chip degrades to three plain register writes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidPin`] if `pin > 13`, or [`Error::I2c`]
    /// if any of the six I²C transactions fails.
    pub async fn configure_output_pin(
        &mut self,
        pin: u8,
        level: bool,
    ) -> Result<(), Error<B::Error>> {
        if pin > PIN_MAX {
            return Err(Error::InvalidPin(pin));
        }
        let regs = pin_regs(pin);
        self.set_bit(regs.dir, regs.mask).await?;
        self.set_bit(regs.pullup, regs.mask).await?;
        if level {
            self.set_bit(regs.out, regs.mask).await
        } else {
            self.clear_bit(regs.out, regs.mask).await
        }
    }

    /// Set the active LED count (0..=32) on the WS2812 fan-out.
    ///
    /// Preserves the refresh-trigger bit so an in-flight `refresh_leds`
    /// isn't cleared by a reconfiguration.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TooManyLeds`] if `count > 32`, or [`Error::I2c`]
    /// on transport failure.
    pub async fn set_led_count(&mut self, count: u8) -> Result<(), Error<B::Error>> {
        if usize::from(count) > MAX_LEDS {
            return Err(Error::TooManyLeds(usize::from(count)));
        }
        let current = self.read_reg(REG_LED_CFG).await?;
        let new = (current & !LED_COUNT_MASK) | (count & LED_COUNT_MASK);
        self.write_reg(REG_LED_CFG, new).await
    }

    /// Bulk-write `pixels` into the on-chip LED RAM.
    ///
    /// Each pixel is an RGB565 value; the wire format is little-endian
    /// (low byte first) to match the PY32 firmware. The write does
    /// **not** update the WS2812 chain — call [`Py32::refresh_leds`]
    /// after staging the frame to latch it. That two-step write-then-
    /// refresh sequence is what prevents torn frames on the ring during
    /// a bulk update.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TooManyLeds`] if `pixels.len() > 32`, or
    /// [`Error::I2c`] on transport failure.
    pub async fn write_led_pixels(&mut self, pixels: &[u16]) -> Result<(), Error<B::Error>> {
        if pixels.len() > MAX_LEDS {
            return Err(Error::TooManyLeds(pixels.len()));
        }
        if pixels.is_empty() {
            // Zero-pixel frame = no-op; skip the wire write entirely so
            // an empty update doesn't poke the I²C bus at all.
            return Ok(());
        }
        // Stage [REG_LED_RAM, b0, b1, b0, b1, ...] on the stack. Max
        // payload is 1 + 32*2 = 65 bytes, well under any reasonable
        // I²C FIFO limit; single transaction keeps the write atomic
        // from the host's perspective.
        let mut buf = [0u8; 1 + MAX_LEDS * 2];
        buf[0] = REG_LED_RAM;
        for (i, &px) in pixels.iter().enumerate() {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "masked to 8 bits before truncation"
            )]
            {
                buf[1 + i * 2] = (px & 0xFF) as u8;
                buf[2 + i * 2] = ((px >> 8) & 0xFF) as u8;
            }
        }
        let len = 1 + pixels.len() * 2;
        self.bus.write(ADDRESS, &buf[..len]).await?;
        Ok(())
    }

    /// Latch the buffered LED RAM onto the WS2812 chain.
    ///
    /// Sets the refresh-trigger bit in the LED-config register via
    /// read-modify-write so the LED count field isn't disturbed. The
    /// PY32 firmware clears the bit itself once the latch completes.
    ///
    /// # Errors
    ///
    /// [`Error::I2c`] on transport failure.
    pub async fn refresh_leds(&mut self) -> Result<(), Error<B::Error>> {
        self.set_bit(REG_LED_CFG, LED_REFRESH_BIT).await
    }

    /// Read a single byte from register `reg`.
    async fn read_reg(&mut self, reg: u8) -> Result<u8, Error<B::Error>> {
        let mut buf = [0u8; 1];
        self.bus.write_read(ADDRESS, &[reg], &mut buf).await?;
        Ok(buf[0])
    }

    /// Write `value` to register `reg`.
    async fn write_reg(&mut self, reg: u8, value: u8) -> Result<(), Error<B::Error>> {
        self.bus.write(ADDRESS, &[reg, value]).await?;
        Ok(())
    }

    /// Set the bits in `mask` within register `reg` (read-modify-write).
    async fn set_bit(&mut self, reg: u8, mask: u8) -> Result<(), Error<B::Error>> {
        let v = self.read_reg(reg).await?;
        self.write_reg(reg, v | mask).await
    }

    /// Clear the bits in `mask` within register `reg` (read-modify-write).
    async fn clear_bit(&mut self, reg: u8, mask: u8) -> Result<(), Error<B::Error>> {
        let v = self.read_reg(reg).await?;
        self.write_reg(reg, v & !mask).await
    }
}

/// Register triplet + pin-bit mask for a given pin index.
struct PinRegs {
    /// Direction register for the port this pin lives on.
    dir: u8,
    /// Output-level register for the port this pin lives on.
    out: u8,
    /// Pull-up enable register for the port this pin lives on.
    pullup: u8,
    /// Bitmask selecting this pin within its port byte.
    mask: u8,
}

/// Select the correct register triplet and pin-bit mask for pin `0..=13`.
const fn pin_regs(pin: u8) -> PinRegs {
    if pin < 8 {
        PinRegs {
            dir: REG_DIR_LO,
            out: REG_OUT_LO,
            pullup: REG_PULLUP_LO,
            mask: 1 << pin,
        }
    } else {
        PinRegs {
            dir: REG_DIR_HI,
            out: REG_OUT_HI,
            pullup: REG_PULLUP_HI,
            mask: 1 << (pin - 8),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test scaffolding: Infallible bus error makes unwrap() sound"
)]
#[allow(
    clippy::expect_used,
    reason = "mock I²C expects matching Write-then-Read ops; a bare Read is a test-harness bug, not runtime code"
)]
#[allow(
    clippy::decimal_bitwise_operands,
    reason = "LED count literals (3, 12) read more clearly in base-10 than as 0x03/0x0C in tests"
)]
#[allow(
    clippy::future_not_send,
    reason = "test mocks hold RefCell for event recording; single-threaded block_on runs them"
)]
mod tests {
    use super::*;
    use core::cell::RefCell;
    use embedded_hal_async::i2c::{Operation, SevenBitAddress};

    /// Event recorded by the mock. Reads and writes are kept distinct so
    /// tests can assert the full transaction order (critical for
    /// read-modify-write correctness).
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Event {
        /// I²C write transaction; full payload after the register byte.
        /// Address is implicit (always [`ADDRESS`]).
        Write(Vec<u8>),
        /// I²C write-read: driver wrote `reg`, got back `value`.
        ReadReg(u8, u8),
    }

    /// Mock that simulates a 256-byte register bank. Reads serve the
    /// bank; single-byte-payload `write` transactions update it so
    /// read-modify-write loops converge naturally. Multi-byte payloads
    /// (bulk LED writes) update the bank too, auto-incrementing the
    /// register index.
    struct MockI2c {
        regs: RefCell<[u8; 256]>,
        events: RefCell<Vec<Event>>,
    }

    impl MockI2c {
        fn new() -> Self {
            Self {
                regs: RefCell::new([0u8; 256]),
                events: RefCell::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<Event> {
            self.events.borrow().clone()
        }

        fn preset_reg(&self, reg: u8, value: u8) {
            self.regs.borrow_mut()[usize::from(reg)] = value;
        }
    }

    impl embedded_hal_async::i2c::ErrorType for MockI2c {
        type Error = core::convert::Infallible;
    }

    impl embedded_hal_async::i2c::I2c for MockI2c {
        async fn transaction(
            &mut self,
            address: SevenBitAddress,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Self::Error> {
            assert_eq!(address, ADDRESS, "driver addressed wrong I²C target");

            // Track the "current" register index across ops in one
            // transaction so a Write([reg]) followed by Read(buf) pulls
            // `buf.len()` bytes starting at `reg`.
            let mut cursor: Option<u8> = None;

            for op in operations {
                match op {
                    Operation::Write(buf) => {
                        if buf.is_empty() {
                            continue;
                        }
                        let reg = buf[0];
                        cursor = Some(reg);
                        if buf.len() >= 2 {
                            // Data write. Auto-increment the reg pointer
                            // for each payload byte to mirror the PY32
                            // firmware's bulk-LED-RAM behaviour.
                            let mut regs = self.regs.borrow_mut();
                            for (i, &b) in buf[1..].iter().enumerate() {
                                let idx = usize::from(reg).saturating_add(i);
                                if idx < regs.len() {
                                    regs[idx] = b;
                                }
                            }
                            drop(regs);
                            self.events.borrow_mut().push(Event::Write(buf.to_vec()));
                        }
                        // A lone `Write([reg])` with no data is the
                        // address-phase of a write-read; emit nothing —
                        // the Read op that follows will log ReadReg.
                    }
                    Operation::Read(buf) => {
                        let reg = cursor.expect("read without prior register-address write");
                        let regs = self.regs.borrow();
                        for (i, slot) in buf.iter_mut().enumerate() {
                            let idx = usize::from(reg).saturating_add(i);
                            *slot = if idx < regs.len() { regs[idx] } else { 0 };
                        }
                        // Log only single-byte reads; bulk reads aren't
                        // exercised by this driver.
                        if buf.len() == 1 {
                            self.events.borrow_mut().push(Event::ReadReg(reg, buf[0]));
                        }
                    }
                }
            }
            Ok(())
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
    fn configure_output_pin_low_byte_from_reset_state() {
        // Pin 0: low-byte registers, bit 0. Registers reset to zero, so
        // read-modify-write writes back `0x01` each time — matching the
        // inline servo-power sequence this driver replaces.
        let mut py = Py32::new(MockI2c::new());
        block_on(py.configure_output_pin(0, true)).unwrap();

        assert_eq!(
            py.bus.events(),
            vec![
                Event::ReadReg(REG_DIR_LO, 0x00),
                Event::Write(vec![REG_DIR_LO, 0x01]),
                Event::ReadReg(REG_PULLUP_LO, 0x00),
                Event::Write(vec![REG_PULLUP_LO, 0x01]),
                Event::ReadReg(REG_OUT_LO, 0x00),
                Event::Write(vec![REG_OUT_LO, 0x01]),
            ]
        );
    }

    #[test]
    fn configure_output_pin_preserves_other_bits() {
        // If another pin on the same port is already configured, RMW
        // must OR rather than overwrite. Simulate: pin 1 already set as
        // output (dir=0x02, pullup=0x02, level=0x02), then configure
        // pin 0 HIGH. Resulting bytes should have bits 0 AND 1 set.
        let bus = MockI2c::new();
        bus.preset_reg(REG_DIR_LO, 0x02);
        bus.preset_reg(REG_PULLUP_LO, 0x02);
        bus.preset_reg(REG_OUT_LO, 0x02);
        let mut py = Py32::new(bus);
        block_on(py.configure_output_pin(0, true)).unwrap();

        let events = py.bus.events();
        // Each write should OR in bit 0 without touching bit 1.
        assert!(events.contains(&Event::Write(vec![REG_DIR_LO, 0x03])));
        assert!(events.contains(&Event::Write(vec![REG_PULLUP_LO, 0x03])));
        assert!(events.contains(&Event::Write(vec![REG_OUT_LO, 0x03])));
    }

    #[test]
    fn configure_output_pin_high_byte_uses_correct_registers() {
        // Pin 13 lives in the high-byte register trio and uses mask
        // 1 << (13 - 8) = 0x20.
        let mut py = Py32::new(MockI2c::new());
        block_on(py.configure_output_pin(13, true)).unwrap();

        let events = py.bus.events();
        assert!(events.contains(&Event::Write(vec![REG_DIR_HI, 0x20])));
        assert!(events.contains(&Event::Write(vec![REG_PULLUP_HI, 0x20])));
        assert!(events.contains(&Event::Write(vec![REG_OUT_HI, 0x20])));
        // And must NOT have touched the low-byte registers.
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::Write(b) if b[0] == REG_DIR_LO || b[0] == REG_OUT_LO))
        );
    }

    #[test]
    fn configure_output_pin_low_level_clears_bit() {
        let bus = MockI2c::new();
        bus.preset_reg(REG_OUT_LO, 0xFF);
        let mut py = Py32::new(bus);
        block_on(py.configure_output_pin(3, false)).unwrap();

        // Bit 3 cleared, bits 0,1,2,4..7 preserved -> 0xF7.
        assert!(
            py.bus
                .events()
                .contains(&Event::Write(vec![REG_OUT_LO, 0xF7]))
        );
    }

    #[test]
    fn configure_output_pin_rejects_out_of_range() {
        let mut py = Py32::new(MockI2c::new());
        let result = block_on(py.configure_output_pin(14, true));
        assert!(matches!(result, Err(Error::InvalidPin(14))));
        assert!(py.bus.events().is_empty(), "no I²C traffic on error");
    }

    #[test]
    fn set_led_count_writes_count_and_preserves_refresh_bit() {
        let bus = MockI2c::new();
        // Simulate a stale refresh bit (unlikely on real hardware since
        // the PY32 clears it, but verifies the RMW contract).
        bus.preset_reg(REG_LED_CFG, LED_REFRESH_BIT);
        let mut py = Py32::new(bus);
        block_on(py.set_led_count(12)).unwrap();

        assert!(
            py.bus
                .events()
                .contains(&Event::Write(vec![REG_LED_CFG, LED_REFRESH_BIT | 12]))
        );
    }

    #[test]
    fn set_led_count_rejects_over_max() {
        let mut py = Py32::new(MockI2c::new());
        let result = block_on(py.set_led_count(33));
        assert!(matches!(result, Err(Error::TooManyLeds(33))));
    }

    #[test]
    fn write_led_pixels_emits_one_bulk_le_rgb565_transaction() {
        let mut py = Py32::new(MockI2c::new());
        // Two pixels: amber-ish, cyan-ish.
        let pixels = [0x1234u16, 0xABCDu16];
        block_on(py.write_led_pixels(&pixels)).unwrap();

        // Wire format: [REG_LED_RAM, lo0, hi0, lo1, hi1]
        assert_eq!(
            py.bus.events(),
            vec![Event::Write(vec![REG_LED_RAM, 0x34, 0x12, 0xCD, 0xAB])]
        );
    }

    #[test]
    fn write_led_pixels_rejects_over_max() {
        let mut py = Py32::new(MockI2c::new());
        let pixels = [0u16; MAX_LEDS + 1];
        let result = block_on(py.write_led_pixels(&pixels));
        assert!(matches!(result, Err(Error::TooManyLeds(n)) if n == MAX_LEDS + 1));
    }

    #[test]
    fn write_led_pixels_empty_slice_is_noop() {
        let mut py = Py32::new(MockI2c::new());
        block_on(py.write_led_pixels(&[])).unwrap();
        assert!(
            py.bus.events().is_empty(),
            "empty pixel slice should not touch the bus"
        );
    }

    #[test]
    fn refresh_leds_sets_bit_6_and_preserves_count() {
        let bus = MockI2c::new();
        bus.preset_reg(REG_LED_CFG, 12); // count = 12, refresh = 0
        let mut py = Py32::new(bus);
        block_on(py.refresh_leds()).unwrap();

        assert!(
            py.bus
                .events()
                .contains(&Event::Write(vec![REG_LED_CFG, LED_REFRESH_BIT | 12]))
        );
    }

    #[test]
    fn full_led_frame_sequence_matches_m5stack_protocol() {
        // End-to-end golden: set_led_count -> bulk write -> refresh.
        // This is the exact per-frame traffic the firmware will emit.
        let mut py = Py32::new(MockI2c::new());
        let pixels: [u16; 3] = [0x001F, 0x07E0, 0xF800]; // R, G, B in 565.
        block_on(async {
            py.set_led_count(3).await.unwrap();
            py.write_led_pixels(&pixels).await.unwrap();
            py.refresh_leds().await.unwrap();
        });

        assert_eq!(
            py.bus.events(),
            vec![
                // set_led_count: RMW 0x24 from 0x00 to 0x03
                Event::ReadReg(REG_LED_CFG, 0x00),
                Event::Write(vec![REG_LED_CFG, 0x03]),
                // bulk write: 3 pixels, LE RGB565
                Event::Write(vec![REG_LED_RAM, 0x1F, 0x00, 0xE0, 0x07, 0x00, 0xF8]),
                // refresh: RMW 0x24 to set bit 6 (count field stays 3)
                Event::ReadReg(REG_LED_CFG, 0x03),
                Event::Write(vec![REG_LED_CFG, LED_REFRESH_BIT | 0x03]),
            ]
        );
    }
}
