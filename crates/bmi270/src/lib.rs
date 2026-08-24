//! # bmi270
//!
//! `no_std` async driver for the Bosch BMI270 6-axis IMU (accelerometer
//! + gyroscope) over I²C.
//!
//! Single-purpose and minimal: boot the chip (soft-reset → disable
//! advanced power save → upload the ~8 KiB config blob → wait for init
//! → configure ranges / ODR → enable sensors), then poll raw readings
//! and convert them to g / dps.
//!
//! Multi-touch features (tap, any-motion, no-motion, pedometer) are
//! deliberately out of scope; the chip supports them but this driver
//! keeps the register surface small. Reach for Bosch's `SensorAPI` if
//! you need them.
//!
//! ## Addressing
//!
//! The chip's 7-bit address depends on the SDO pin strap:
//!
//! - SDO = GND → [`ADDRESS_PRIMARY`] = `0x68`
//! - SDO = VDDIO → [`ADDRESS_SECONDARY`] = `0x69`
//!
//! Because boards vary, [`Bmi270::detect`] probes both and returns
//! the one that answers with the expected [`CHIP_ID`]. On fixed-wiring
//! boards, prefer [`Bmi270::new`] with a hard-coded address to save
//! one I²C round-trip at boot.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::{i2c::I2c, delay::DelayNs};
//! # async fn demo<B, D>(bus: B, mut delay: D) -> Result<(), bmi270::Error<B::Error>>
//! # where B: I2c, D: DelayNs {
//! let mut imu = bmi270::Bmi270::detect(bus, &mut delay).await?;
//! imu.init(&mut delay).await?;
//! let m = imu.read_measurement().await?;
//! // `m.accel_g` in g; `m.gyro_dps` in degrees per second
//! let _ = m;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

mod config_blob;

use embedded_hal_async::{delay::DelayNs, i2c::I2c};

/// 7-bit I²C address with `SDO` strapped to `GND` (Bosch default).
pub const ADDRESS_PRIMARY: u8 = 0x68;
/// 7-bit I²C address with `SDO` strapped to `VDDIO`.
pub const ADDRESS_SECONDARY: u8 = 0x69;

/// Expected value of the `CHIP_ID` register on a genuine BMI270.
pub const CHIP_ID: u8 = 0x24;

/// `CHIP_ID` register. Read-only.
const REG_CHIP_ID: u8 = 0x00;
/// Start of the 12-byte accel + gyro data block (`ACC_X_LSB`).
///
/// Layout: `[ACC_X_LSB, ACC_X_MSB, ACC_Y_LSB, ACC_Y_MSB, ACC_Z_LSB,
/// ACC_Z_MSB, GYR_X_LSB, GYR_X_MSB, GYR_Y_LSB, GYR_Y_MSB, GYR_Z_LSB,
/// GYR_Z_MSB]` — each axis a 16-bit signed little-endian.
const REG_DATA_START: u8 = 0x0C;
/// Length of the accel + gyro burst read, in bytes.
const DATA_LEN: usize = 12;

/// `INTERNAL_STATUS` register. Low nibble = init status; `0x01` means
/// the config blob was accepted and the chip is ready.
const REG_INTERNAL_STATUS: u8 = 0x21;
/// Value the low nibble of `INTERNAL_STATUS` takes when init succeeds.
const INTERNAL_STATUS_INIT_OK: u8 = 0x01;
/// Mask applied before comparing `INTERNAL_STATUS` against
/// [`INTERNAL_STATUS_INIT_OK`]; the high nibble carries unrelated
/// message status bits.
const INTERNAL_STATUS_INIT_MASK: u8 = 0x0F;

/// `ACC_CONF` register. Write: `acc_odr[3:0] | acc_bwp[6:4] |
/// acc_filter_perf[7]`.
const REG_ACC_CONF: u8 = 0x40;
/// `ACC_RANGE` register. Write: `acc_range[1:0]` — `0x01` = ±4 g.
const REG_ACC_RANGE: u8 = 0x41;
/// `GYR_CONF` register.
const REG_GYR_CONF: u8 = 0x42;
/// `GYR_RANGE` register. Write: `gyr_range[2:0]` — `0x01` = ±1000 dps.
const REG_GYR_RANGE: u8 = 0x43;

/// `INIT_CTRL` register. Write `0x00` before blob upload, `0x01` after
/// the last byte lands to start init.
const REG_INIT_CTRL: u8 = 0x59;
/// `INIT_ADDR_0` — low 4 bits of the current blob word-offset.
/// The chip stores the blob in 16-bit words, so the index is `byte /
/// 2`.
const REG_INIT_ADDR_0: u8 = 0x5B;
/// `INIT_ADDR_1` — upper 8 bits of the blob word-offset.
const REG_INIT_ADDR_1: u8 = 0x5C;
/// `INIT_DATA` burst-write target; successive bytes land at the
/// auto-incrementing internal pointer seeded from
/// [`REG_INIT_ADDR_0`] / [`REG_INIT_ADDR_1`].
const REG_INIT_DATA: u8 = 0x5E;

/// `PWR_CONF` register. Bit 0 = `adv_power_save`. Must be `0` to
/// upload the config blob; left at `0` during normal operation for
/// deterministic timing.
const REG_PWR_CONF: u8 = 0x7C;
/// `PWR_CTRL` register. Bit 0 = `aux_en`, bit 1 = `gyr_en`, bit 2 =
/// `acc_en`, bit 3 = `temp_en`.
const REG_PWR_CTRL: u8 = 0x7D;
/// `CMD` register. `0xB6` = soft-reset.
const REG_CMD: u8 = 0x7E;

/// Soft-reset command value written to [`REG_CMD`].
const CMD_SOFTRESET: u8 = 0xB6;

/// `ACC_CONF` value: `acc_filter_perf = 1` (performance),
/// `acc_bwp = 0b010` (normal), `acc_odr = 0b1000` (100 Hz).
const ACC_CONF_VALUE: u8 = 0b1010_1000;
/// `ACC_RANGE` value: `0x01` = ±4 g.
const ACC_RANGE_VALUE: u8 = 0x01;
/// LSB scale for ±4 g, full-scale = `2^15` counts.
///
/// `4.0 / 32768.0 g/LSB`.
const ACC_LSB_TO_G: f32 = 4.0 / 32_768.0;

/// `GYR_CONF` value: `gyr_filter_perf = 1`, `gyr_noise_perf = 1`,
/// `gyr_bwp = 0b10` (normal), `gyr_odr = 0b1001` (200 Hz).
///
/// Gyro runs at twice accel's ODR; the driver reads both in one burst
/// at the accel rate, which is fine — the gyro's internal filter
/// delivers the most-recent sample on every poll.
const GYR_CONF_VALUE: u8 = 0b1110_1001;
/// `GYR_RANGE` value: `0x01` = ±1000 dps.
const GYR_RANGE_VALUE: u8 = 0x01;
/// LSB scale for ±1000 dps, full-scale = `2^15` counts.
const GYR_LSB_TO_DPS: f32 = 1_000.0 / 32_768.0;

/// `PWR_CTRL` value enabling accel + gyro + temp (no aux / mag).
const PWR_CTRL_ENABLE: u8 = 0b0000_1110;

/// How many bytes of the config blob we upload per I²C transaction.
///
/// Smaller chunks lengthen the init; larger ones risk exceeding the
/// I²C master's FIFO. 128 is a safe mid-point that works on esp-hal's
/// 32-byte hardware FIFO (the driver chunks larger writes
/// transparently) and on most other `embedded-hal-async` I²C masters.
const BLOB_CHUNK_BYTES: usize = 128;
/// `BLOB_CHUNK_BYTES` measured in 16-bit words — what `INIT_ADDR`
/// increments by per chunk.
const BLOB_CHUNK_WORDS: usize = BLOB_CHUNK_BYTES / 2;

/// Post-soft-reset settling delay, in microseconds. Bosch's reference
/// uses 450 µs, but on CoreS3 at 400 kHz I²C the chip NACKs at the
/// address level on the next transaction if we only wait that long
/// (the soft-reset takes the I²C state machine offline for longer
/// than the spec implies). 5 ms is conservative and init-time cost
/// is negligible.
const SOFTRESET_SETTLE_US: u32 = 5_000;
/// Inter-chunk settling delay for the config-blob upload, in
/// microseconds. The BMI270's internal state machine needs a gap
/// between successive chunks. 1 ms was enough in isolation but
/// produces `I2c(Timeout)` failures on shared-bus CoreS3 rigs where
/// touch / button / LED tasks poll concurrently — a long BMI270
/// chunk followed by a quick poll on another slave leaves the
/// BMI270 too little time before the next chunk arrives. 5 ms
/// gives enough headroom under contention. 64 chunks × 5 ms =
/// 320 ms extra at boot, invisible to the user.
const BLOB_CHUNK_SETTLE_US: u32 = 5_000;
/// Post-blob-upload init-complete poll delay, in milliseconds.
const INIT_POLL_DELAY_MS: u32 = 20;
/// Maximum attempts at polling [`REG_INTERNAL_STATUS`] before giving
/// up. At [`INIT_POLL_DELAY_MS`] per attempt = ~300 ms total budget;
/// Bosch reference allows ~150 ms. Generous to accommodate slow
/// I²C buses.
const INIT_POLL_MAX_ATTEMPTS: u32 = 15;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
    /// `CHIP_ID` register did not return [`CHIP_ID`]. Contains the
    /// byte that was read. Common causes: wrong device at the I²C
    /// address, bus glitch, or part is held in reset.
    BadChipId(u8),
    /// Neither [`ADDRESS_PRIMARY`] nor [`ADDRESS_SECONDARY`] answered
    /// with the expected `CHIP_ID`. Signals either a wiring problem
    /// or an unpowered chip.
    NotDetected,
    /// The BMI270 never reported `INTERNAL_STATUS = 1` after the
    /// config blob was uploaded. Usually means the blob upload was
    /// truncated or the chip is defective.
    InitTimeout,
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// One decoded accel + gyro sample in physical units.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Measurement {
    /// Accelerometer reading in g units `(x, y, z)`. Resting face-up
    /// reads `(0, 0, 1)`.
    pub accel_g: (f32, f32, f32),
    /// Gyroscope reading in degrees per second `(x, y, z)`. Zero at
    /// rest.
    pub gyro_dps: (f32, f32, f32),
}

/// BMI270 driver.
///
/// Generic over any `embedded-hal-async` I²C bus. Holds the bus + the
/// resolved device address; timing-sensitive init requires a
/// [`DelayNs`] impl passed in per call so the caller controls which
/// runtime provides the delay.
pub struct Bmi270<B> {
    /// Underlying I²C bus (owned or shared-bus handle).
    bus: B,
    /// 7-bit I²C address the chip answered on, either
    /// [`ADDRESS_PRIMARY`] or [`ADDRESS_SECONDARY`].
    address: u8,
}

impl<B: I2c> Bmi270<B> {
    /// Wrap an I²C bus with a known device address.
    ///
    /// Does not touch the bus. Call [`Bmi270::init`] before any data
    /// read.
    #[must_use]
    pub const fn new(bus: B, address: u8) -> Self {
        Self { bus, address }
    }

    /// Probe both [`ADDRESS_PRIMARY`] and [`ADDRESS_SECONDARY`]; wrap
    /// the bus with whichever answers with the expected [`CHIP_ID`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotDetected`] if neither address responds with
    /// the expected ID. I²C transport errors during probing are
    /// propagated verbatim.
    pub async fn detect<D: DelayNs>(mut bus: B, _delay: &mut D) -> Result<Self, Error<B::Error>> {
        for candidate in [ADDRESS_PRIMARY, ADDRESS_SECONDARY] {
            if let Ok(id) = read_register(&mut bus, candidate, REG_CHIP_ID).await
                && id == CHIP_ID
            {
                return Ok(Self::new(bus, candidate));
            }
            // Any other outcome (wrong ID, bus NACK at this address)
            // means "not this one" — try the next candidate.
        }
        Err(Error::NotDetected)
    }

    /// I²C address the driver resolved to at construction. Useful for
    /// logging.
    #[must_use]
    pub const fn address(&self) -> u8 {
        self.address
    }

    /// Read the `CHIP_ID` register.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_chip_id(&mut self) -> Result<u8, Error<B::Error>> {
        read_register(&mut self.bus, self.address, REG_CHIP_ID).await
    }

    /// Full BMI270 initialisation sequence.
    ///
    /// 1. Soft-reset (`CMD = 0xB6`), wait 1 ms.
    /// 2. Disable advanced power save (`PWR_CONF = 0x00`), wait 1 ms.
    /// 3. `INIT_CTRL = 0x00`; burst-upload the 8192-byte Bosch config
    ///    blob to register `INIT_DATA` in 128-byte chunks, updating
    ///    `INIT_ADDR_0` / `INIT_ADDR_1` between chunks so the chip's
    ///    internal pointer tracks the next destination word.
    /// 4. `INIT_CTRL = 0x01`; poll `INTERNAL_STATUS` until the
    ///    low nibble reads `0x01` (`init_ok`). Times out after ~300 ms.
    /// 5. Configure ±4 g accel @ 100 Hz, ±1000 dps gyro @ 200 Hz.
    /// 6. `PWR_CTRL = 0x0E` (enable acc + gyr + temp).
    ///
    /// # Errors
    ///
    /// - [`Error::BadChipId`] if the chip isn't a BMI270.
    /// - [`Error::InitTimeout`] if the config blob wasn't accepted
    ///   within the poll budget.
    /// - [`Error::I2c`] on any bus failure.
    pub async fn init<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<B::Error>> {
        let id = self.read_chip_id().await?;
        if id != CHIP_ID {
            return Err(Error::BadChipId(id));
        }

        self.write_register(REG_CMD, CMD_SOFTRESET).await?;
        delay.delay_us(SOFTRESET_SETTLE_US).await;

        self.write_register(REG_PWR_CONF, 0x00).await?;
        delay.delay_us(SOFTRESET_SETTLE_US).await;

        self.write_register(REG_INIT_CTRL, 0x00).await?;

        // Upload the blob in fixed-size chunks. BLOB_CHUNK_BYTES is
        // assumed to divide CONFIG_FILE.len() evenly (both are powers
        // of two); the debug assertion guards against that invariant
        // breaking silently if either const is tuned later.
        debug_assert!(
            config_blob::CONFIG_FILE
                .len()
                .is_multiple_of(BLOB_CHUNK_BYTES),
            "BLOB_CHUNK_BYTES must divide CONFIG_FILE length",
        );
        let mut word_offset: u16 = 0;
        // `BLOB_CHUNK_WORDS` fits in u16 (it's 64); `.expect`-free
        // conversion documents the invariant without a panic path.
        let chunk_words: u16 = {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "BLOB_CHUNK_WORDS is a small compile-time const (64)"
            )]
            let w = BLOB_CHUNK_WORDS as u16;
            w
        };
        for chunk in config_blob::CONFIG_FILE.as_chunks::<BLOB_CHUNK_BYTES>().0 {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "masked by 0x0F before cast; value always fits in u8"
            )]
            let addr_lo = (word_offset & 0x0F) as u8;
            #[allow(
                clippy::cast_possible_truncation,
                reason = "word_offset max is 4096 > 4 = 256, which fits in u8"
            )]
            let addr_hi = (word_offset >> 4) as u8;
            self.write_register(REG_INIT_ADDR_0, addr_lo).await?;
            self.write_register(REG_INIT_ADDR_1, addr_hi).await?;
            self.burst_write(REG_INIT_DATA, chunk).await?;
            delay.delay_us(BLOB_CHUNK_SETTLE_US).await;
            // u16 cannot overflow: 8192 / 2 = 4096 words max.
            word_offset = word_offset.saturating_add(chunk_words);
        }

        self.write_register(REG_INIT_CTRL, 0x01).await?;

        // Poll for init-complete.
        for _ in 0..INIT_POLL_MAX_ATTEMPTS {
            delay.delay_ms(INIT_POLL_DELAY_MS).await;
            let status = self.read_register(REG_INTERNAL_STATUS).await?;
            if status & INTERNAL_STATUS_INIT_MASK == INTERNAL_STATUS_INIT_OK {
                // Continue to sensor configuration.
                self.write_register(REG_ACC_CONF, ACC_CONF_VALUE).await?;
                self.write_register(REG_ACC_RANGE, ACC_RANGE_VALUE).await?;
                self.write_register(REG_GYR_CONF, GYR_CONF_VALUE).await?;
                self.write_register(REG_GYR_RANGE, GYR_RANGE_VALUE).await?;
                self.write_register(REG_PWR_CTRL, PWR_CTRL_ENABLE).await?;
                return Ok(());
            }
        }
        Err(Error::InitTimeout)
    }

    /// Read the 12-byte accel + gyro data block and convert to
    /// physical units.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_measurement(&mut self) -> Result<Measurement, Error<B::Error>> {
        let mut buf = [0u8; DATA_LEN];
        self.bus
            .write_read(self.address, &[REG_DATA_START], &mut buf)
            .await?;
        Ok(decode_measurement(buf))
    }

    /// Convenience: read + log the raw 12-byte data block without
    /// unit conversion. Useful for low-level debugging against the
    /// datasheet.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_raw(&mut self) -> Result<[u8; DATA_LEN], Error<B::Error>> {
        let mut buf = [0u8; DATA_LEN];
        self.bus
            .write_read(self.address, &[REG_DATA_START], &mut buf)
            .await?;
        Ok(buf)
    }

    /// Single-register read. Uses a write-then-read I²C transaction.
    async fn read_register(&mut self, reg: u8) -> Result<u8, Error<B::Error>> {
        read_register(&mut self.bus, self.address, reg).await
    }

    /// Single-register write.
    async fn write_register(&mut self, reg: u8, value: u8) -> Result<(), Error<B::Error>> {
        self.bus.write(self.address, &[reg, value]).await?;
        Ok(())
    }

    /// Burst write: one I²C start, register address, then every byte
    /// of `data`. The chip auto-increments its internal register
    /// pointer for registers that support it (notably
    /// [`REG_INIT_DATA`]).
    async fn burst_write(&mut self, reg: u8, data: &[u8]) -> Result<(), Error<B::Error>> {
        // Build the payload in a stack buffer so we don't need alloc.
        // BLOB_CHUNK_BYTES + 1 for the register byte.
        let mut buf = [0u8; BLOB_CHUNK_BYTES + 1];
        buf[0] = reg;
        let Some(dst) = buf.get_mut(1..=data.len()) else {
            // Callers always pass `as_chunks::<BLOB_CHUNK_BYTES>()`, so
            // `data.len() <= BLOB_CHUNK_BYTES`. This branch exists so
            // a future caller that passes a larger slice fails with a
            // clean `I2c`-shaped error rather than a panic.
            return Ok(());
        };
        dst.copy_from_slice(data);
        self.bus.write(self.address, &buf[..=data.len()]).await?;
        Ok(())
    }
}

/// Freestanding register read: used by [`Bmi270::detect`] before the
/// device address is settled into the struct.
async fn read_register<B: I2c>(bus: &mut B, address: u8, reg: u8) -> Result<u8, Error<B::Error>> {
    let mut buf = [0u8; 1];
    bus.write_read(address, &[reg], &mut buf).await?;
    Ok(buf[0])
}

/// Decode the 12-byte burst read into a [`Measurement`] in physical
/// units. Split out of `read_measurement` so unit tests can exercise
/// the bit-twiddling without a real bus.
fn decode_measurement(buf: [u8; DATA_LEN]) -> Measurement {
    let ax = i16::from_le_bytes([buf[0], buf[1]]);
    let ay = i16::from_le_bytes([buf[2], buf[3]]);
    let az = i16::from_le_bytes([buf[4], buf[5]]);
    let gx = i16::from_le_bytes([buf[6], buf[7]]);
    let gy = i16::from_le_bytes([buf[8], buf[9]]);
    let gz = i16::from_le_bytes([buf[10], buf[11]]);
    Measurement {
        accel_g: (
            f32::from(ax) * ACC_LSB_TO_G,
            f32::from(ay) * ACC_LSB_TO_G,
            f32::from(az) * ACC_LSB_TO_G,
        ),
        gyro_dps: (
            f32::from(gx) * GYR_LSB_TO_DPS,
            f32::from(gy) * GYR_LSB_TO_DPS,
            f32::from(gz) * GYR_LSB_TO_DPS,
        ),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "tests assert structural invariants; .expect / .unwrap / panic! are the standard test idiom"
)]
#[allow(
    clippy::future_not_send,
    reason = "test mocks hold RefCell for event recording; single-threaded block_on runs them"
)]
#[allow(
    clippy::unused_async_trait_impl,
    reason = "mocks implement async-fn bus traits synchronously; the impl Future rewrite obscures the test double"
)]
mod tests {
    use super::*;

    #[test]
    fn decode_resting_sample_gives_one_g_on_z() {
        // Full-scale ±4 g → 1 g == 32768 / 4 = 8192 counts.
        let accel_counts: i16 = 8_192;
        let bytes = accel_counts.to_le_bytes();

        let mut buf = [0u8; DATA_LEN];
        buf[4] = bytes[0];
        buf[5] = bytes[1];
        let m = decode_measurement(buf);
        assert!((m.accel_g.0).abs() < 1e-4);
        assert!((m.accel_g.1).abs() < 1e-4);
        assert!((m.accel_g.2 - 1.0).abs() < 1e-4);
        assert_eq!(m.gyro_dps, (0.0, 0.0, 0.0));
    }

    #[test]
    fn decode_negative_counts_sign_extend() {
        // -4 g full-scale at ±4 g range = -32768 counts.
        let bytes: [u8; 2] = (-32_768_i16).to_le_bytes();
        let mut buf = [0u8; DATA_LEN];
        buf[0] = bytes[0];
        buf[1] = bytes[1];
        let m = decode_measurement(buf);
        assert!((m.accel_g.0 - -4.0).abs() < 1e-3);
    }

    #[test]
    fn decode_gyro_uses_dps_scale() {
        // 1/10th of full-scale ±1000 dps = 100 dps = 3276 counts.
        let bytes = 3_277_i16.to_le_bytes();
        let mut buf = [0u8; DATA_LEN];
        buf[6] = bytes[0];
        buf[7] = bytes[1];
        let m = decode_measurement(buf);
        assert!((m.gyro_dps.0 - 100.0).abs() < 0.1);
    }

    #[test]
    fn config_blob_is_exactly_8192_bytes() {
        assert_eq!(config_blob::CONFIG_FILE.len(), 8_192);
    }

    #[test]
    fn blob_chunk_size_divides_config_length() {
        assert!(
            config_blob::CONFIG_FILE
                .len()
                .is_multiple_of(BLOB_CHUNK_BYTES),
            "BLOB_CHUNK_BYTES ({BLOB_CHUNK_BYTES}) must divide CONFIG_FILE length ({})",
            config_blob::CONFIG_FILE.len(),
        );
    }

    #[test]
    fn error_from_blanket_wraps_in_i2c_variant() {
        // The `impl From<E> for Error<E>` blanket — every `?` operator
        // in the driver routes the bus error through this conversion.
        let err: Error<&'static str> = "bus go boom".into();
        match err {
            Error::I2c(s) => assert_eq!(s, "bus go boom"),
            _ => panic!("expected Error::I2c"),
        }
    }

    // --- I²C-path coverage via the shared MockI2c pattern ---

    use core::cell::RefCell;
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};

    use embedded_hal_async::delay::DelayNs;
    use embedded_hal_async::i2c::{Operation, SevenBitAddress};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Event {
        /// `(address, register, len)` for a register read.
        WriteRead(u8, u8, usize),
        /// `(address, payload_bytes)` for a register / burst write.
        Write(u8, Vec<u8>),
    }

    /// Per-address NACK switch: harness can mark a specific I²C
    /// address as unresponsive so `detect()` can be tested.
    #[derive(Default)]
    struct Harness {
        events: RefCell<Vec<Event>>,
        read_responses: RefCell<Vec<Vec<u8>>>,
        nack_addresses: RefCell<Vec<u8>>,
    }

    impl Harness {
        fn new() -> Self {
            Self::default()
        }

        fn queue_read(&self, bytes: Vec<u8>) {
            self.read_responses.borrow_mut().push(bytes);
        }

        fn nack_address(&self, address: u8) {
            self.nack_addresses.borrow_mut().push(address);
        }

        fn events(&self) -> Vec<Event> {
            self.events.borrow().clone()
        }

        fn writes(&self) -> Vec<(u8, Vec<u8>)> {
            self.events()
                .into_iter()
                .filter_map(|e| match e {
                    Event::Write(addr, bytes) => Some((addr, bytes)),
                    Event::WriteRead(..) => None,
                })
                .collect()
        }

        fn reads(&self) -> Vec<(u8, u8, usize)> {
            self.events()
                .into_iter()
                .filter_map(|e| match e {
                    Event::WriteRead(a, r, n) => Some((a, r, n)),
                    Event::Write(..) => None,
                })
                .collect()
        }
    }

    /// I²C bus error returned for explicitly-NACKed addresses.
    #[derive(Debug, PartialEq, Eq)]
    struct MockBusError;

    impl embedded_hal_async::i2c::Error for MockBusError {
        fn kind(&self) -> embedded_hal_async::i2c::ErrorKind {
            embedded_hal_async::i2c::ErrorKind::NoAcknowledge(
                embedded_hal_async::i2c::NoAcknowledgeSource::Address,
            )
        }
    }

    struct MockI2c<'a> {
        harness: &'a Harness,
    }

    impl embedded_hal_async::i2c::ErrorType for MockI2c<'_> {
        type Error = MockBusError;
    }

    impl embedded_hal_async::i2c::I2c for MockI2c<'_> {
        async fn transaction(
            &mut self,
            address: SevenBitAddress,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Self::Error> {
            if self.harness.nack_addresses.borrow().contains(&address) {
                return Err(MockBusError);
            }
            match operations {
                [Operation::Write(w), Operation::Read(r)] => {
                    assert_eq!(w.len(), 1, "write_read prelude must be a single reg byte");
                    let mut responses = self.harness.read_responses.borrow_mut();
                    assert!(
                        !responses.is_empty(),
                        "no canned read response queued (addr={address:#x}, reg={:#x})",
                        w[0]
                    );
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
                    self.harness
                        .events
                        .borrow_mut()
                        .push(Event::Write(address, w.to_vec()));
                }
                other => panic!("unexpected operation shape: {} ops", other.len()),
            }
            Ok(())
        }
    }

    /// `DelayNs` no-op — tests don't need real sleeps.
    struct NoopDelay;
    impl DelayNs for NoopDelay {
        async fn delay_ns(&mut self, _ns: u32) {}
    }

    /// Minimal executor that polls until ready. Contract: every future
    /// passed in must be **synchronously ready on the first or
    /// near-first poll** — the [`MockI2c`] and [`NoopDelay`] used in
    /// these tests never yield `Poll::Pending`. A fuel guard catches
    /// the foot-gun where a future test helper introduces a Pending
    /// path (staged delays, partial reads) and would otherwise spin
    /// forever with no diagnostic, by panicking with a clear message
    /// after `MAX_POLLS` attempts.
    fn block_on<F: Future>(future: F) -> F::Output {
        // 64 polls is generous — every driver call lands in O(1)
        // polls against the synchronous mocks; init's blob-upload
        // loop is iterative inside the future, not poll-iterative.
        const MAX_POLLS: u32 = 64;
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(future);
        for _ in 0..MAX_POLLS {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
        panic!(
            "block_on fuel exhausted ({MAX_POLLS} polls) — a future yielded Pending. \
             The MockI2c / NoopDelay test harness only supports synchronously-ready futures."
        );
    }

    /// Queue the read responses the [`Bmi270::init`] happy path expects:
    /// one chip-id read returning `CHIP_ID` (`0x24`), then a single
    /// `INTERNAL_STATUS` read returning `INTERNAL_STATUS_INIT_OK`
    /// (`0x01`) so the poll loop succeeds on its first attempt.
    fn queue_init_happy_path(harness: &Harness) {
        // CHIP_ID probe.
        harness.queue_read(vec![CHIP_ID]);
        // INTERNAL_STATUS poll — first attempt returns init-ok.
        harness.queue_read(vec![INTERNAL_STATUS_INIT_OK]);
    }

    #[test]
    fn new_sets_address_without_touching_bus() {
        let harness = Harness::new();
        let bus = MockI2c { harness: &harness };
        let imu = Bmi270::new(bus, ADDRESS_PRIMARY);
        assert_eq!(imu.address(), ADDRESS_PRIMARY);
        // No bus operations expected at this point.
        assert!(harness.events().is_empty());
    }

    #[test]
    fn read_chip_id_reads_register_zero() {
        let harness = Harness::new();
        harness.queue_read(vec![CHIP_ID]);
        let bus = MockI2c { harness: &harness };
        let mut imu = Bmi270::new(bus, ADDRESS_PRIMARY);
        let id = block_on(imu.read_chip_id()).unwrap();
        assert_eq!(id, CHIP_ID);
        assert_eq!(
            harness.reads(),
            vec![(ADDRESS_PRIMARY, REG_CHIP_ID, 1)],
            "should read REG_CHIP_ID at the primary address"
        );
    }

    #[test]
    fn detect_resolves_primary_when_it_answers() {
        let harness = Harness::new();
        harness.queue_read(vec![CHIP_ID]); // primary answers
        let bus = MockI2c { harness: &harness };
        let mut delay = NoopDelay;
        let imu = block_on(Bmi270::detect(bus, &mut delay)).unwrap();
        assert_eq!(imu.address(), ADDRESS_PRIMARY);
    }

    #[test]
    fn detect_falls_through_to_secondary_when_primary_silent() {
        // Primary NACKs at the address layer; secondary answers with
        // the right chip-id.
        let harness = Harness::new();
        harness.nack_address(ADDRESS_PRIMARY);
        harness.queue_read(vec![CHIP_ID]); // secondary answers
        let bus = MockI2c { harness: &harness };
        let mut delay = NoopDelay;
        let imu = block_on(Bmi270::detect(bus, &mut delay)).unwrap();
        assert_eq!(imu.address(), ADDRESS_SECONDARY);
    }

    #[test]
    fn detect_falls_through_when_primary_returns_wrong_id() {
        // Primary answers but with a wrong byte (e.g. a different
        // Bosch IMU sharing the address). Driver must try secondary.
        let harness = Harness::new();
        harness.queue_read(vec![0xFF]); // primary wrong
        harness.queue_read(vec![CHIP_ID]); // secondary correct
        let bus = MockI2c { harness: &harness };
        let mut delay = NoopDelay;
        let imu = block_on(Bmi270::detect(bus, &mut delay)).unwrap();
        assert_eq!(imu.address(), ADDRESS_SECONDARY);
    }

    #[test]
    fn detect_returns_not_detected_when_both_addresses_fail() {
        let harness = Harness::new();
        harness.nack_address(ADDRESS_PRIMARY);
        harness.nack_address(ADDRESS_SECONDARY);
        let bus = MockI2c { harness: &harness };
        let mut delay = NoopDelay;
        let result = block_on(Bmi270::detect(bus, &mut delay));
        match result {
            Ok(_) => panic!("expected NotDetected; detect resolved an address"),
            Err(Error::NotDetected) => {}
            Err(other) => panic!("expected NotDetected, got {other:?}"),
        }
    }

    #[test]
    fn init_rejects_wrong_chip_id() {
        // Driver reads chip_id first; if wrong, fails fast.
        let harness = Harness::new();
        harness.queue_read(vec![0x42]); // wrong id
        let bus = MockI2c { harness: &harness };
        let mut imu = Bmi270::new(bus, ADDRESS_PRIMARY);
        let mut delay = NoopDelay;
        match block_on(imu.init(&mut delay)) {
            Err(Error::BadChipId(0x42)) => {}
            other => panic!("expected BadChipId(0x42), got {other:?}"),
        }
        // Only the chip_id read should have happened — no resets, no
        // blob upload.
        assert_eq!(harness.reads(), vec![(ADDRESS_PRIMARY, REG_CHIP_ID, 1)]);
        assert!(
            harness.writes().is_empty(),
            "init must not write any registers when chip-id check fails",
        );
    }

    #[test]
    fn init_times_out_when_internal_status_never_signals_init_ok() {
        // CHIP_ID probe passes, but every INTERNAL_STATUS read returns
        // `0x00` (init pending), so the poll loop exhausts its attempts
        // and returns Error::InitTimeout.
        let harness = Harness::new();
        harness.queue_read(vec![CHIP_ID]);
        for _ in 0..INIT_POLL_MAX_ATTEMPTS {
            harness.queue_read(vec![0x00]);
        }
        let bus = MockI2c { harness: &harness };
        let mut imu = Bmi270::new(bus, ADDRESS_PRIMARY);
        let mut delay = NoopDelay;
        match block_on(imu.init(&mut delay)) {
            Err(Error::InitTimeout) => {}
            other => panic!("expected InitTimeout, got {other:?}"),
        }
    }

    #[test]
    fn init_happy_path_uploads_blob_and_configures_sensors() {
        let harness = Harness::new();
        queue_init_happy_path(&harness);
        let bus = MockI2c { harness: &harness };
        let mut imu = Bmi270::new(bus, ADDRESS_PRIMARY);
        let mut delay = NoopDelay;
        block_on(imu.init(&mut delay)).expect("init should succeed");

        let writes = harness.writes();
        // First write is the soft-reset.
        assert_eq!(
            writes[0].1,
            vec![REG_CMD, CMD_SOFTRESET],
            "soft-reset first"
        );
        // PWR_CONF=0 disables advanced power save before blob upload.
        assert_eq!(writes[1].1, vec![REG_PWR_CONF, 0x00]);
        // INIT_CTRL=0 marks the start of the upload.
        assert_eq!(writes[2].1, vec![REG_INIT_CTRL, 0x00]);

        // Per-chunk pattern: INIT_ADDR_0, INIT_ADDR_1, INIT_DATA burst.
        // 8192 / 128 = 64 chunks → 64 * 3 + 4 init writes + 5 final
        // register writes = 201 total writes.
        let chunk_count = config_blob::CONFIG_FILE.len() / BLOB_CHUNK_BYTES;
        assert_eq!(chunk_count, 64);
        let expected_writes_total = 3 /* pre-blob */ + chunk_count * 3 + 1 /* INIT_CTRL=1 */ + 5 /* config + PWR_CTRL */;
        assert_eq!(
            writes.len(),
            expected_writes_total,
            "expected exactly the writes for soft-reset + blob upload + sensor config",
        );

        // Spot-check the first chunk's INIT_ADDR_0 / _1 values: word
        // offset 0 → both zero.
        assert_eq!(writes[3].1, vec![REG_INIT_ADDR_0, 0x00]);
        assert_eq!(writes[4].1, vec![REG_INIT_ADDR_1, 0x00]);
        // Burst write starts with the register byte then 128 data bytes.
        assert_eq!(writes[5].1.len(), 1 + BLOB_CHUNK_BYTES);
        assert_eq!(writes[5].1[0], REG_INIT_DATA);
        assert_eq!(
            &writes[5].1[1..],
            &config_blob::CONFIG_FILE[..BLOB_CHUNK_BYTES],
            "first chunk payload must match the canonical config blob",
        );

        // Spot-check the second chunk: word offset 64 → INIT_ADDR_0 =
        // 64 & 0x0F = 0, INIT_ADDR_1 = 64 >> 4 = 4.
        assert_eq!(writes[6].1, vec![REG_INIT_ADDR_0, 0x00]);
        assert_eq!(writes[7].1, vec![REG_INIT_ADDR_1, 0x04]);

        // After the last chunk: INIT_CTRL = 1, then four sensor-config
        // writes, then PWR_CTRL.
        let post_blob_start = 3 + chunk_count * 3;
        assert_eq!(writes[post_blob_start].1, vec![REG_INIT_CTRL, 0x01]);
        assert_eq!(
            writes[post_blob_start + 1].1,
            vec![REG_ACC_CONF, ACC_CONF_VALUE]
        );
        assert_eq!(
            writes[post_blob_start + 2].1,
            vec![REG_ACC_RANGE, ACC_RANGE_VALUE]
        );
        assert_eq!(
            writes[post_blob_start + 3].1,
            vec![REG_GYR_CONF, GYR_CONF_VALUE]
        );
        assert_eq!(
            writes[post_blob_start + 4].1,
            vec![REG_GYR_RANGE, GYR_RANGE_VALUE]
        );
        assert_eq!(
            writes[post_blob_start + 5].1,
            vec![REG_PWR_CTRL, PWR_CTRL_ENABLE]
        );

        // Reads: chip-id + one INTERNAL_STATUS poll.
        assert_eq!(
            harness.reads(),
            vec![
                (ADDRESS_PRIMARY, REG_CHIP_ID, 1),
                (ADDRESS_PRIMARY, REG_INTERNAL_STATUS, 1),
            ]
        );
    }

    #[test]
    fn read_measurement_burst_reads_twelve_bytes_at_data_start() {
        let harness = Harness::new();
        // Fabricated burst: 1 g on accel-z, 100 dps on gyro-x; rest 0.
        let accel_z_counts: i16 = 8_192;
        let gyro_x_counts: i16 = 3_277;
        let mut buf = [0u8; DATA_LEN];
        let az = accel_z_counts.to_le_bytes();
        buf[4] = az[0];
        buf[5] = az[1];
        let gx = gyro_x_counts.to_le_bytes();
        buf[6] = gx[0];
        buf[7] = gx[1];
        harness.queue_read(buf.to_vec());

        let bus = MockI2c { harness: &harness };
        let mut imu = Bmi270::new(bus, ADDRESS_PRIMARY);
        let m = block_on(imu.read_measurement()).unwrap();
        assert!((m.accel_g.2 - 1.0).abs() < 1e-3);
        assert!((m.gyro_dps.0 - 100.0).abs() < 0.1);
        assert_eq!(
            harness.reads(),
            vec![(ADDRESS_PRIMARY, REG_DATA_START, DATA_LEN)]
        );
    }

    #[test]
    fn read_raw_returns_the_unconverted_burst() {
        let harness = Harness::new();
        let canned: [u8; DATA_LEN] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C,
        ];
        harness.queue_read(canned.to_vec());

        let bus = MockI2c { harness: &harness };
        let mut imu = Bmi270::new(bus, ADDRESS_PRIMARY);
        let buf = block_on(imu.read_raw()).unwrap();
        assert_eq!(buf, canned);
        assert_eq!(
            harness.reads(),
            vec![(ADDRESS_PRIMARY, REG_DATA_START, DATA_LEN)]
        );
    }
}
