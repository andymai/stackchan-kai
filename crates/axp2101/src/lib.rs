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

    /// Host-side I²C mock that records every transaction payload in order.
    struct MockI2c {
        transactions: RefCell<Vec<(u8, Vec<u8>)>>,
    }

    impl MockI2c {
        fn new() -> Self {
            Self {
                transactions: RefCell::new(Vec::new()),
            }
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
            for op in operations {
                if let Operation::Write(buf) = op {
                    self.transactions.borrow_mut().push((address, buf.to_vec()));
                }
                // Reads and write-reads never issued by the driver under test.
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
}
