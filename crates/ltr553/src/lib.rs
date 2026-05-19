//! # ltr553
//!
//! `no_std` async driver for the Lite-On LTR-553ALS ambient-light +
//! proximity sensor over I²C. Keeps the register surface minimal: one
//! init call, then two polling accessors (`read_ambient`,
//! `read_proximity`). Configures the chip for default gain /
//! integration so the lux math matches Lite-On's app-note piecewise
//! formula at unit scale.
//!
//! ## Addressing
//!
//! The LTR-553 lives at fixed I²C address `0x23` on CoreS3. No strap
//! pin, so [`ADDRESS`] is a single constant.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::i2c::I2c;
//! # async fn demo<B: I2c>(bus: B) -> Result<(), ltr553::Error<B::Error>> {
//! let mut als = ltr553::Ltr553::new(bus);
//! als.init().await?;
//! let reading = als.read_ambient().await?;
//! let _ = reading.lux;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::i2c::I2c;

/// Fixed 7-bit I²C address of the LTR-553 on CoreS3.
pub const ADDRESS: u8 = 0x23;

/// Expected value of the `PART_ID` register for the LTR-553. The
/// low nibble is the revision ID; only the upper nibble (`0x9`)
/// identifies the part family.
pub const PART_ID_UPPER: u8 = 0x90;

/// `ALS_CONTR` register. Controls ambient-light sensor mode + gain.
const REG_ALS_CONTR: u8 = 0x80;
/// `PS_CONTR` register. Controls proximity sensor mode + gain.
const REG_PS_CONTR: u8 = 0x81;
/// `PART_ID` register (read-only). Upper nibble `0x9` for LTR-553.
const REG_PART_ID: u8 = 0x86;
/// First byte of the ambient data burst. Registers 0x88..=0x8B hold
/// the two channel pairs: `CH1_LSB`, `CH1_MSB`, `CH0_LSB`, `CH0_MSB`.
const REG_ALS_DATA_START: u8 = 0x88;
/// `ALS_PS_STATUS` register. Bit 2 = `ALS_DATA_STATUS` (new data
/// ready since last read), bit 0 = `PS_DATA_STATUS`.
const REG_ALS_PS_STATUS: u8 = 0x8C;
/// First byte of the proximity data pair. Registers 0x8D..=0x8E hold
/// the 11-bit PS value; bit 15 of the MSB is a saturation flag.
const REG_PS_DATA_START: u8 = 0x8D;

/// `ALS_CONTR` value: bit 1 = `ALS_MODE = active`, gain = 1× (default).
const ALS_CONTR_ACTIVE_1X: u8 = 0b0000_0010;
/// `PS_CONTR` value: bits 1-0 = `PS_MODE = active` (`0b11`), default
/// gain.
const PS_CONTR_ACTIVE: u8 = 0b0000_0011;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// I²C transport error.
    I2c(E),
    /// `PART_ID` upper nibble didn't match [`PART_ID_UPPER`]. Carries
    /// the byte that was read. Common causes: wrong device at the
    /// I²C address, or a related Lite-On part with a different lux
    /// formula.
    BadPartId(u8),
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// One decoded ambient-light reading.
///
/// `ch0` is the visible + IR channel; `ch1` is IR-only. `lux` is the
/// piecewise-formula estimate at default gain / integration, which is
/// what [`Ltr553::init`] configures. Callers that change gain /
/// integration need to post-scale or reach for a more complete driver.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AmbientReading {
    /// Visible + IR channel raw count.
    pub ch0: u16,
    /// IR-only channel raw count.
    pub ch1: u16,
    /// Estimated lux. `0.0` for all-IR sources (e.g. an incandescent
    /// bulb seen through filters), positive for anything with visible
    /// content.
    pub lux: f32,
}

/// LTR-553 driver.
pub struct Ltr553<B> {
    /// The I²C bus the chip is addressed on.
    bus: B,
}

impl<B: I2c> Ltr553<B> {
    /// Wrap an I²C bus.
    #[must_use]
    pub const fn new(bus: B) -> Self {
        Self { bus }
    }

    /// Read the `PART_ID` register. Upper nibble `0x9` identifies an
    /// LTR-553 family part; lower nibble is silicon revision.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_part_id(&mut self) -> Result<u8, Error<B::Error>> {
        let mut buf = [0u8];
        self.bus
            .write_read(ADDRESS, &[REG_PART_ID], &mut buf)
            .await?;
        Ok(buf[0])
    }

    /// Validate the `PART_ID`, then enable ALS + PS at default gain /
    /// integration.
    ///
    /// Default ALS config: gain 1×, integration 100 ms, measurement
    /// rate 500 ms. Default PS config: gain 16×, active.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPartId`] if the upper nibble of `PART_ID` isn't
    ///   `0x9`.
    /// - [`Error::I2c`] on any bus failure.
    pub async fn init(&mut self) -> Result<(), Error<B::Error>> {
        let id = self.read_part_id().await?;
        if (id & 0xF0) != PART_ID_UPPER {
            return Err(Error::BadPartId(id));
        }
        self.write_register(REG_ALS_CONTR, ALS_CONTR_ACTIVE_1X)
            .await?;
        self.write_register(REG_PS_CONTR, PS_CONTR_ACTIVE).await?;
        Ok(())
    }

    /// Read the ambient channels + compute estimated lux.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_ambient(&mut self) -> Result<AmbientReading, Error<B::Error>> {
        let mut buf = [0u8; 4];
        self.bus
            .write_read(ADDRESS, &[REG_ALS_DATA_START], &mut buf)
            .await?;
        // Datasheet order: CH1_LSB, CH1_MSB, CH0_LSB, CH0_MSB.
        let ch1 = u16::from_le_bytes([buf[0], buf[1]]);
        let ch0 = u16::from_le_bytes([buf[2], buf[3]]);
        Ok(AmbientReading {
            ch0,
            ch1,
            lux: lux_from_channels(ch0, ch1),
        })
    }

    /// Read the 11-bit proximity count. Raw units; larger = closer.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_proximity(&mut self) -> Result<u16, Error<B::Error>> {
        let mut buf = [0u8; 2];
        self.bus
            .write_read(ADDRESS, &[REG_PS_DATA_START], &mut buf)
            .await?;
        // PS_DATA_1 bits 0..=2 are the high 3 bits of the 11-bit
        // value; bit 7 is the saturation flag, which we ignore.
        let high = u16::from(buf[1] & 0x07);
        let low = u16::from(buf[0]);
        Ok((high << 8) | low)
    }

    /// Read the `ALS_PS_STATUS` register.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_status(&mut self) -> Result<u8, Error<B::Error>> {
        let mut buf = [0u8];
        self.bus
            .write_read(ADDRESS, &[REG_ALS_PS_STATUS], &mut buf)
            .await?;
        Ok(buf[0])
    }

    /// Single-register write helper.
    async fn write_register(&mut self, reg: u8, value: u8) -> Result<(), Error<B::Error>> {
        self.bus.write(ADDRESS, &[reg, value]).await?;
        Ok(())
    }
}

/// Compute estimated lux from the two raw channels at the driver's
/// default gain / integration settings.
///
/// Implements the Lite-On LTR-55x app-note piecewise formula keyed on
/// `ratio = ch1 / (ch0 + ch1)`. Returns `0.0` when both channels read
/// zero or when the ratio lands in the "all IR" range (ratio ≥ 0.85).
///
/// The driver always configures gain = 1× and integration = 100 ms,
/// which makes the denominator `gain * integration = 1.0` — i.e. the
/// raw weighted sum is the lux estimate directly.
fn lux_from_channels(ch0: u16, ch1: u16) -> f32 {
    let sum = u32::from(ch0) + u32::from(ch1);
    if sum == 0 {
        return 0.0;
    }
    let ch0 = f32::from(ch0);
    let ch1 = f32::from(ch1);
    #[allow(
        clippy::cast_precision_loss,
        reason = "sum fits in u17; f32 precision is ample for lux comparisons"
    )]
    let sum_f = sum as f32;
    let ratio = ch1 / sum_f;
    // Piecewise formula (Lite-On LTR-55x app note).
    #[allow(
        clippy::suboptimal_flops,
        reason = "f32::mul_add needs libm; stay consistent with workspace"
    )]
    if ratio < 0.45 {
        1.7743 * ch0 + 1.1059 * ch1
    } else if ratio < 0.64 {
        4.2785 * ch0 - 1.9548 * ch1
    } else if ratio < 0.85 {
        0.5926 * ch0 + 0.1185 * ch1
    } else {
        0.0
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
    reason = "test scaffolding: mock panics if the driver issues an unexpected op shape"
)]
#[allow(
    clippy::suboptimal_flops,
    reason = "test expectations mirror the production formula's bare-* + bare-+ shape; \
              switching to mul_add here would obscure the formula-equivalence check"
)]
mod tests {
    use super::*;

    use core::cell::RefCell;
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};

    use embedded_hal_async::i2c::{Operation, SevenBitAddress};

    /// Event recorded by the mock harness.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Event {
        /// `write_read`: (address, register-byte, requested read length).
        WriteRead(u8, u8, usize),
        /// `write` of `[reg, value]`.
        Write(u8, u8, u8),
    }

    /// Event log + queue of canned read responses.
    struct Harness {
        events: RefCell<Vec<Event>>,
        read_responses: RefCell<Vec<Vec<u8>>>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                events: RefCell::new(Vec::new()),
                read_responses: RefCell::new(Vec::new()),
            }
        }

        fn queue_read(&self, bytes: Vec<u8>) {
            self.read_responses.borrow_mut().push(bytes);
        }

        fn events(&self) -> Vec<Event> {
            self.events.borrow().clone()
        }
    }

    /// Mock I²C bus that decodes the two patterns the driver issues:
    /// `[Write([reg]), Read(buf)]` (=`write_read`) and `[Write([reg,
    /// val])]` (=`write`). Anything else panics so a future driver
    /// change that emits a different shape surfaces here instead of
    /// reading garbage out of an unmatched canned response.
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
            match operations {
                [Operation::Write(w), Operation::Read(r)] => {
                    assert_eq!(w.len(), 1, "write_read prelude should be a single reg byte");
                    let mut responses = self.harness.read_responses.borrow_mut();
                    assert!(!responses.is_empty(), "no canned read response queued");
                    let response = responses.remove(0);
                    drop(responses);
                    assert_eq!(
                        response.len(),
                        r.len(),
                        "canned response length mismatch (queued {} vs requested {})",
                        response.len(),
                        r.len()
                    );
                    r.copy_from_slice(&response);
                    self.harness
                        .events
                        .borrow_mut()
                        .push(Event::WriteRead(address, w[0], r.len()));
                }
                [Operation::Write(w)] => {
                    assert_eq!(w.len(), 2, "single-write should be [reg, value]");
                    self.harness
                        .events
                        .borrow_mut()
                        .push(Event::Write(address, w[0], w[1]));
                }
                other => panic!("unexpected operation shape: {} ops", other.len()),
            }
            Ok(())
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
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
    fn zero_channels_gives_zero_lux() {
        assert!(lux_from_channels(0, 0).abs() < f32::EPSILON);
    }

    #[test]
    fn visible_dominant_uses_first_formula() {
        // Low-ratio (mostly visible) input: ratio ≈ 0.2.
        let ch0 = 200;
        let ch1 = 50;
        let expected = 1.7743 * f32::from(ch0) + 1.1059 * f32::from(ch1);
        let got = lux_from_channels(ch0, ch1);
        assert!((got - expected).abs() < 0.001);
    }

    #[test]
    fn all_ir_returns_zero() {
        // Fabricate a ratio just above the final branch (0.85): ch1
        // much larger than ch0.
        let lux = lux_from_channels(10, 100);
        assert!(lux.abs() < f32::EPSILON);
    }

    #[test]
    fn part_id_mask_accepts_rev_variants() {
        // The driver accepts any silicon revision: mask is `0xF0`.
        assert_eq!(0x92 & 0xF0, PART_ID_UPPER);
        assert_eq!(0x9A & 0xF0, PART_ID_UPPER);
        assert_ne!(0x82 & 0xF0, PART_ID_UPPER); // not LTR-553
    }

    #[test]
    fn lux_middle_branch_uses_second_formula() {
        // ratio = 100/(100+100) = 0.50, lands in 0.45..0.64 → formula 2.
        let ch0 = 100;
        let ch1 = 100;
        let expected = 4.2785 * f32::from(ch0) - 1.9548 * f32::from(ch1);
        let got = lux_from_channels(ch0, ch1);
        assert!(
            (got - expected).abs() < 0.001,
            "got {got}, expected {expected} (mid-ratio formula)"
        );
    }

    #[test]
    fn lux_high_ratio_uses_third_formula() {
        // ratio = 300/(100+300) = 0.75, lands in 0.64..0.85 → formula 3.
        let ch0 = 100;
        let ch1 = 300;
        let expected = 0.5926 * f32::from(ch0) + 0.1185 * f32::from(ch1);
        let got = lux_from_channels(ch0, ch1);
        assert!(
            (got - expected).abs() < 0.001,
            "got {got}, expected {expected} (high-ratio formula)"
        );
    }

    #[test]
    fn lux_at_ratio_0_45_boundary_uses_second_formula() {
        // ratio = 9/(11+9) = 0.45 (rounds to the same nearest f32 as
        // the literal `0.45`; 0.45 itself is a repeating binary
        // fraction so neither side is bit-exact, they just agree).
        // Branch comparators use `<`, so ratio == 0.45 falls out of
        // formula 1 (`ratio < 0.45` is false) and into formula 2.
        // Pinning this so a future "<=" tweak surfaces the boundary
        // ambiguity rather than silently shifting which formula
        // applies at the cut.
        let ch0: u16 = 11;
        let ch1: u16 = 9;
        let ratio = f32::from(ch1) / f32::from(ch0 + ch1);
        assert_eq!(
            ratio.to_bits(),
            0.45_f32.to_bits(),
            "fixture didn't land at the boundary: ratio={ratio}"
        );
        let expected = 4.2785 * f32::from(ch0) - 1.9548 * f32::from(ch1);
        let got = lux_from_channels(ch0, ch1);
        assert!(
            (got - expected).abs() < 0.001,
            "got {got}, expected {expected} (formula 2 at ratio=0.45 boundary)"
        );
    }

    #[test]
    fn init_validates_part_id_then_configures_als_and_ps() {
        let harness = Harness::new();
        harness.queue_read(vec![0x9A]); // valid LTR-553, rev nibble A
        let mut bus = MockI2c { harness: &harness };
        let mut driver = Ltr553::new(&mut bus);
        block_on(driver.init()).unwrap();

        let expected = vec![
            Event::WriteRead(ADDRESS, REG_PART_ID, 1),
            Event::Write(ADDRESS, REG_ALS_CONTR, ALS_CONTR_ACTIVE_1X),
            Event::Write(ADDRESS, REG_PS_CONTR, PS_CONTR_ACTIVE),
        ];
        assert_eq!(harness.events(), expected);
    }

    #[test]
    fn init_rejects_bad_part_id_without_writing_config() {
        let harness = Harness::new();
        harness.queue_read(vec![0x82]); // upper nibble != 0x9
        let mut bus = MockI2c { harness: &harness };
        let mut driver = Ltr553::new(&mut bus);

        let err = block_on(driver.init()).unwrap_err();
        assert!(matches!(err, Error::BadPartId(0x82)), "got {err:?}");
        // No ALS / PS writes should have happened — the part ID gate
        // short-circuits before any config write.
        assert_eq!(
            harness.events(),
            vec![Event::WriteRead(ADDRESS, REG_PART_ID, 1)]
        );
    }

    #[test]
    fn read_ambient_decodes_ch1_then_ch0_little_endian() {
        // Datasheet burst order at REG_ALS_DATA_START (0x88):
        //   CH1_LSB, CH1_MSB, CH0_LSB, CH0_MSB. Pinning so a future
        //   refactor that swaps the two pairs surfaces here instead
        //   of as wrong lux numbers in the field.
        let harness = Harness::new();
        harness.queue_read(vec![0x34, 0x12, 0x78, 0x56]);
        let mut bus = MockI2c { harness: &harness };
        let mut driver = Ltr553::new(&mut bus);
        let reading = block_on(driver.read_ambient()).unwrap();

        assert_eq!(reading.ch1, 0x1234);
        assert_eq!(reading.ch0, 0x5678);
        assert_eq!(
            harness.events(),
            vec![Event::WriteRead(ADDRESS, REG_ALS_DATA_START, 4)]
        );
        // Spot-check the lux path is wired through: ch1/(ch0+ch1)
        // here is ~0.21, well below 0.45 → formula 1.
        let expected_lux = 1.7743 * f32::from(reading.ch0) + 1.1059 * f32::from(reading.ch1);
        assert!((reading.lux - expected_lux).abs() < 0.01);
    }

    #[test]
    fn read_proximity_extracts_11_bit_value() {
        // Encoded as: PS_DATA_0 = low 8 bits, PS_DATA_1 = high 3 bits
        // in bits 0..=2. Construct a known 11-bit value (0x456 = 0b100
        // _0101_0110), distribute it correctly across the two bytes,
        // and assert the decoder reassembles it.
        let target: u16 = 0x456;
        let low = (target & 0xFF) as u8;
        let high = ((target >> 8) & 0x07) as u8;
        let harness = Harness::new();
        harness.queue_read(vec![low, high]);
        let mut bus = MockI2c { harness: &harness };
        let mut driver = Ltr553::new(&mut bus);
        let ps = block_on(driver.read_proximity()).unwrap();
        assert_eq!(ps, target);
    }

    #[test]
    fn read_proximity_ignores_saturation_flag_and_upper_bits_of_high_byte() {
        // Bit 7 of PS_DATA_1 is the saturation flag; the driver
        // deliberately ignores it. Bits 3..=6 are reserved. Confirm
        // the decoder masks to only bits 0..=2 of the high byte so a
        // saturation event (or noise in reserved bits) doesn't add
        // garbage to the returned count.
        let harness = Harness::new();
        // Low byte = 0xFF; high byte = 0b1111_1101 — saturation+reserved
        // bits all set, low 3 bits = 0b101 = 5.
        harness.queue_read(vec![0xFF, 0b1111_1101]);
        let mut bus = MockI2c { harness: &harness };
        let mut driver = Ltr553::new(&mut bus);
        let ps = block_on(driver.read_proximity()).unwrap();
        // Expected: (5 << 8) | 0xFF = 0x5FF. NOT 0xFDFF (which would
        // include saturation flag) or 0x7FF (which would include all
        // reserved + low bits).
        assert_eq!(ps, 0x5FF);
    }
}
