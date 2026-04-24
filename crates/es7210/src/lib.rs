//! # es7210
//!
//! `no_std` async I²C control-path driver for the Everest ES7210
//! four-channel 24-bit audio ADC.
//!
//! On the CoreS3 Stack-chan the ES7210 captures the two on-board
//! microphones (channels 1 + 2); channels 3 + 4 are unused. Audio data
//! is clocked out over I2S; this crate handles register configuration
//! for a fixed audio shape: **12.288 MHz MCLK, 16 kHz sample rate,
//! 16-bit mono, chip as I²S slave**. Re-programming at runtime is not
//! supported — change the constants and rebuild if you need another
//! rate.
//!
//! ## Initialisation
//!
//! The sequence is a direct port of
//! `components/esp_codec_dev/device/es7210/es7210.c` in
//! [`espressif/esp-adf`][esp-adf] (Apache-2.0), simplified for the
//! Stack-chan's two-mic layout at a fixed sample rate.
//!
//! [esp-adf]: https://github.com/espressif/esp-adf
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::{i2c::I2c, delay::DelayNs};
//! # async fn demo<B, D>(bus: B, mut delay: D) -> Result<(), es7210::Error<B::Error>>
//! # where B: I2c, D: DelayNs {
//! let mut adc = es7210::Es7210::new(bus);
//! adc.init(&mut delay).await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::{delay::DelayNs, i2c::I2c};

/// Default 7-bit I²C address with `AD1 = AD0 = GND`.
pub const ADDRESS: u8 = 0x40;

/// Expected chip-ID low byte at [`REG_CHIP_ID1`].
pub const CHIP_ID1: u8 = 0x72;
/// Expected chip-ID high byte at [`REG_CHIP_ID2`].
pub const CHIP_ID2: u8 = 0x10;

/// `RESET` register. `0x71` asserts software reset, `0x41` releases.
const REG_RESET: u8 = 0x00;
/// `CLOCK_OFF` / mic-power gate register. Bits 0..5 = per-channel clock
/// enable; higher bits = MIC12/34 power gates.
const REG_CLOCK_OFF: u8 = 0x01;
/// `MAINCLK` register. Bits 5:0 = `adc_div`, bit 6 = `doubler`, bit 7 = `dll`.
const REG_MAINCLK: u8 = 0x02;
/// `LRCK_DIVH` register. Upper 4 bits of the 12-bit LRCK divider.
const REG_LRCK_DIVH: u8 = 0x04;
/// `LRCK_DIVL` register. Lower 8 bits of the LRCK divider.
const REG_LRCK_DIVL: u8 = 0x05;
/// `POWER_DOWN` register. `0x00` = all blocks on; `0x07` = everything off.
const REG_POWER_DOWN: u8 = 0x06;
/// `OSR` (over-sample ratio) register.
const REG_OSR: u8 = 0x07;
/// `ANALOG` block configuration register.
const REG_ANALOG: u8 = 0x40;
/// `MIC1_GAIN` register. Bit 4 = gain-enable, bits 3:0 = gain step.
const REG_MIC1_GAIN: u8 = 0x43;
/// `MIC2_GAIN` register.
const REG_MIC2_GAIN: u8 = 0x44;
/// `MIC1_POWER` register. `0x08` = on, `0xFF` = off.
const REG_MIC1_POWER: u8 = 0x47;
/// `MIC2_POWER` register.
const REG_MIC2_POWER: u8 = 0x48;
/// `MIC3_POWER` register.
const REG_MIC3_POWER: u8 = 0x49;
/// `MIC4_POWER` register.
const REG_MIC4_POWER: u8 = 0x4A;
/// `MIC12_POWER` group register. `0x00` = mics 1+2 powered, `0xFF` = off.
const REG_MIC12_POWER: u8 = 0x4B;
/// `MIC34_POWER` group register.
const REG_MIC34_POWER: u8 = 0x4C;
/// `CHIP_ID1` register (low byte of two-byte ID).
const REG_CHIP_ID1: u8 = 0xFD;
/// `CHIP_ID2` register (high byte).
const REG_CHIP_ID2: u8 = 0xFE;

// ---- Clock coefficients for 12.288 MHz MCLK → 16 kHz sample rate -----
//
// Lifted from `coeff_div[]` in esp-adf's `es7210.c`, row
// `{12288000, 16000, 0x00, 0x03, 0x01, 0x01, 0x20, 0x00, 0x03, 0x00}`.
// Fields: `(mclk, lrck, ss_ds, adc_div, dll, doubler, osr, mclk_src,
// lrck_h, lrck_l)`. Only `adc_div`, `dll`, `doubler`, `osr`, `lrck_h`,
// `lrck_l` matter for the configured rate; the rest are defaults.

/// `MAINCLK` value: `adc_div=0x03 | doubler << 6 | dll << 7` = `0xC3`.
const MAINCLK_VALUE: u8 = 0x03 | (0x01 << 6) | (0x01 << 7);
/// `OSR` value: `0x20` (256× oversample).
const OSR_VALUE: u8 = 0x20;
/// `LRCK_DIVH` value: `0x03` (upper nibble of LRCK divider).
const LRCK_DIVH_VALUE: u8 = 0x03;
/// `LRCK_DIVL` value: `0x00` (lower byte).
const LRCK_DIVL_VALUE: u8 = 0x00;

/// `CLOCK_OFF` value: enable mic1+2 channel clocks, gate mic3+4. Lower
/// nibble bits 0/1 = mic12 clock gates (0 = on); upper nibble bits
/// 4/5 = mic34 clock gates (1 = off).
const CLOCK_OFF_MIC12_ON: u8 = 0b0011_0000;
/// `ANALOG` value: `0x43` (datasheet block-enable pattern for active-mode).
const ANALOG_ACTIVE: u8 = 0x43;
/// Per-mic power-on value (`0x08`) applied to `MIC1..4_POWER`.
const MIC_POWER_ON: u8 = 0x08;
/// `MIC12_POWER` value: both mics powered.
const MIC12_POWER_ON: u8 = 0x00;
/// `MIC34_POWER` value: both mics powered off (we don't use them).
const MIC34_POWER_OFF: u8 = 0xFF;
/// Mic-gain enable bit in `MICx_GAIN`.
const MIC_GAIN_ENABLE: u8 = 0x10;
/// Default mic-gain step (bits 3:0). `0x0A` ≈ +30 dB (datasheet).
const MIC_GAIN_DEFAULT: u8 = 0x0A;

/// `RESET` assert value.
const RESET_ASSERT: u8 = 0x71;
/// `RESET` release value. Written after assert, and again at the end of
/// init to "latch" the applied config.
const RESET_RELEASE: u8 = 0x41;

/// Post-reset settle delay, in milliseconds. Datasheet asks "a few ms";
/// 5 ms matches the reference.
const RESET_SETTLE_MS: u32 = 5;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
    /// Chip-ID registers did not match the expected `(0x72, 0x10)`.
    /// Contains the raw bytes read from (`CHIP_ID1`, `CHIP_ID2`).
    BadChipId(u8, u8),
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// ES7210 driver handle.
pub struct Es7210<B> {
    /// Underlying I²C bus.
    bus: B,
    /// Resolved 7-bit I²C address.
    address: u8,
}

impl<B: I2c> Es7210<B> {
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

    /// Read the two chip-ID bytes.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_chip_id(&mut self) -> Result<(u8, u8), Error<B::Error>> {
        let mut lo = [0u8; 1];
        let mut hi = [0u8; 1];
        self.bus
            .write_read(self.address, &[REG_CHIP_ID1], &mut lo)
            .await?;
        self.bus
            .write_read(self.address, &[REG_CHIP_ID2], &mut hi)
            .await?;
        Ok((lo[0], hi[0]))
    }

    /// Full initialisation for the CoreS3 Stack-chan's two-mic layout.
    ///
    /// Applies the esp-adf reference sequence, simplified to fixed
    /// params: 12.288 MHz MCLK, 16 kHz sample rate, 16-bit mono,
    /// ES7210 as I²S slave, mic1+2 on at +30 dB, mic3+4 off.
    ///
    /// # Errors
    ///
    /// - [`Error::BadChipId`] if the chip-ID bytes don't read
    ///   `(0x72, 0x10)`.
    /// - [`Error::I2c`] on any bus failure.
    pub async fn init<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<B::Error>> {
        // Verify this is an ES7210 before poking it further.
        let (lo, hi) = self.read_chip_id().await?;
        if lo != CHIP_ID1 || hi != CHIP_ID2 {
            return Err(Error::BadChipId(lo, hi));
        }

        // Soft-reset pulse.
        self.write_reg(REG_RESET, RESET_ASSERT).await?;
        delay.delay_ms(RESET_SETTLE_MS).await;
        self.write_reg(REG_RESET, RESET_RELEASE).await?;
        delay.delay_ms(RESET_SETTLE_MS).await;

        // Clock tree for 12.288 MHz MCLK → 16 kHz LRCK.
        self.write_reg(REG_MAINCLK, MAINCLK_VALUE).await?;
        self.write_reg(REG_OSR, OSR_VALUE).await?;
        self.write_reg(REG_LRCK_DIVH, LRCK_DIVH_VALUE).await?;
        self.write_reg(REG_LRCK_DIVL, LRCK_DIVL_VALUE).await?;

        // Start sequence: clock gates, power-up, analog, per-mic power,
        // mic select, analog re-assert, latch reset. Mirrors the
        // `es7210_start()` body in esp-adf.
        self.write_reg(REG_CLOCK_OFF, CLOCK_OFF_MIC12_ON).await?;
        self.write_reg(REG_POWER_DOWN, 0x00).await?;
        self.write_reg(REG_ANALOG, ANALOG_ACTIVE).await?;
        self.write_reg(REG_MIC1_POWER, MIC_POWER_ON).await?;
        self.write_reg(REG_MIC2_POWER, MIC_POWER_ON).await?;
        self.write_reg(REG_MIC3_POWER, MIC_POWER_ON).await?;
        self.write_reg(REG_MIC4_POWER, MIC_POWER_ON).await?;

        // Mic select: power up MIC12 group, power down MIC34 group,
        // enable gain + preset step on the two channels we use.
        self.write_reg(REG_MIC12_POWER, MIC12_POWER_ON).await?;
        self.write_reg(REG_MIC34_POWER, MIC34_POWER_OFF).await?;
        self.write_reg(REG_MIC1_GAIN, MIC_GAIN_ENABLE | MIC_GAIN_DEFAULT)
            .await?;
        self.write_reg(REG_MIC2_GAIN, MIC_GAIN_ENABLE | MIC_GAIN_DEFAULT)
            .await?;

        // Re-assert the analog register (the reference sequence does
        // this twice) and latch via a second reset pulse.
        self.write_reg(REG_ANALOG, ANALOG_ACTIVE).await?;
        self.write_reg(REG_RESET, RESET_ASSERT).await?;
        self.write_reg(REG_RESET, RESET_RELEASE).await?;

        Ok(())
    }

    /// Set the gain step on both active mic channels.
    ///
    /// `step` is the lower-nibble value written to `MICx_GAIN` —
    /// `0x00..=0x0A` maps 0 dB to +30 dB in 3 dB increments per the
    /// datasheet. Values above `0x0F` are clamped.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn set_gain(&mut self, step: u8) -> Result<(), Error<B::Error>> {
        let clamped = step & 0x0F;
        let value = MIC_GAIN_ENABLE | clamped;
        self.write_reg(REG_MIC1_GAIN, value).await?;
        self.write_reg(REG_MIC2_GAIN, value).await
    }

    /// Single-register write. 8-bit register, 8-bit value.
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
    clippy::panic,
    reason = "tests panic via assertion-by-match on unexpected variants"
)]
mod tests {
    use super::*;
    use core::cell::RefCell;
    use embedded_hal_async::{
        delay::DelayNs,
        i2c::{ErrorType, Operation, SevenBitAddress},
    };

    /// Host-side I²C mock: canned reads keyed by register, records every write.
    struct MockI2c {
        /// (register, value) response table; the mock tracks the most
        /// recently written register as "the next read target."
        reads: [(u8, u8); 2],
        /// Register the most recent 1-byte write selected (from
        /// `write_read`'s Write half).
        next_read_reg: RefCell<u8>,
        /// Every 2-byte `Operation::Write` payload, in emission order.
        writes: RefCell<Vec<(u8, u8)>>,
    }

    impl MockI2c {
        fn with_chip_id(lo: u8, hi: u8) -> Self {
            Self {
                reads: [(REG_CHIP_ID1, lo), (REG_CHIP_ID2, hi)],
                next_read_reg: RefCell::new(0),
                writes: RefCell::new(Vec::new()),
            }
        }

        fn read_for(&self, reg: u8) -> u8 {
            self.reads
                .iter()
                .find(|(r, _)| *r == reg)
                .map_or(0, |(_, v)| *v)
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
                        if buf.len() == 1 {
                            // `write_read`'s register-select byte.
                            *self.next_read_reg.borrow_mut() = buf[0];
                        } else if buf.len() == 2 {
                            // Register write.
                            self.writes.borrow_mut().push((buf[0], buf[1]));
                        }
                    }
                    Operation::Read(buf) => {
                        let reg = *self.next_read_reg.borrow();
                        buf[0] = self.read_for(reg);
                    }
                }
            }
            Ok(())
        }
    }

    struct NoopDelay;

    impl DelayNs for NoopDelay {
        async fn delay_ns(&mut self, _ns: u32) {}
    }

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
    fn init_writes_clock_tree_for_12288_mhz_16khz() {
        let mut adc = Es7210::new(MockI2c::with_chip_id(CHIP_ID1, CHIP_ID2));
        block_on(adc.init(&mut NoopDelay)).unwrap();
        let writes = adc.bus.writes.borrow();
        // The clock-tree writes come right after the two reset pulses.
        let clock_writes: Vec<&(u8, u8)> = writes
            .iter()
            .filter(|(r, _)| matches!(*r, REG_MAINCLK | REG_OSR | REG_LRCK_DIVH | REG_LRCK_DIVL))
            .collect();
        assert_eq!(clock_writes.len(), 4);
        assert!(clock_writes.contains(&&(REG_MAINCLK, 0xC3)));
        assert!(clock_writes.contains(&&(REG_OSR, 0x20)));
        assert!(clock_writes.contains(&&(REG_LRCK_DIVH, 0x03)));
        assert!(clock_writes.contains(&&(REG_LRCK_DIVL, 0x00)));
    }

    #[test]
    fn init_enables_mic12_and_disables_mic34() {
        let mut adc = Es7210::new(MockI2c::with_chip_id(CHIP_ID1, CHIP_ID2));
        block_on(adc.init(&mut NoopDelay)).unwrap();
        let writes = adc.bus.writes.borrow();
        // MIC12 group powered on (0x00), MIC34 group powered off (0xFF).
        assert!(writes.contains(&(REG_MIC12_POWER, MIC12_POWER_ON)));
        assert!(writes.contains(&(REG_MIC34_POWER, MIC34_POWER_OFF)));
        // Gain enable + default step on mic1 and mic2.
        let expected_gain = MIC_GAIN_ENABLE | MIC_GAIN_DEFAULT;
        assert!(writes.contains(&(REG_MIC1_GAIN, expected_gain)));
        assert!(writes.contains(&(REG_MIC2_GAIN, expected_gain)));
    }

    #[test]
    fn init_bookends_with_two_reset_pulses() {
        let mut adc = Es7210::new(MockI2c::with_chip_id(CHIP_ID1, CHIP_ID2));
        block_on(adc.init(&mut NoopDelay)).unwrap();
        let writes = adc.bus.writes.borrow();
        let resets: Vec<&(u8, u8)> = writes.iter().filter(|(r, _)| *r == REG_RESET).collect();
        // Two pulses: open + close. Each pulse is assert then release.
        assert_eq!(resets.len(), 4);
        assert_eq!(resets[0], &(REG_RESET, RESET_ASSERT));
        assert_eq!(resets[1], &(REG_RESET, RESET_RELEASE));
        assert_eq!(resets[2], &(REG_RESET, RESET_ASSERT));
        assert_eq!(resets[3], &(REG_RESET, RESET_RELEASE));
    }

    #[test]
    fn init_rejects_wrong_chip_id() {
        let mut adc = Es7210::new(MockI2c::with_chip_id(0x00, 0x00));
        match block_on(adc.init(&mut NoopDelay)) {
            Err(Error::BadChipId(0, 0)) => {}
            other => panic!("expected BadChipId(0, 0), got {other:?}"),
        }
    }

    #[test]
    fn set_gain_clamps_to_lower_nibble() {
        let mut adc = Es7210::new(MockI2c::with_chip_id(CHIP_ID1, CHIP_ID2));
        block_on(adc.set_gain(0xFF)).unwrap();
        let writes = adc.bus.writes.borrow();
        assert_eq!(writes.len(), 2);
        // High bits dropped; gain-enable bit forced on.
        assert_eq!(writes[0], (REG_MIC1_GAIN, 0x1F));
        assert_eq!(writes[1], (REG_MIC2_GAIN, 0x1F));
    }
}
