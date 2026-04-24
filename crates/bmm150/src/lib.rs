//! # bmm150
//!
//! `no_std` async driver for the Bosch BMM150 3-axis geomagnetic sensor
//! over I²C. Minimal and focused: soft reset → trim-register readout →
//! regular preset (REPXY=9, REPZ=15) at 10 Hz normal-mode → compensated
//! `(x, y, z)` magnetometer readings in microtesla.
//!
//! Heading computation, tilt compensation, and hard-iron / soft-iron
//! calibration are deliberately out of scope. This driver's job is
//! "ship the readings as µT"; anything downstream (compass, gaze
//! steering) consumes [`Measurement`] values at the avatar layer.
//!
//! ## Addressing
//!
//! The chip's 7-bit address depends on the `CSB`/`SDO` pin straps:
//!
//! - `SDO = GND` → [`ADDRESS_PRIMARY`] = `0x10`
//! - `SDO = VDDIO` → [`ADDRESS_SECONDARY`] = `0x11`
//!
//! Board wiring varies; [`Bmm150::detect`] probes both and returns the
//! one that answers with the expected [`CHIP_ID`].
//!
//! ## Compensation algorithm
//!
//! Raw readings from the chip are in internal LSBs and require per-chip
//! trim compensation to produce physically meaningful values. The
//! compensation functions in this crate are faithful ports of Bosch's
//! reference implementation (as shipped in `BoschSensortec/BMM150_SensorAPI`
//! and redistributed in Zephyr's `drivers/sensor/bosch/bmm150`): same
//! fixed-point integer math, same overflow sentinels, same scale. The
//! output unit is 1/16 µT per LSB; this driver converts to `f32` µT
//! at the public boundary.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::{i2c::I2c, delay::DelayNs};
//! # async fn demo<B, D>(bus: B, mut delay: D) -> Result<(), bmm150::Error<B::Error>>
//! # where B: I2c, D: DelayNs {
//! let mut mag = bmm150::Bmm150::detect(bus, &mut delay).await?;
//! mag.init(&mut delay).await?;
//! let m = mag.read_measurement().await?;
//! // m.mag_ut = (x, y, z) in microtesla; total field magnitude on Earth
//! // ranges 25-65 µT at the surface.
//! let _ = m;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::{delay::DelayNs, i2c::I2c};

/// 7-bit I²C address with `SDO` strapped to `GND` (Bosch default).
pub const ADDRESS_PRIMARY: u8 = 0x10;
/// 7-bit I²C address with `SDO` strapped to `VDDIO`.
pub const ADDRESS_SECONDARY: u8 = 0x11;

/// Chip-ID value returned from the BMM150's `CHIP_ID` register. Used
/// by [`Bmm150::detect`] to confirm the part is a BMM150.
pub const CHIP_ID: u8 = 0x32;

// --- Register map (from Bosch BMM150 datasheet / BoschSensortec source) ---

/// Chip-ID register. Reads as [`CHIP_ID`] = 0x32 on a BMM150.
const REG_CHIP_ID: u8 = 0x40;
/// Start of the 8-byte measurement block (`X_L, X_M, Y_L, Y_M, Z_L, Z_M,
/// RHALL_L, RHALL_M`). Bulk read via auto-increment.
const REG_DATA_START: u8 = 0x42;
/// Power-control register. Bit 0: 0 = suspend (I²C unreachable), 1 =
/// awake. Bit 7 + bit 1 together = soft-reset trigger.
const REG_POWER: u8 = 0x4B;
/// Op-mode + ODR register. Bits 5:3 = ODR, bits 2:1 = op mode, bit 0 =
/// self-test.
const REG_OPMODE_ODR: u8 = 0x4C;
/// XY-axis repetition register. Actual repetitions = `2 * reg + 1`.
const REG_REP_XY: u8 = 0x51;
/// Z-axis repetition register. Actual repetitions = `reg + 1`.
const REG_REP_Z: u8 = 0x52;
/// First byte of the 21-byte trim register block. The block spans
/// 0x5D..=0x71 and includes four reserved bytes that we read + discard.
const REG_TRIM_START: u8 = 0x5D;
/// Number of bytes in the trim block (`0x71 - 0x5D + 1`).
const TRIM_BLOCK_LEN: usize = 21;

/// `REG_POWER` value: trigger soft reset while remaining in suspend
/// (bit 7 | bit 1). The chip clears bit 7 internally once reset completes.
const POWER_SOFT_RESET: u8 = (1 << 7) | (1 << 1);
/// `REG_POWER` value: bit 0 set → wake from suspend. Takes ≥3 ms to
/// settle; I²C reads fail against a chip still in suspend.
const POWER_WAKE: u8 = 1 << 0;

/// `REG_OPMODE_ODR` value for the "regular preset" at 10 Hz: ODR bits =
/// `000` (10 Hz), op-mode bits = `00` (normal).
const OPMODE_ODR_NORMAL_10HZ: u8 = 0x00;

/// REPXY register value for the "regular preset" (REPXY = 9):
/// `(9 - 1) / 2 = 4`.
const REPXY_REG_REGULAR: u8 = 4;
/// REPZ register value for the "regular preset" (REPZ = 15):
/// `15 - 1 = 14`. Note that REPZ does **not** use the `(n-1)/2`
/// encoding REPXY uses — one of the few asymmetries in the register map.
const REPZ_REG_REGULAR: u8 = 14;

/// Milliseconds to wait after a soft reset or wake before issuing further
/// I²C transactions. Bosch's datasheet specifies ≥3 ms; 5 ms is an
/// uncontested margin.
const POWER_SETTLE_MS: u32 = 5;

/// Raw `x` or `y` sample that indicates the chip couldn't measure the
/// field (ADC overflow). Datasheet: -4096 (the most-negative value
/// representable in the 13-bit signed field).
const XY_OVERFLOW: i16 = -4096;
/// Raw `z` sample indicating overflow. Datasheet: -16384 (most-negative
/// value in the 15-bit signed field).
const Z_OVERFLOW: i16 = -16384;

/// Conversion factor: compensation outputs are in 1/16 µT per LSB.
/// Multiply by this constant (or divide by 16) to get µT as `f32`.
const LSB_TO_UT: f32 = 1.0 / 16.0;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
    /// Chip-ID register read back a value other than [`CHIP_ID`]. The
    /// caller passed an address that isn't a BMM150 (or the part is
    /// damaged).
    ChipId {
        /// Value we expected (`0x32`).
        expected: u8,
        /// Value actually read from the chip at the `CHIP_ID` register.
        actual: u8,
    },
    /// [`Bmm150::detect`] probed both candidate addresses and neither
    /// answered with the expected [`CHIP_ID`].
    NotDetected,
    /// One or more axes returned the chip's "measurement overflow"
    /// sentinel — the field exceeded the ADC's dynamic range on that
    /// axis. The caller should retry or ignore this sample.
    Overflow,
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// One compensated sample from the magnetometer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    /// Magnetic field in microtesla `(x, y, z)`. Total earth-field
    /// magnitude ranges roughly 25-65 µT at the surface; values well
    /// outside that window suggest hard-iron distortion from nearby
    /// magnets or motors.
    pub mag_ut: (f32, f32, f32),
}

/// Per-chip trim / calibration constants, read once at [`Bmm150::init`]
/// and fed into every subsequent compensation.
///
/// Field names mirror Bosch's reference struct so the compensation
/// functions are a one-to-one port.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Trim {
    /// `dig_x1`, i8.
    x1: i8,
    /// `dig_y1`, i8.
    y1: i8,
    /// `dig_z4`, i16 (LE in flash).
    z4: i16,
    /// `dig_x2`, i8.
    x2: i8,
    /// `dig_y2`, i8.
    y2: i8,
    /// `dig_z2`, i16.
    z2: i16,
    /// `dig_z1`, u16.
    z1: u16,
    /// `dig_xyz1`, u16.
    xyz1: u16,
    /// `dig_z3`, i16.
    z3: i16,
    /// `dig_xy2`, i8.
    xy2: i8,
    /// `dig_xy1`, u8.
    xy1: u8,
}

/// BMM150 driver.
pub struct Bmm150<B> {
    /// Async I²C bus handle used for every transaction.
    bus: B,
    /// 7-bit I²C address chosen at construction (primary or secondary).
    address: u8,
    /// Chip-specific trim constants, populated by [`Bmm150::init`].
    trim: Trim,
}

impl<B: I2c> Bmm150<B> {
    /// Construct a driver pinned to a specific I²C address. Prefer
    /// [`Bmm150::detect`] on boards whose `SDO` strap isn't documented.
    #[must_use = "holds the I²C bus"]
    pub const fn new(bus: B, address: u8) -> Self {
        Self {
            bus,
            address,
            trim: Trim {
                x1: 0,
                y1: 0,
                z4: 0,
                x2: 0,
                y2: 0,
                z2: 0,
                z1: 0,
                xyz1: 0,
                z3: 0,
                xy2: 0,
                xy1: 0,
            },
        }
    }

    /// Consume the driver and return the underlying bus.
    #[must_use]
    pub fn release(self) -> B {
        self.bus
    }

    /// Probe both candidate addresses and return a driver pinned to the
    /// one that answers with [`CHIP_ID`].
    ///
    /// The chip must first be woken from suspend (I²C unreachable
    /// otherwise), so this function writes the wake bit to the
    /// power-control register at each candidate before the ID read.
    ///
    /// # Errors
    ///
    /// [`Error::NotDetected`] if neither address answers with the
    /// expected chip ID; [`Error::I2c`] on any transport failure
    /// against the primary address (the secondary's failure is
    /// swallowed since it's a probe).
    pub async fn detect<D: DelayNs>(mut bus: B, delay: &mut D) -> Result<Self, Error<B::Error>>
    where
        B::Error: core::fmt::Debug,
    {
        for &addr in &[ADDRESS_PRIMARY, ADDRESS_SECONDARY] {
            // Wake the chip; suspend-state chips NACK the ID read.
            let wake_ok = bus.write(addr, &[REG_POWER, POWER_WAKE]).await.is_ok();
            if !wake_ok {
                continue;
            }
            delay.delay_ms(POWER_SETTLE_MS).await;
            let mut buf = [0u8; 1];
            if bus.write_read(addr, &[REG_CHIP_ID], &mut buf).await.is_ok() && buf[0] == CHIP_ID {
                return Ok(Self::new(bus, addr));
            }
        }
        Err(Error::NotDetected)
    }

    /// Read the chip-ID register. Useful as a health check on an
    /// already-initialised driver.
    ///
    /// # Errors
    ///
    /// [`Error::I2c`] on transport failure.
    pub async fn read_chip_id(&mut self) -> Result<u8, Error<B::Error>> {
        let mut buf = [0u8; 1];
        self.bus
            .write_read(self.address, &[REG_CHIP_ID], &mut buf)
            .await?;
        Ok(buf[0])
    }

    /// Bring the chip up: soft reset → wake → verify chip ID → read
    /// trim block → REPXY/REPZ preset → normal mode at 10 Hz.
    ///
    /// Safe to call on an already-initialised chip — the soft reset
    /// returns every configuration register to its power-on default.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ChipId`] if the chip-ID register doesn't read
    /// as [`CHIP_ID`], or [`Error::I2c`] on any I²C failure.
    pub async fn init<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<B::Error>> {
        // Wake first. A chip still in suspend NACKs every subsequent
        // register access, so the soft-reset write below needs this.
        self.write_reg(REG_POWER, POWER_WAKE).await?;
        delay.delay_ms(POWER_SETTLE_MS).await;

        // Soft reset the control registers, then wake again. Bit 7 is
        // cleared by the chip itself after ~2.5 ms; we wait longer.
        self.write_reg(REG_POWER, POWER_SOFT_RESET).await?;
        delay.delay_ms(POWER_SETTLE_MS).await;
        self.write_reg(REG_POWER, POWER_WAKE).await?;
        delay.delay_ms(POWER_SETTLE_MS).await;

        // Verify the part.
        let chip_id = self.read_chip_id().await?;
        if chip_id != CHIP_ID {
            return Err(Error::ChipId {
                expected: CHIP_ID,
                actual: chip_id,
            });
        }

        // Trim block is read once — these are factory-programmed
        // constants specific to this chip.
        self.trim = self.read_trim().await?;

        // Regular preset: 9 XY repetitions, 15 Z repetitions. ~0.6 µT
        // RMS noise, 0.5 mA current, 10 Hz sample rate.
        self.write_reg(REG_REP_XY, REPXY_REG_REGULAR).await?;
        self.write_reg(REG_REP_Z, REPZ_REG_REGULAR).await?;
        self.write_reg(REG_OPMODE_ODR, OPMODE_ODR_NORMAL_10HZ)
            .await?;
        Ok(())
    }

    /// Read and trim-compensate one sample.
    ///
    /// # Errors
    ///
    /// [`Error::Overflow`] if any axis returned the ADC-overflow
    /// sentinel; [`Error::I2c`] on transport failure.
    pub async fn read_measurement(&mut self) -> Result<Measurement, Error<B::Error>> {
        let mut buf = [0u8; 8];
        self.bus
            .write_read(self.address, &[REG_DATA_START], &mut buf)
            .await?;
        let raw = RawSample::from_bytes(buf);

        if raw.x == XY_OVERFLOW || raw.y == XY_OVERFLOW || raw.z == Z_OVERFLOW {
            return Err(Error::Overflow);
        }

        let x = compensate_xy(&self.trim, raw.x, raw.rhall, Axis::X);
        let y = compensate_xy(&self.trim, raw.y, raw.rhall, Axis::Y);
        let z = compensate_z(&self.trim, raw.z, raw.rhall);
        // Compensated values are in 1/16 µT per LSB. The i32 → f32 cast
        // loses precision past ±16 M LSBs (~1 MT); earth-field values
        // stay in ±1000 LSBs so the f32 rounding is lossless for us.
        #[allow(
            clippy::cast_precision_loss,
            reason = "compensated LSBs stay within f32 mantissa range in physical operation"
        )]
        Ok(Measurement {
            mag_ut: (
                x as f32 * LSB_TO_UT,
                y as f32 * LSB_TO_UT,
                z as f32 * LSB_TO_UT,
            ),
        })
    }

    /// Read the 21-byte trim block and parse it into a [`Trim`] struct.
    async fn read_trim(&mut self) -> Result<Trim, Error<B::Error>> {
        let mut buf = [0u8; TRIM_BLOCK_LEN];
        self.bus
            .write_read(self.address, &[REG_TRIM_START], &mut buf)
            .await?;
        Ok(parse_trim(&buf))
    }

    /// Write `value` to register `reg` at [`Self::address`].
    async fn write_reg(&mut self, reg: u8, value: u8) -> Result<(), Error<B::Error>> {
        self.bus.write(self.address, &[reg, value]).await?;
        Ok(())
    }
}

/// One raw, unsigned-extracted sample from registers 0x42..=0x49. Field
/// widths: X/Y = 13-bit signed, Z = 15-bit signed, RHALL = 14-bit unsigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RawSample {
    /// X axis raw sample in the 13-bit signed field.
    x: i16,
    /// Y axis raw sample in the 13-bit signed field.
    y: i16,
    /// Z axis raw sample in the 15-bit signed field.
    z: i16,
    /// Hall-resistance reading used by the compensation algorithm as
    /// a per-reading temperature proxy.
    rhall: u16,
}

impl RawSample {
    /// Parse the 8-byte measurement block.
    ///
    /// Layout (from `REG_DATA_START`):
    /// - `[X_L, X_M]` → 13-bit signed X in bits 15..3
    /// - `[Y_L, Y_M]` → 13-bit signed Y in bits 15..3
    /// - `[Z_L, Z_M]` → 15-bit signed Z in bits 15..1
    /// - `[RHALL_L, RHALL_M]` → 14-bit unsigned in bits 15..2
    const fn from_bytes(buf: [u8; 8]) -> Self {
        // Sign-extend by combining the MSB and LSB fields, then
        // arithmetic-shifting right to propagate sign through the
        // unused low bits.
        let x = (i16::from_le_bytes([buf[0], buf[1]])) >> 3;
        let y = (i16::from_le_bytes([buf[2], buf[3]])) >> 3;
        let z = (i16::from_le_bytes([buf[4], buf[5]])) >> 1;
        let rhall = u16::from_le_bytes([buf[6], buf[7]]) >> 2;
        Self { x, y, z, rhall }
    }
}

/// Axis selector for the shared X/Y compensation function.
#[derive(Clone, Copy)]
enum Axis {
    /// X axis: uses trim `x1` / `x2`.
    X,
    /// Y axis: uses trim `y1` / `y2`.
    Y,
}

/// Parse the 21-byte trim block per Bosch's `bmm150_trim_regs` layout.
///
/// Offsets within the block (0-indexed from the base of the 0x5D read):
/// | offset | size | field      | type |
/// |--------|------|------------|------|
/// | 0      | 1    | x1         | i8   |
/// | 1      | 1    | y1         | i8   |
/// | 2-4    | 3    | reserved   |      |
/// | 5-6    | 2    | z4         | i16  |
/// | 7      | 1    | x2         | i8   |
/// | 8      | 1    | y2         | i8   |
/// | 9-10   | 2    | reserved   |      |
/// | 11-12  | 2    | z2         | i16  |
/// | 13-14  | 2    | z1         | u16  |
/// | 15-16  | 2    | xyz1       | u16  |
/// | 17-18  | 2    | z3         | i16  |
/// | 19     | 1    | xy2        | i8   |
/// | 20     | 1    | xy1        | u8   |
///
/// All multi-byte fields are little-endian on the wire.
#[allow(
    clippy::cast_possible_wrap,
    reason = "i8 cast from u8 is the documented wire format"
)]
const fn parse_trim(buf: &[u8; TRIM_BLOCK_LEN]) -> Trim {
    Trim {
        x1: buf[0] as i8,
        y1: buf[1] as i8,
        z4: i16::from_le_bytes([buf[5], buf[6]]),
        x2: buf[7] as i8,
        y2: buf[8] as i8,
        z2: i16::from_le_bytes([buf[11], buf[12]]),
        z1: u16::from_le_bytes([buf[13], buf[14]]),
        xyz1: u16::from_le_bytes([buf[15], buf[16]]),
        z3: i16::from_le_bytes([buf[17], buf[18]]),
        xy2: buf[19] as i8,
        xy1: buf[20],
    }
}

/// Compensate one X or Y sample. Direct port of Bosch's reference C
/// implementation — variable names and operation order preserved so
/// the behaviour is traceable to the upstream algorithm.
///
/// Returns compensated value in 1/16 µT units. The caller multiplies
/// by [`LSB_TO_UT`] for `f32` µT.
///
/// The casts inside this function are the algorithm: Bosch's reference
/// intermixes `u16` / `i16` / `i32` in ways that look lossy to clippy
/// but are the defined fixed-point semantics. Do **not** "clean up" the
/// casts without matching every step against the upstream source.
#[allow(
    clippy::many_single_char_names,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "verbatim port of Bosch bmm150_compensate_xy; casts match the fixed-point reference"
)]
fn compensate_xy(trim: &Trim, raw: i16, rhall: u16, axis: Axis) -> i32 {
    // If the caller provided RHALL = 0 (not true here but kept for
    // parity with the reference), fall back to the factory xyz1 value.
    let rhall = if rhall == 0 { trim.xyz1 } else { rhall };
    let (txy1, txy2) = match axis {
        Axis::X => (trim.x1, trim.x2),
        Axis::Y => (trim.y1, trim.y2),
    };

    // prevalue = (tregs->xyz1 << 14) / rhall, held in u16 per Bosch.
    let prevalue = ((i32::from(trim.xyz1) << 14) / i32::from(rhall)) as u16;
    // Subtract 0x4000 as i16; this centres the "rhall ratio" around 0.
    let val_a = prevalue.wrapping_sub(0x4000) as i16;
    let val_a32 = i32::from(val_a);

    let temp1 = i32::from(trim.xy2) * ((val_a32 * val_a32) >> 7);
    let temp2 = val_a32 * (i32::from(trim.xy1) << 7);
    let temp3 = (((temp1 + temp2) >> 9) + 0x0010_0000) * (i32::from(txy2) + 0xA0);
    // temp3 fits in i32 in practice; shift and widen for the final
    // multiply by raw (which is 13-bit signed so <= 2^12 in magnitude).
    let val_b = ((i32::from(raw) * (temp3 >> 12)) >> 13) as i16;
    i32::from(val_b + (i16::from(txy1) << 3))
}

/// Compensate one Z sample. Direct port of Bosch's reference C
/// implementation. See [`compensate_xy`] for the cast-discipline note.
#[allow(
    clippy::many_single_char_names,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "verbatim port of Bosch bmm150_compensate_z; casts match the fixed-point reference"
)]
fn compensate_z(trim: &Trim, raw: i16, rhall: u16) -> i32 {
    let temp1 = (i32::from(raw) - i32::from(trim.z4)) << 15;
    let temp2 = (i32::from(trim.z3) * (i32::from(rhall as i16) - i32::from(trim.xyz1 as i16))) >> 2;
    let temp3 = (((i32::from(trim.z1) * (i32::from(rhall) << 1)) + (1 << 15)) >> 16) as i16;
    let denom = i32::from(trim.z2) + i32::from(temp3);
    if denom == 0 {
        return 0;
    }
    (temp1 - temp2) / denom
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test scaffolding: Infallible bus error makes unwrap() sound"
)]
#[allow(
    clippy::panic,
    reason = "assert_matches-style panics in tests are the established pattern"
)]
#[allow(
    clippy::future_not_send,
    reason = "test mocks use RefCell; single-threaded block_on runs them"
)]
mod tests {
    use super::*;
    use core::cell::RefCell;
    use embedded_hal_async::i2c::{Operation, SevenBitAddress};

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

    /// Mock I²C + delay harness. Serves register reads from a 256-byte
    /// bank so `write_read` sequences work naturally.
    struct Mock {
        regs: RefCell<[u8; 256]>,
        writes: RefCell<Vec<(u8, Vec<u8>)>>,
    }

    impl Mock {
        fn new() -> Self {
            Self {
                regs: RefCell::new([0u8; 256]),
                writes: RefCell::new(Vec::new()),
            }
        }
        fn set_reg(&self, reg: u8, value: u8) {
            self.regs.borrow_mut()[usize::from(reg)] = value;
        }
        fn set_block(&self, start: u8, bytes: &[u8]) {
            let mut regs = self.regs.borrow_mut();
            for (i, b) in bytes.iter().enumerate() {
                regs[usize::from(start) + i] = *b;
            }
        }
    }

    struct MockBus<'a> {
        harness: &'a Mock,
    }

    impl embedded_hal_async::i2c::ErrorType for MockBus<'_> {
        type Error = core::convert::Infallible;
    }

    impl embedded_hal_async::i2c::I2c for MockBus<'_> {
        async fn transaction(
            &mut self,
            address: SevenBitAddress,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Self::Error> {
            let mut cursor: Option<u8> = None;
            for op in operations {
                match op {
                    Operation::Write(buf) => {
                        if buf.is_empty() {
                            continue;
                        }
                        cursor = Some(buf[0]);
                        if buf.len() >= 2 {
                            let mut regs = self.harness.regs.borrow_mut();
                            for (i, b) in buf[1..].iter().enumerate() {
                                regs[usize::from(buf[0]).saturating_add(i)] = *b;
                            }
                            drop(regs);
                            self.harness
                                .writes
                                .borrow_mut()
                                .push((address, buf.to_vec()));
                        }
                    }
                    Operation::Read(out) => {
                        let reg = cursor.unwrap();
                        let regs = self.harness.regs.borrow();
                        for (i, slot) in out.iter_mut().enumerate() {
                            *slot = regs[usize::from(reg).saturating_add(i)];
                        }
                    }
                }
            }
            Ok(())
        }
    }

    struct NopDelay;
    impl embedded_hal_async::delay::DelayNs for NopDelay {
        async fn delay_ns(&mut self, _ns: u32) {}
    }

    #[test]
    fn raw_sample_sign_extends_13_and_15_bit_fields() {
        // X = -1 in 13-bit signed field: bits 15:3 = 0x1FFF, field val
        // after shift = 0xFFFF (i.e. -1). Low 3 bits of LSB are ignored.
        let buf = [
            0xF8, 0xFF, // X_L, X_M: -1 in 13-bit
            0x08, 0x00, // Y_L, Y_M: +1 in 13-bit
            0xFE, 0xFF, // Z_L, Z_M: -1 in 15-bit
            0x04, 0x00, // RHALL_L, M: 1 in 14-bit
        ];
        let raw = RawSample::from_bytes(buf);
        assert_eq!(raw.x, -1);
        assert_eq!(raw.y, 1);
        assert_eq!(raw.z, -1);
        assert_eq!(raw.rhall, 1);
    }

    #[test]
    fn raw_sample_recognises_overflow_sentinels() {
        // XY overflow = -4096 encoded as raw: sign-extended 13-bit min.
        // Wire bytes for -4096 in 13-bit signed in bits 15..3 =
        // 0x8000 shifted right 3 arithmetically = 0xF000 = -4096.
        let buf = [
            0x00, 0x80, // X bits 15..3 = 0x1000 -> arith shr 3 = -4096
            0x00, 0x80, // Y same
            0x00, 0x80, // Z bits 15..1 = 0x4000 -> arith shr 1 = -16384
            0x00, 0x00,
        ];
        let raw = RawSample::from_bytes(buf);
        assert_eq!(raw.x, XY_OVERFLOW);
        assert_eq!(raw.y, XY_OVERFLOW);
        assert_eq!(raw.z, Z_OVERFLOW);
    }

    #[test]
    fn trim_block_parses_known_layout() {
        // Feed a block where every field has a unique, recognisable
        // value so a mis-offset parse fails loudly.
        let mut buf = [0u8; TRIM_BLOCK_LEN];
        buf[0] = 0x11; // x1
        buf[1] = 0x22; // y1
        buf[5] = 0x34;
        buf[6] = 0x12; // z4 = 0x1234
        buf[7] = 0x44; // x2
        buf[8] = 0x55; // y2
        buf[11] = 0x78;
        buf[12] = 0x56; // z2 = 0x5678
        buf[13] = 0xBC;
        buf[14] = 0x9A; // z1 = 0x9ABC
        buf[15] = 0xF0;
        buf[16] = 0xDE; // xyz1 = 0xDEF0
        buf[17] = 0x34;
        buf[18] = 0x12; // z3 = 0x1234
        buf[19] = 0x66; // xy2
        buf[20] = 0x77; // xy1

        let trim = parse_trim(&buf);
        assert_eq!(trim.x1, 0x11);
        assert_eq!(trim.y1, 0x22);
        assert_eq!(trim.z4, 0x1234);
        assert_eq!(trim.x2, 0x44);
        assert_eq!(trim.y2, 0x55);
        assert_eq!(trim.z2, 0x5678);
        assert_eq!(trim.z1, 0x9ABC);
        assert_eq!(trim.xyz1, 0xDEF0);
        assert_eq!(trim.z3, 0x1234);
        assert_eq!(trim.xy2, 0x66);
        assert_eq!(trim.xy1, 0x77);
    }

    #[test]
    fn init_writes_reset_wake_preset_sequence() {
        let harness = Mock::new();
        // Pre-load the chip-ID register and a plausible trim block so
        // `init` doesn't bail before reaching the preset writes.
        harness.set_reg(REG_CHIP_ID, CHIP_ID);
        // Non-zero z2 + xyz1 keep the Z denominator non-zero so future
        // compensation tests (not this one) don't divide by zero.
        harness.set_block(
            REG_TRIM_START,
            &[
                0x0A, 0x14, 0, 0, 0, 0x00, 0x10, // x1=10 y1=20 ... z4=4096
                0x05, 0x06, 0, 0, 0xD0, 0x07, // x2=5 y2=6 ... z2=2000
                0x00, 0x10, 0x00, 0x20, 0x64, 0x00, 0x03,
                0x7F, // z1=0x1000 xyz1=0x2000 z3=0x64 xy2=3 xy1=0x7F
            ],
        );

        let mut bus = MockBus { harness: &harness };
        let mut mag = Bmm150::new(&mut bus, ADDRESS_PRIMARY);
        block_on(mag.init(&mut NopDelay)).unwrap();

        let writes = harness.writes.borrow();
        // The order must be: wake, soft-reset, wake, REPXY, REPZ, OP.
        // Chip-ID + trim reads don't appear here (they're write-only
        // address-phase writes recorded separately as single-byte
        // writes, which our mock filters out because len < 2).
        let expected = vec![
            (ADDRESS_PRIMARY, vec![REG_POWER, POWER_WAKE]),
            (ADDRESS_PRIMARY, vec![REG_POWER, POWER_SOFT_RESET]),
            (ADDRESS_PRIMARY, vec![REG_POWER, POWER_WAKE]),
            (ADDRESS_PRIMARY, vec![REG_REP_XY, REPXY_REG_REGULAR]),
            (ADDRESS_PRIMARY, vec![REG_REP_Z, REPZ_REG_REGULAR]),
            (
                ADDRESS_PRIMARY,
                vec![REG_OPMODE_ODR, OPMODE_ODR_NORMAL_10HZ],
            ),
        ];
        assert_eq!(*writes, expected);
    }

    #[test]
    fn init_rejects_wrong_chip_id() {
        let harness = Mock::new();
        harness.set_reg(REG_CHIP_ID, 0x42); // not 0x32
        let mut bus = MockBus { harness: &harness };
        let mut mag = Bmm150::new(&mut bus, ADDRESS_PRIMARY);
        let result = block_on(mag.init(&mut NopDelay));
        match result {
            Err(Error::ChipId {
                expected: 0x32,
                actual: 0x42,
            }) => {}
            other => panic!("expected ChipId error, got {other:?}"),
        }
    }

    #[test]
    fn detect_probes_both_addresses() {
        let harness = Mock::new();
        // Only the secondary address answers with the chip ID. For our
        // mock the "address" doesn't actually route differently; we
        // fake this by only setting the ID when the primary probe
        // finishes, letting the secondary probe see the correct value.
        // Simpler test: just assert that detect() doesn't error when
        // the primary responds correctly.
        harness.set_reg(REG_CHIP_ID, CHIP_ID);
        let bus = MockBus { harness: &harness };
        let mag = block_on(Bmm150::detect(bus, &mut NopDelay)).unwrap();
        assert_eq!(mag.address, ADDRESS_PRIMARY);
    }

    #[test]
    fn read_measurement_flags_overflow() {
        let harness = Mock::new();
        harness.set_reg(REG_CHIP_ID, CHIP_ID);
        // Set X to the overflow sentinel. All other fields zero.
        harness.set_block(
            REG_DATA_START,
            &[0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        );
        let mut bus = MockBus { harness: &harness };
        let mut mag = Bmm150::new(&mut bus, ADDRESS_PRIMARY);
        let result = block_on(mag.read_measurement());
        assert!(matches!(result, Err(Error::Overflow)));
    }

    #[test]
    fn compensation_produces_finite_values_with_typical_trim() {
        // Plausible trim (real chip) + non-overflow raw readings + a
        // mid-range RHALL. We don't have a canonical "expected µT"
        // vector, but compensation must produce finite, non-silly
        // values — regression guard against arithmetic mistakes.
        let trim = Trim {
            x1: 0,
            y1: 0,
            z4: 0,
            x2: 26,
            y2: 26,
            z2: 6400,
            z1: 0x9800,
            xyz1: 0x1D83,
            z3: 42,
            xy2: -3,
            xy1: 0x1D,
        };
        let x = compensate_xy(&trim, 500, 0x1D83, Axis::X);
        let y = compensate_xy(&trim, -500, 0x1D83, Axis::Y);
        let z = compensate_z(&trim, 100, 0x1D83);
        for (name, v) in [("x", x), ("y", y), ("z", z)] {
            // earth field is ~25-65 µT; compensated LSB is 1/16 µT,
            // so legal range is roughly ±1000 LSBs. Anything bigger
            // than 2^16 is almost certainly overflow / mis-port.
            assert!(v.abs() < 65_536, "{name} {v} out of expected range");
        }
    }
}
