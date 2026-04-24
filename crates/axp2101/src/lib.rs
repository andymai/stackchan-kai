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

#![no_std]
#![deny(unsafe_code)]

use embedded_hal_async::i2c::I2c;

/// 7-bit I²C address of the AXP2101 on CoreS3.
pub const ADDRESS: u8 = 0x34;

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
    // LDO enable bitmap. 0xBF enables ALDO1..4 (bits 0..3) and BLDO1..2
    // (bits 4..5); BLDO1/BLDO2 default to 3.3V for the LCD backlight +
    // logic rails on CoreS3 so no explicit voltage write is needed.
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
    // DLDO1 = 0.5V — gates the vibration motor. Safe default off-ish.
    (0x99, 0x00),
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
}
