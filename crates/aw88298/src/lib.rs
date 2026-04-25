//! # aw88298
//!
//! `no_std` async I²C control-path driver for the Awinic AW88298
//! 16-bit I2S "smart K" digital audio amplifier.
//!
//! On the CoreS3 Stack-chan the AW88298 drives the single 1 W speaker
//! from an integrated 10.25 V smart boost converter. The I²C side
//! handles configuration; audio data arrives over I2S from the MCU.
//!
//! ## Register width
//!
//! Every data register is **16 bits** wide, transferred big-endian on
//! the I²C bus: one transaction is `[reg, msb, lsb]`. The register
//! *addresses* themselves remain 8-bit. Mixing this up with the usual
//! 8-bit-register convention yields a chip that NACKs every second
//! write.
//!
//! ## External reset
//!
//! The `RST` pin is wired to AW9523 `P0_1`. The `aw9523` crate's CoreS3
//! bring-up helper releases it as part of board init — the amp NACKs
//! every I²C transaction until then.
//!
//! ## Initialisation
//!
//! The init sequence mirrors Espressif's `esp_codec_dev` reference for
//! this exact CoreS3 codec (see
//! `components/esp_codec_dev/device/aw88298/aw88298.c` in
//! [`espressif/esp-adf`][esp-adf], Apache-2.0): reset, enable I2S,
//! start muted, program I2S format + sample rate, program volume /
//! AGC, disable boost. Sample rate is stored in the lower nibble of
//! `I2SCTRL`; callers can re-program it at runtime via
//! [`Aw88298::set_sample_rate`].
//!
//! [esp-adf]: https://github.com/espressif/esp-adf
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::{i2c::I2c, delay::DelayNs};
//! # async fn demo<B, D>(bus: B, mut delay: D) -> Result<(), aw88298::Error<B::Error>>
//! # where B: I2c, D: DelayNs {
//! let mut amp = aw88298::Aw88298::new(bus);
//! amp.init(&mut delay).await?;
//! amp.set_sample_rate(aw88298::SampleRate::Hz16000).await?;
//! amp.set_muted(false).await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::{delay::DelayNs, i2c::I2c};

/// 7-bit I²C address with `AD1 = AD2 = GND` (the CoreS3 wiring).
///
/// Datasheet address format is `0b01101xx`; strap pins set the two
/// LSBs, giving the `0x34..=0x37` window.
pub const ADDRESS: u8 = 0x36;

/// Expected chip ID, read as a 16-bit big-endian value from register
/// `0x00`.
pub const CHIP_ID: u16 = 0x1852;

/// `RESET` register. Doubles as the chip-ID read address — reads yield
/// `0x1852`, writes trigger actions (`0x55AA` = soft-reset).
const REG_RESET: u8 = 0x00;
/// `SYSCTRL` register. Bits: `PWDN` (0), `AMPPD` (1), `I2SEN` (6).
const REG_SYSCTRL: u8 = 0x04;
/// `SYSCTRL2` register. Bit 0 = `HMUTE` (mute output).
const REG_SYSCTRL2: u8 = 0x05;
/// `I2SCTRL` register. Bits 7:4 bit-depth, bits 3:0 sample-rate index.
const REG_I2SCTRL: u8 = 0x06;
/// `HAGCCFG4` register. Upper byte volume, lower byte AGC preset.
const REG_HAGCCFG4: u8 = 0x0C;
/// `BSTCTRL2` register. Boost converter mode + current-limit config.
const REG_BSTCTRL2: u8 = 0x61;

/// Reset-magic written to `REG_RESET`.
const RESET_MAGIC: u16 = 0x55AA;
/// `SYSCTRL` value: `I2SEN = 1`, `AMPPD = 0`, `PWDN = 0`.
const SYSCTRL_ENABLE: u16 = 0x4040;
/// `SYSCTRL2` value: HMUTE clear, AGC off. Pair with
/// [`SYSCTRL2_MUTED`] for the mute bit.
const SYSCTRL2_RUN: u16 = 0x0008;
/// `SYSCTRL2` value: `HMUTE` set (bit 0).
const SYSCTRL2_MUTED: u16 = SYSCTRL2_RUN | 0x0001;
/// `I2SCTRL` base value: 16-bit Philips I2S, BCK mode ×16. Combined
/// with a [`SampleRate`] byte in the low nibble.
const I2SCTRL_BASE: u16 = 0x3CC0;
/// `HAGCCFG4` default: volume `0x30` (≈ -24 dB) + AGC preset `0x64`.
const HAGCCFG4_DEFAULT: u16 = 0x3064;
/// `BSTCTRL2` value: boost mode disabled. Matches esp-adf's
/// `0x0673` vs the datasheet default `0x6673`.
const BSTCTRL2_BOOST_OFF: u16 = 0x0673;

/// Post-reset settle delay, in milliseconds. Datasheet ≥1 ms.
const RESET_SETTLE_MS: u32 = 5;

/// Supported I²S sample rates. Nibble value written into `REG_I2SCTRL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SampleRate {
    /// 8 kHz.
    Hz8000 = 0x01,
    /// 11.025 kHz.
    Hz11025 = 0x02,
    /// 12 kHz.
    Hz12000 = 0x03,
    /// 16 kHz. Default for the firmware's voice pipeline.
    Hz16000 = 0x04,
    /// 22.05 kHz.
    Hz22050 = 0x05,
    /// 24 kHz.
    Hz24000 = 0x06,
    /// 32 kHz.
    Hz32000 = 0x07,
    /// 44.1 kHz.
    Hz44100 = 0x08,
    /// 48 kHz.
    Hz48000 = 0x09,
    /// 96 kHz.
    Hz96000 = 0x0A,
}

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
    /// `CHIPID` register did not return [`CHIP_ID`]. Contains the raw
    /// 16-bit value that was read.
    BadChipId(u16),
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// AW88298 driver handle.
pub struct Aw88298<B> {
    /// Underlying I²C bus.
    bus: B,
    /// Resolved 7-bit I²C address.
    address: u8,
}

impl<B: I2c> Aw88298<B> {
    /// Wrap an I²C bus with the default [`ADDRESS`].
    #[must_use]
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            address: ADDRESS,
        }
    }

    /// Wrap an I²C bus with a specific address (for strap variants).
    #[must_use]
    pub const fn with_address(bus: B, address: u8) -> Self {
        Self { bus, address }
    }

    /// Resolved 7-bit I²C address. Useful for logging.
    #[must_use]
    pub const fn address(&self) -> u8 {
        self.address
    }

    /// Read the 16-bit `CHIPID` register (big-endian on the wire).
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_chip_id(&mut self) -> Result<u16, Error<B::Error>> {
        let mut buf = [0u8; 2];
        self.bus
            .write_read(self.address, &[REG_RESET], &mut buf)
            .await?;
        Ok(u16::from_be_bytes(buf))
    }

    /// Full initialisation sequence. Caller releases the AW88298 `RST`
    /// pin (via AW9523) before this runs.
    ///
    /// After this returns:
    ///
    /// - I2S interface is enabled (waiting for MCLK / BCLK / LRCK)
    /// - Output is **muted** via `HMUTE`; call [`Aw88298::set_muted`]
    ///   once the I2S stream is flowing
    /// - Sample rate is configured to 16 kHz; re-program via
    ///   [`Aw88298::set_sample_rate`]
    /// - Boost converter disabled (8 V rail, sufficient for the
    ///   Stack-chan's 1 W speaker without clip)
    /// - Volume at -24 dB (esp-adf default)
    ///
    /// # Errors
    ///
    /// - [`Error::BadChipId`] if `CHIPID` doesn't read `0x1852`.
    /// - [`Error::I2c`] on any bus failure.
    pub async fn init<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<B::Error>> {
        delay.delay_ms(RESET_SETTLE_MS).await;
        let id = self.read_chip_id().await?;
        if id != CHIP_ID {
            return Err(Error::BadChipId(id));
        }

        self.write_reg(REG_RESET, RESET_MAGIC).await?;
        delay.delay_ms(RESET_SETTLE_MS).await;
        self.write_reg(REG_SYSCTRL, SYSCTRL_ENABLE).await?;
        self.write_reg(REG_SYSCTRL2, SYSCTRL2_MUTED).await?;
        self.write_reg(
            REG_I2SCTRL,
            I2SCTRL_BASE | u16::from(SampleRate::Hz16000 as u8),
        )
        .await?;
        self.write_reg(REG_HAGCCFG4, HAGCCFG4_DEFAULT).await?;
        self.write_reg(REG_BSTCTRL2, BSTCTRL2_BOOST_OFF).await?;
        Ok(())
    }

    /// Select the I²S sample rate. Must match the MCU's I²S master
    /// configuration; a mismatch pitches audio up or down.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn set_sample_rate(&mut self, rate: SampleRate) -> Result<(), Error<B::Error>> {
        self.write_reg(REG_I2SCTRL, I2SCTRL_BASE | u16::from(rate as u8))
            .await
    }

    /// Mute / un-mute the output stage. Toggles `HMUTE` (bit 0) in
    /// `SYSCTRL2`.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn set_muted(&mut self, muted: bool) -> Result<(), Error<B::Error>> {
        let value = if muted { SYSCTRL2_MUTED } else { SYSCTRL2_RUN };
        self.write_reg(REG_SYSCTRL2, value).await
    }

    /// Single 16-bit-register write. Encodes value big-endian on wire.
    async fn write_reg(&mut self, reg: u8, value: u16) -> Result<(), Error<B::Error>> {
        let bytes = value.to_be_bytes();
        self.bus
            .write(self.address, &[reg, bytes[0], bytes[1]])
            .await?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test scaffolding: Infallible bus error makes unwrap() sound"
)]
#[allow(
    clippy::panic,
    reason = "tests panic via assert! / assertion-by-match on unexpected variants"
)]
mod tests {
    use super::*;
    use core::cell::RefCell;
    use embedded_hal_async::{
        delay::DelayNs,
        i2c::{ErrorType, Operation, SevenBitAddress},
    };

    /// Host-side I²C mock: canned chip-ID read response, records every
    /// write payload in order.
    struct MockI2c {
        /// 16-bit chip-ID the mock returns on any read (big-endian).
        chip_id: [u8; 2],
        /// Every `Operation::Write` payload, in emission order.
        writes: RefCell<Vec<Vec<u8>>>,
    }

    impl MockI2c {
        fn with_chip_id(id: u16) -> Self {
            Self {
                chip_id: id.to_be_bytes(),
                writes: RefCell::new(Vec::new()),
            }
        }
    }

    impl ErrorType for MockI2c {
        type Error = core::convert::Infallible;
    }

    impl I2c for MockI2c {
        async fn transaction(
            &mut self,
            _address: SevenBitAddress,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Self::Error> {
            for op in operations {
                match op {
                    Operation::Write(buf) => {
                        self.writes.borrow_mut().push(buf.to_vec());
                    }
                    Operation::Read(buf) => {
                        let n = buf.len().min(self.chip_id.len());
                        buf[..n].copy_from_slice(&self.chip_id[..n]);
                    }
                }
            }
            Ok(())
        }
    }

    /// No-op delay: blocking async impl that never sleeps.
    struct NoopDelay;

    impl DelayNs for NoopDelay {
        async fn delay_ns(&mut self, _ns: u32) {}
    }

    /// Tiny future poller — axp2101 tests use the same pattern.
    fn block_on<F: core::future::Future>(future: F) -> F::Output {
        use core::pin::pin;
        use core::task::{Context, Poll, Waker};
        let mut cx = Context::from_waker(Waker::noop());
        let mut fut = pin!(future);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    #[test]
    fn init_issues_canonical_sequence() {
        let mut amp = Aw88298::new(MockI2c::with_chip_id(CHIP_ID));
        block_on(amp.init(&mut NoopDelay)).unwrap();

        // Filter to the 3-byte "register write" payloads — the mock
        // also captures the 1-byte register-address setup that
        // `write_read` emits during the chip-ID probe.
        let writes: Vec<Vec<u8>> = amp
            .bus
            .writes
            .borrow()
            .iter()
            .filter(|w| w.len() == 3)
            .cloned()
            .collect();
        // 6 register writes in the canonical esp-adf order.
        assert_eq!(writes.len(), 6);
        // RESET = 0x55AA
        assert_eq!(writes[0], [REG_RESET, 0x55, 0xAA]);
        // SYSCTRL = 0x4040
        assert_eq!(writes[1], [REG_SYSCTRL, 0x40, 0x40]);
        // SYSCTRL2 muted (0x0009 = SYSCTRL2_RUN | HMUTE)
        assert_eq!(writes[2], [REG_SYSCTRL2, 0x00, 0x09]);
        // I2SCTRL base 0x3CC0 | 16 kHz nibble 0x04 = 0x3CC4
        assert_eq!(writes[3], [REG_I2SCTRL, 0x3C, 0xC4]);
        // HAGCCFG4 default
        assert_eq!(writes[4], [REG_HAGCCFG4, 0x30, 0x64]);
        // BSTCTRL2 boost-off
        assert_eq!(writes[5], [REG_BSTCTRL2, 0x06, 0x73]);
    }

    #[test]
    fn init_rejects_wrong_chip_id() {
        let mut amp = Aw88298::new(MockI2c::with_chip_id(0x1234));
        match block_on(amp.init(&mut NoopDelay)) {
            Err(Error::BadChipId(0x1234)) => {}
            other => panic!("expected BadChipId(0x1234), got {other:?}"),
        }
    }

    #[test]
    fn set_sample_rate_writes_i2sctrl_with_rate_nibble() {
        let mut amp = Aw88298::new(MockI2c::with_chip_id(CHIP_ID));
        block_on(amp.set_sample_rate(SampleRate::Hz48000)).unwrap();
        let writes = amp.bus.writes.borrow();
        assert_eq!(writes.len(), 1);
        // I2SCTRL base 0x3CC0 | 48 kHz nibble 0x09 = 0x3CC9.
        assert_eq!(writes[0], [REG_I2SCTRL, 0x3C, 0xC9]);
    }

    #[test]
    fn set_muted_toggles_hmute_bit() {
        let mut amp = Aw88298::new(MockI2c::with_chip_id(CHIP_ID));
        block_on(amp.set_muted(true)).unwrap();
        block_on(amp.set_muted(false)).unwrap();
        let writes = amp.bus.writes.borrow();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0], [REG_SYSCTRL2, 0x00, 0x09]);
        assert_eq!(writes[1], [REG_SYSCTRL2, 0x00, 0x08]);
    }

    #[test]
    fn read_chip_id_decodes_big_endian() {
        let mut amp = Aw88298::new(MockI2c::with_chip_id(0x1852));
        let id = block_on(amp.read_chip_id()).unwrap();
        assert_eq!(id, 0x1852);
    }
}
