//! # axp2101
//!
//! `no_std` driver for the X-Powers AXP2101 PMIC used on the M5Stack CoreS3.
//!
//! The driver is generic over any `embedded_hal_async::i2c::I2c`, and the
//! high-level [`Axp2101::init_cores3`] method applies the exact register
//! sequence the `M5Unified` library uses for the CoreS3 board — enough to
//! bring up the LCD rails **and** configure the power-management behavior
//! (button timing, BATFET, PMU common config) so the chip doesn't
//! auto-shutdown after a few seconds of idle.
//!
//! Battery-state readout, charging configuration, and IRQ handling are left
//! for future releases; adding them is a matter of wiring more register
//! accesses through the existing I²C surface.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::i2c::I2c;
//! # async fn demo<B: I2c>(bus: B) -> Result<(), axp2101::Error<B::Error>> {
//! let mut pmic = axp2101::Axp2101::new(bus);
//! pmic.init_cores3().await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::i2c::I2c;

/// 7-bit I²C address of the AXP2101 on CoreS3.
pub const ADDRESS: u8 = 0x34;

/// `IRQ_EN_1` register address (`0x41`). Controls which power-key
/// events generate interrupts.
const REG_IRQ_EN_1: u8 = 0x41;
/// `IRQ_STATUS_1` register address (`0x49`). Flags for power-key
/// events; write-1-to-clear.
const REG_IRQ_STATUS_1: u8 = 0x49;
/// `BAT_GAUGE` register address (`0xA4`). Reads as 0..=100, the
/// AXP2101's internal coulomb-counter / OCV-blend state-of-charge
/// estimate. Reads outside that range only happen during transient
/// chip warm-up and are clamped by [`Axp2101::read_battery_percent`].
const REG_BAT_GAUGE: u8 = 0xA4;
/// `PMU_STATUS_1` register address (`0x00`). Bit 5 = `VBUSGD`, set
/// when the chip detects a valid VBUS voltage on the USB-C input.
const REG_PMU_STATUS_1: u8 = 0x00;
/// Bit mask for `VBUSGD` inside `PMU_STATUS_1`. `1` = USB power good
/// (chip can be charging or simply running off USB).
const VBUS_GOOD_BIT: u8 = 1 << 5;

/// Maximum value the AXP2101 battery-gauge register reports. Anything
/// higher is read-back noise during ADC settle and is saturated by
/// [`Axp2101::read_battery_percent`].
pub const BATTERY_PERCENT_MAX: u8 = 100;

/// Bit for "short-press" in AXP2101's `IRQ_EN_1` / `IRQ_STATUS_1`
/// registers (release after a brief hold, < 1 s).
///
/// Bit layout of `IRQ_STATUS_1`:
///   - bit 1: power-key positive edge (press)
///   - bit 0: power-key negative edge (release)
///   - bit 4: short-press
///   - bit 5: long-press
///   - bit 6: over-press (held > 2 s)
pub const IRQ_SHORT_PRESS_BIT: u8 = 1 << 4;

/// Bit for the power-key positive edge (button press).
///
/// Used together with [`IRQ_RELEASE_EDGE_BIT`] for software-timed
/// long-press detection — the chip's own long-press IRQ has a ≥2 s
/// minimum threshold, which is too slow for a UI gesture.
pub const IRQ_PRESS_EDGE_BIT: u8 = 1 << 1;

/// Bit for the power-key negative edge (button release). Pairs with
/// [`IRQ_PRESS_EDGE_BIT`] for software duration measurement.
pub const IRQ_RELEASE_EDGE_BIT: u8 = 1 << 0;

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

/// Register + value pair. Used by [`Axp2101::init_cores3`] to apply a
/// fixed initialization sequence in one method.
type RegWrite = (u8, u8);

/// `M5Unified`'s CoreS3 AXP2101 register sequence, in order.
///
/// The values are copied verbatim from `M5Unified`'s
/// `Power_Class.cpp` (both the CoreS3-specific block at
/// `board_M5StackCoreS3` and the shared AXP2101 block that runs after
/// it). Writing the full sequence is what
/// prevents the "idle → auto shutdown" behavior seen with the minimal
/// LDO-only init: register `0x27` sets the button press timing to sane
/// values (1 s hold to wake, 4 s hold to power off) and `0x10` + `0x12`
/// put the chip into the operating mode `M5Unified` boards expect.
///
/// Note that LDO voltage registers (`0x92`..`0x95`) must be written **before**
/// the enable bitmap at `0x90` so the rails come up at the correct voltage
/// on their first on-edge.
const CORES3_INIT_SEQUENCE: &[RegWrite] = &[
    // LDO voltage setpoints. Encoding: (mV - 500) / 100 for ALDOs.
    (0x92, 13), // ALDO1 = 1.8V  — AW88298 audio codec
    (0x93, 28), // ALDO2 = 3.3V  — ES7210 audio ADC
    (0x94, 28), // ALDO3 = 3.3V  — camera
    (0x95, 28), // ALDO4 = 3.3V  — TF card slot
    (0x96, 28), // BLDO1 = 3.3V  — LCD backlight (PoR default is 0.5V; must
    //                             write explicitly or the panel enables
    //                             with no light)
    (0x97, 28), // BLDO2 = 3.3V  — LCD logic rail
    // LDO enable bitmap. 0xBF enables ALDO1..4 (bits 0..3) and BLDO1..2
    // (bits 4..5). Voltages for every rail in the mask are set above so
    // each comes up at 3.3V (ALDO1 at 1.8V) on its first on-edge.
    (0x90, 0xBF),
    // Power-key timing. 0x00 = hold 1 s to wake, 4 s to power off. Without
    // this write the chip boots with an aggressive default that treats
    // mild button glitches as shutdown requests.
    (0x27, 0x00),
    // PMU common config: bits 4/5 set "internal off-discharge enable",
    // which `M5Unified` applies to every AXP2101 board. Required for stable
    // power-on behavior on CoreS3.
    (0x10, 0x30),
    // BATFET disable. Keeps the chip from trying to run through the
    // battery FET when no battery is attached — that path otherwise
    // triggers an undervoltage shutdown.
    (0x12, 0x00),
    // Battery detection enable (no-op if battery not present).
    (0x68, 0x01),
    // CHGLED behavior: controlled by the charger, flashing on charge.
    (0x69, 0x13),
    // DLDO1 = 3.3V — LCD backlight. `M5Unified`'s `SetBrightness` writes
    // this register with `(brightness + 641) >> 5`, mapping 0..255 input
    // to 20..28 register values (~1.5V..3.3V). 28 = full brightness; a
    // future `set_brightness` API can drop it.
    (0x99, 28),
    // Enable the PMU's ADC block so later reads of battery / VBUS voltage
    // return something meaningful.
    (0x30, 0x0F),
];

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

    /// Consume the driver and return the underlying I²C bus.
    ///
    /// Useful for single-task firmware that needs to hand the bus to a
    /// second peripheral (e.g. the CoreS3's AW9523 IO expander) after
    /// PMIC bring-up is done, without pulling in a shared-bus abstraction.
    pub fn into_inner(self) -> B {
        self.bus
    }

    /// Apply the M5Stack CoreS3 power-management defaults in one shot.
    ///
    /// Mirrors the register sequence `M5Unified` writes on CoreS3 boot:
    /// LDO voltages, enable bitmap, power-key timing, PMU common config,
    /// BATFET, battery detect, and ADC enable. After this returns, the
    /// LCD rails are up and the chip is configured not to auto-shut down
    /// on idle.
    ///
    /// # Errors
    ///
    /// Returns [`Error::I2c`] on the first failed I²C write.
    pub async fn init_cores3(&mut self) -> Result<(), Error<B::Error>> {
        for &(reg, val) in CORES3_INIT_SEQUENCE {
            self.write_reg(reg, val).await?;
        }
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

    /// Enable the "short-press" power-key IRQ. After calling,
    /// [`Axp2101::check_short_press_edge`] will observe one edge per
    /// brief button press (< 1 s hold + release).
    ///
    /// Leaves other `IRQ_EN_1` bits untouched via read-modify-write.
    ///
    /// # Errors
    ///
    /// Returns [`Error::I2c`] on bus errors.
    pub async fn enable_power_key_short_press_irq(&mut self) -> Result<(), Error<B::Error>> {
        let current = self.read_reg(REG_IRQ_EN_1).await?;
        self.write_reg(REG_IRQ_EN_1, current | IRQ_SHORT_PRESS_BIT)
            .await?;
        Ok(())
    }

    /// Check for a pending short-press edge and clear it atomically.
    ///
    /// Returns `true` iff the short-press bit was set in `IRQ_STATUS_1`
    /// (`0x49`). The bit is cleared (write-1-to-clear) as part of the
    /// check so a subsequent call returns `false` until another press
    /// arrives.
    ///
    /// # Errors
    ///
    /// Returns [`Error::I2c`] on bus errors. A bus failure at *read*
    /// leaves no edge flag visible to the caller; a failure at *clear*
    /// means the next call may see a stale-true result (acceptable —
    /// a double-fire of the tap signal is benign given the downstream
    /// modifier is edge-triggered).
    pub async fn check_short_press_edge(&mut self) -> Result<bool, Error<B::Error>> {
        let status = self.read_reg(REG_IRQ_STATUS_1).await?;
        if status & IRQ_SHORT_PRESS_BIT == 0 {
            return Ok(false);
        }
        // Write 1 to clear; preserve any other flags the chip set
        // during this read's narrow window so we don't lose them.
        self.write_reg(REG_IRQ_STATUS_1, IRQ_SHORT_PRESS_BIT)
            .await?;
        Ok(true)
    }

    /// Enable both press-edge and release-edge IRQs.
    ///
    /// Pairs with [`Axp2101::take_power_key_edges`] for software-timed
    /// long-press detection: the caller measures the gap between a
    /// `press_edge` and the subsequent `release_edge` to differentiate
    /// taps from holds with sub-second granularity (the chip's built-in
    /// long-press IRQ has a ≥2 s minimum threshold).
    ///
    /// Leaves other `IRQ_EN_1` bits untouched via read-modify-write.
    /// Safe to call alongside [`Axp2101::enable_power_key_short_press_irq`]
    /// — both signals are independent.
    ///
    /// # Errors
    ///
    /// Returns [`Error::I2c`] on bus errors.
    pub async fn enable_power_key_edge_irqs(&mut self) -> Result<(), Error<B::Error>> {
        let current = self.read_reg(REG_IRQ_EN_1).await?;
        let bits = IRQ_PRESS_EDGE_BIT | IRQ_RELEASE_EDGE_BIT;
        self.write_reg(REG_IRQ_EN_1, current | bits).await?;
        Ok(())
    }

    /// Drain pending press / release edges atomically.
    ///
    /// Returns `(press, release)`: each `true` iff the corresponding
    /// flag was set in `IRQ_STATUS_1`. Both flags (if set) are cleared
    /// in a single write so the next call observes only fresh edges.
    /// In a single press-and-release that arrives between polls, both
    /// flags will fire together — callers should treat `(true, true)`
    /// as a tap shorter than the polling period.
    ///
    /// # Errors
    ///
    /// Returns [`Error::I2c`] on bus errors. A bus failure at *clear*
    /// can leak edges into the next read; downstream consumers must
    /// be tolerant of an extra spurious tap (the existing tap pipeline
    /// is edge-triggered, so this is benign).
    pub async fn take_power_key_edges(&mut self) -> Result<(bool, bool), Error<B::Error>> {
        let status = self.read_reg(REG_IRQ_STATUS_1).await?;
        let press = status & IRQ_PRESS_EDGE_BIT != 0;
        let release = status & IRQ_RELEASE_EDGE_BIT != 0;
        if press || release {
            let mut clear = 0u8;
            if press {
                clear |= IRQ_PRESS_EDGE_BIT;
            }
            if release {
                clear |= IRQ_RELEASE_EDGE_BIT;
            }
            self.write_reg(REG_IRQ_STATUS_1, clear).await?;
        }
        Ok((press, release))
    }

    /// Read whether the AXP2101 sees valid VBUS (USB) input voltage.
    ///
    /// Returns `true` when the chip's `VBUSGD` flag in `PMU_STATUS_1`
    /// is set. The flag asserts whenever there's enough voltage on the
    /// USB-C input for the chip to consider it a valid source —
    /// independent of whether the unit is actively charging the
    /// battery, just running off USB, or both.
    ///
    /// # Errors
    ///
    /// Returns [`Error::I2c`] on bus errors.
    pub async fn read_usb_power_good(&mut self) -> Result<bool, Error<B::Error>> {
        let status = self.read_reg(REG_PMU_STATUS_1).await?;
        Ok(status & VBUS_GOOD_BIT != 0)
    }

    /// Read the AXP2101's battery state-of-charge estimate, in percent.
    ///
    /// Returns a value in <code>0..=[BATTERY_PERCENT_MAX]</code>. The
    /// chip's gauge register can transiently report values above 100
    /// during ADC settling immediately after `init_cores3` enables
    /// battery detect; this method saturates instead of surfacing the
    /// physically-meaningless reading.
    ///
    /// Note: the `SoC` estimate is only meaningful once the AXP2101
    /// has observed at least one charge / discharge transition since
    /// power-up — on a freshly-powered unit it can read 0% for several
    /// seconds before stabilising. Callers that poll this on a
    /// schedule should expect the early values to be noisy.
    ///
    /// # Errors
    ///
    /// Returns [`Error::I2c`] on bus errors.
    pub async fn read_battery_percent(&mut self) -> Result<u8, Error<B::Error>> {
        let raw = self.read_reg(REG_BAT_GAUGE).await?;
        Ok(raw.min(BATTERY_PERCENT_MAX))
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test scaffolding: Infallible bus error makes unwrap() sound"
)]
mod tests {
    use super::*;
    use core::cell::RefCell;
    use embedded_hal_async::i2c::{Operation, SevenBitAddress};

    /// Host-side I²C mock. Records every write payload in order and
    /// answers reads from a per-register canned-value table. Reads
    /// against unmapped registers return `0`.
    struct MockI2c {
        transactions: RefCell<Vec<(u8, Vec<u8>)>>,
        read_responses: RefCell<Vec<(u8, u8)>>,
    }

    impl MockI2c {
        fn new() -> Self {
            Self {
                transactions: RefCell::new(Vec::new()),
                read_responses: RefCell::new(Vec::new()),
            }
        }

        /// Stage a value to return when the driver reads `reg`.
        fn with_register(self, reg: u8, value: u8) -> Self {
            self.read_responses.borrow_mut().push((reg, value));
            self
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
            // For a `write_read`, the operation list is `[Write(reg),
            // Read(buf)]`. Track the most-recent write so the read can
            // look up the canned value for that register.
            let mut last_write_reg: Option<u8> = None;
            for op in operations {
                match op {
                    Operation::Write(buf) => {
                        self.transactions.borrow_mut().push((address, buf.to_vec()));
                        if let Some(&reg) = buf.first() {
                            last_write_reg = Some(reg);
                        }
                    }
                    Operation::Read(buf) => {
                        let value = last_write_reg
                            .and_then(|reg| {
                                self.read_responses
                                    .borrow()
                                    .iter()
                                    .rev()
                                    .find(|(r, _)| *r == reg)
                                    .map(|(_, v)| *v)
                            })
                            .unwrap_or(0);
                        if let Some(slot) = buf.first_mut() {
                            *slot = value;
                        }
                    }
                }
            }
            Ok(())
        }
    }

    /// Tiny future poller used instead of pulling in an async executor.
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
    fn init_cores3_issues_exact_m5unified_register_sequence() {
        let mut pmic = Axp2101::new(MockI2c::new());
        block_on(pmic.init_cores3()).unwrap();

        // Every write hits the PMIC address.
        let txs = pmic.bus.transactions.borrow().clone();
        assert_eq!(txs.len(), CORES3_INIT_SEQUENCE.len());
        for (addr, _) in &txs {
            assert_eq!(*addr, ADDRESS, "all init writes must target AXP2101");
        }

        // Each write is a (reg, value) pair in the exact order of
        // CORES3_INIT_SEQUENCE. This is a golden test: if anyone edits
        // the sequence without meaning to, this fails.
        let actual: Vec<(u8, u8)> = txs.iter().map(|(_, buf)| (buf[0], buf[1])).collect();
        let expected: Vec<(u8, u8)> = CORES3_INIT_SEQUENCE.to_vec();
        assert_eq!(actual, expected);
    }

    #[test]
    fn init_cores3_writes_backlight_voltage_before_enable_bitmap() {
        // BLDO1 (0x96) / BLDO2 (0x97) voltage setpoints must come before
        // the enable bitmap (0x90) so the rails come up at 3.3V, not the
        // 0.5V PoR default.
        let bldo1 = CORES3_INIT_SEQUENCE
            .iter()
            .position(|&(r, _)| r == 0x96)
            .unwrap();
        let enable = CORES3_INIT_SEQUENCE
            .iter()
            .position(|&(r, _)| r == 0x90)
            .unwrap();
        assert!(bldo1 < enable, "BLDO1 voltage must precede enable bitmap");
    }

    #[test]
    fn init_cores3_keeps_battery_detect_and_adc_enabled() {
        // Battery detect + ADC enable are both part of the sequence —
        // without them, later battery-voltage reads return zero.
        assert!(CORES3_INIT_SEQUENCE.contains(&(0x68, 0x01)));
        assert!(CORES3_INIT_SEQUENCE.contains(&(0x30, 0x0F)));
    }

    #[test]
    fn into_inner_releases_bus() {
        let pmic = Axp2101::new(MockI2c::new());
        let _bus: MockI2c = pmic.into_inner();
    }

    #[test]
    fn read_battery_percent_returns_register_value() {
        let bus = MockI2c::new().with_register(REG_BAT_GAUGE, 73);
        let mut pmic = Axp2101::new(bus);
        let pct = block_on(pmic.read_battery_percent()).unwrap();
        assert_eq!(pct, 73);
    }

    #[test]
    fn read_battery_percent_saturates_above_max() {
        // Some chips have been observed reporting 0xFF during ADC settle
        // immediately after init. The driver clamps to BATTERY_PERCENT_MAX.
        let bus = MockI2c::new().with_register(REG_BAT_GAUGE, 0xFF);
        let mut pmic = Axp2101::new(bus);
        let pct = block_on(pmic.read_battery_percent()).unwrap();
        assert_eq!(pct, BATTERY_PERCENT_MAX);
    }

    #[test]
    fn read_battery_percent_passes_zero_through() {
        // A genuinely-flat battery reads 0; the driver must not
        // mis-clamp this up.
        let bus = MockI2c::new().with_register(REG_BAT_GAUGE, 0);
        let mut pmic = Axp2101::new(bus);
        let pct = block_on(pmic.read_battery_percent()).unwrap();
        assert_eq!(pct, 0);
    }

    #[test]
    fn read_usb_power_good_true_when_vbus_bit_set() {
        // PMU_STATUS_1 with VBUSGD bit set + some unrelated bits.
        let bus = MockI2c::new().with_register(REG_PMU_STATUS_1, VBUS_GOOD_BIT | 0x83);
        let mut pmic = Axp2101::new(bus);
        let usb_good = block_on(pmic.read_usb_power_good()).unwrap();
        assert!(usb_good);
    }

    #[test]
    fn read_usb_power_good_false_when_vbus_bit_clear() {
        // PMU_STATUS_1 with everything except VBUSGD set — the read
        // must isolate the right bit, not key off any-bit-set.
        let bus = MockI2c::new().with_register(REG_PMU_STATUS_1, !VBUS_GOOD_BIT);
        let mut pmic = Axp2101::new(bus);
        let usb_good = block_on(pmic.read_usb_power_good()).unwrap();
        assert!(!usb_good);
    }

    #[test]
    fn read_usb_power_good_zero_register_returns_false() {
        let bus = MockI2c::new().with_register(REG_PMU_STATUS_1, 0);
        let mut pmic = Axp2101::new(bus);
        let usb_good = block_on(pmic.read_usb_power_good()).unwrap();
        assert!(!usb_good);
    }

    #[test]
    fn enable_power_key_edge_irqs_or_in_press_release_bits() {
        // Mock starts with the short-press bit already enabled (the
        // existing button task does that); the new method must OR the
        // edge bits onto the same byte, not overwrite.
        let bus = MockI2c::new().with_register(REG_IRQ_EN_1, IRQ_SHORT_PRESS_BIT);
        let mut pmic = Axp2101::new(bus);
        block_on(pmic.enable_power_key_edge_irqs()).unwrap();

        // Find the actual write to IRQ_EN_1 in the recorded transactions.
        let writes: Vec<(u8, u8)> = pmic
            .bus
            .transactions
            .borrow()
            .iter()
            .filter(|(_, buf)| buf.len() == 2)
            .map(|(_, buf)| (buf[0], buf[1]))
            .collect();
        let (_, value) = writes
            .iter()
            .rev()
            .find(|(reg, _)| *reg == REG_IRQ_EN_1)
            .copied()
            .unwrap();
        assert_eq!(
            value,
            IRQ_SHORT_PRESS_BIT | IRQ_PRESS_EDGE_BIT | IRQ_RELEASE_EDGE_BIT
        );
    }

    #[test]
    fn take_power_key_edges_decodes_and_clears_only_set_bits() {
        // Status reports both edges set + an unrelated bit. The clear
        // write must target only the press/release bits — leaving the
        // unrelated flag intact for whichever consumer owns it.
        let bus = MockI2c::new().with_register(
            REG_IRQ_STATUS_1,
            IRQ_PRESS_EDGE_BIT | IRQ_RELEASE_EDGE_BIT | IRQ_SHORT_PRESS_BIT,
        );
        let mut pmic = Axp2101::new(bus);
        let (press, release) = block_on(pmic.take_power_key_edges()).unwrap();
        assert!(press);
        assert!(release);

        let writes: Vec<(u8, u8)> = pmic
            .bus
            .transactions
            .borrow()
            .iter()
            .filter(|(_, buf)| buf.len() == 2)
            .map(|(_, buf)| (buf[0], buf[1]))
            .collect();
        let (_, clear_value) = writes
            .iter()
            .find(|(reg, _)| *reg == REG_IRQ_STATUS_1)
            .copied()
            .unwrap();
        assert_eq!(clear_value, IRQ_PRESS_EDGE_BIT | IRQ_RELEASE_EDGE_BIT);
    }

    #[test]
    fn take_power_key_edges_skips_clear_when_no_edge_set() {
        // Pure-zero status must NOT issue a clearing write — saves bus
        // bandwidth on the (common) idle case where the button isn't
        // being touched.
        let bus = MockI2c::new().with_register(REG_IRQ_STATUS_1, 0);
        let mut pmic = Axp2101::new(bus);
        let (press, release) = block_on(pmic.take_power_key_edges()).unwrap();
        assert!(!press);
        assert!(!release);

        let status_writes = pmic
            .bus
            .transactions
            .borrow()
            .iter()
            .filter(|(_, buf)| buf.len() == 2 && buf[0] == REG_IRQ_STATUS_1)
            .count();
        assert_eq!(status_writes, 0);
    }
}
