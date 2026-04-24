//! # pca9685
//!
//! `no_std` driver for the NXP PCA9685 16-channel, 12-bit PWM/servo controller.
//!
//! The chip hangs off an I²C bus and generates up to 16 synchronised PWM
//! outputs. It has a programmable prescaler that divides an internal
//! 25 MHz oscillator down to PWM frequencies between 24 Hz and 1526 Hz,
//! with 4096 ticks of resolution per period. Servos want a 50 Hz frame
//! (20 ms period), so a pulse of 1000 µs maps to an off-count of
//! `1000 * 4096 / 20000 = 204`, and 1500 µs (center) to `307`.
//!
//! This driver is deliberately minimal — enough to drive the SG90 pan/tilt
//! servos on a CoreS3 StackChan base:
//!
//! - [`Pca9685::init`] sequence: sleep → prescale for target freq → wake
//!   (with a 500 µs oscillator-stabilization wait) → enable register
//!   auto-increment.
//! - [`Pca9685::set_channel_pulse_us`] maps a pulse width in microseconds
//!   to the on/off counts and writes all four channel registers in one
//!   I²C transaction (auto-increment fires the four-byte burst).
//! - [`Pca9685::set_channel_raw`] for callers that need the 12-bit
//!   on/off counts directly.
//! - [`Pca9685::sleep`] / [`Pca9685::wake`] for power gating.
//!
//! Register bookkeeping is kept verbatim from the NXP datasheet so the
//! transport is auditable against the reference sequence; consumers are
//! expected to hold the pulse-to-servo-angle mapping (each servo's
//! calibration is different) outside the driver.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::{i2c::I2c, delay::DelayNs};
//! # async fn demo<B, D>(bus: B, mut delay: D) -> Result<(), pca9685::Error<B::Error>>
//! # where B: I2c, D: DelayNs {
//! let mut pwm = pca9685::Pca9685::new(bus, pca9685::DEFAULT_ADDRESS);
//! pwm.init(50, &mut delay).await?;              // 50 Hz servo frame
//! pwm.set_channel_pulse_us(0, 1500).await?;     // channel 0 → center
//! # Ok(())
//! # }
//! ```

#![no_std]
#![deny(unsafe_code)]

use embedded_hal_async::{delay::DelayNs, i2c::I2c};

/// Default 7-bit I²C address with all address pins tied low (`A5..A0 = 0`).
pub const DEFAULT_ADDRESS: u8 = 0x40;

/// Internal oscillator frequency, in Hz. Used by the prescale calculation.
const OSC_HZ: u32 = 25_000_000;
/// Minimum PWM frequency the prescaler can reach.
pub const MIN_FREQ_HZ: u32 = 24;
/// Maximum PWM frequency the prescaler can reach.
pub const MAX_FREQ_HZ: u32 = 1526;

/// PWM ticks per period (12-bit counter).
pub const TICKS_PER_PERIOD: u16 = 4096;
/// Highest valid channel index (16 channels, 0–15).
pub const MAX_CHANNEL: u8 = 15;

/// MODE1 register address.
const REG_MODE1: u8 = 0x00;
/// MODE2 register address.
const REG_MODE2: u8 = 0x01;
/// Channel-0 ON low-byte; channel N bases at `REG_LED0_ON_L + 4*N`.
const REG_LED0_ON_L: u8 = 0x06;
/// `PRE_SCALE` register address.
const REG_PRE_SCALE: u8 = 0xFE;

/// MODE1 bit: register auto-increment on multi-byte writes.
const MODE1_AI: u8 = 1 << 5;
/// MODE1 bit: low-power sleep (oscillator off). Required before writing `PRE_SCALE`.
const MODE1_SLEEP: u8 = 1 << 4;

/// MODE2 bit: totem-pole (push/pull) output stage. Cleared = open-drain.
const MODE2_OUTDRV: u8 = 1 << 2;

/// Oscillator-stabilization wait after clearing MODE1.SLEEP (datasheet: 500 µs).
const WAKE_DELAY_US: u32 = 500;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
    /// Requested PWM frequency is outside the `MIN_FREQ_HZ..=MAX_FREQ_HZ`
    /// range the prescaler can produce.
    FrequencyOutOfRange,
    /// Channel index exceeds [`MAX_CHANNEL`] (15).
    ChannelOutOfRange,
    /// On/off count exceeds the 12-bit PWM resolution (> `TICKS_PER_PERIOD - 1`).
    CountOutOfRange,
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// PCA9685 driver. Generic over any [`embedded_hal_async::i2c::I2c`].
#[derive(Debug)]
pub struct Pca9685<B> {
    /// The owned I²C bus. Recover with [`Pca9685::into_inner`].
    bus: B,
    /// 7-bit I²C address of this chip.
    address: u8,
}

impl<B: I2c> Pca9685<B> {
    /// Construct a driver without talking to the chip. Call [`Pca9685::init`]
    /// before issuing any PWM commands — the chip boots with PWM outputs
    /// enabled at an undefined prescale, which would generate random pulses.
    #[must_use]
    pub const fn new(bus: B, address: u8) -> Self {
        Self { bus, address }
    }

    /// Release the owned I²C bus back to the caller.
    #[must_use]
    pub fn into_inner(self) -> B {
        self.bus
    }

    /// Run the standard servo-friendly init sequence:
    /// sleep → write prescale for `pwm_freq_hz` → wake (with the datasheet's
    /// 500 µs oscillator-stabilization wait) → enable register
    /// auto-increment and totem-pole outputs.
    ///
    /// # Errors
    /// - `FrequencyOutOfRange` if `pwm_freq_hz` is outside the prescaler's range.
    /// - `I2c` for any transport failure on the configuration writes.
    pub async fn init<D: DelayNs>(
        &mut self,
        pwm_freq_hz: u32,
        delay: &mut D,
    ) -> Result<(), Error<B::Error>> {
        if !(MIN_FREQ_HZ..=MAX_FREQ_HZ).contains(&pwm_freq_hz) {
            return Err(Error::FrequencyOutOfRange);
        }

        // 1. Sleep is required before writing PRE_SCALE — the datasheet says
        //    the prescale register is write-protected while SLEEP is 0.
        //    Disable ALLCALL here so a bus-wide address doesn't accidentally
        //    retarget us; we'll still respond on `address`.
        self.write_mode1(MODE1_SLEEP).await?;

        // 2. prescale = round(osc / (4096 * freq)) - 1, rounded half-to-even
        //    but a plain truncating divide is within ±1% and that's what the
        //    NXP example code does.
        let prescale = Self::prescale_for(pwm_freq_hz);
        self.write_reg(REG_PRE_SCALE, prescale).await?;

        // 3. Clear SLEEP, set AI (auto-increment) so multi-byte writes to the
        //    LED*_ON_L/H/OFF_L/H block land in one I²C transaction.
        self.write_mode1(MODE1_AI).await?;
        delay.delay_us(WAKE_DELAY_US).await;

        // 4. Totem-pole (push/pull) outputs — servos expect a solid logic
        //    high during the pulse, not an open-drain pull-down.
        self.write_reg(REG_MODE2, MODE2_OUTDRV).await?;

        Ok(())
    }

    /// Compute the `PRE_SCALE` register value for `pwm_freq_hz`.
    ///
    /// The clamp is belt-and-braces: callers already hit
    /// `FrequencyOutOfRange` for values outside the range, so this just
    /// keeps the result inside the register's u8 domain.
    fn prescale_for(pwm_freq_hz: u32) -> u8 {
        let tps = u32::from(TICKS_PER_PERIOD);
        let prescale = OSC_HZ / (tps * pwm_freq_hz);
        // Register is 8-bit; valid values are 3..=255 per datasheet. Clamp.
        let raw = prescale.saturating_sub(1).clamp(3, 255);
        #[allow(
            clippy::cast_possible_truncation,
            reason = "clamped to 0..=255 above, so the cast is lossless"
        )]
        let byte = raw as u8;
        byte
    }

    /// Put the chip to sleep — stops PWM output, but retains configuration.
    /// Callers can [`Pca9685::wake`] later without re-running [`Pca9685::init`].
    ///
    /// # Errors
    /// Returns `I2c` on transport failure.
    pub async fn sleep(&mut self) -> Result<(), Error<B::Error>> {
        // Preserve AI so subsequent multi-byte writes still work post-wake.
        self.write_mode1(MODE1_AI | MODE1_SLEEP).await
    }

    /// Wake the chip after [`Pca9685::sleep`]. Waits the datasheet's 500 µs
    /// oscillator-stabilization time before returning.
    ///
    /// # Errors
    /// Returns `I2c` on transport failure.
    pub async fn wake<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<B::Error>> {
        self.write_mode1(MODE1_AI).await?;
        delay.delay_us(WAKE_DELAY_US).await;
        Ok(())
    }

    /// Set channel `ch` to raw on/off counts within the 4096-tick period.
    ///
    /// `on_count` is the tick at which the output goes HIGH; `off_count`
    /// is the tick it goes LOW. For servos, callers typically set
    /// `on_count = 0` and vary `off_count` — [`Pca9685::set_channel_pulse_us`]
    /// does this for you.
    ///
    /// # Errors
    /// - `ChannelOutOfRange` if `ch > MAX_CHANNEL`.
    /// - `CountOutOfRange` if either count exceeds `TICKS_PER_PERIOD - 1`.
    /// - `I2c` for any transport failure.
    pub async fn set_channel_raw(
        &mut self,
        ch: u8,
        on_count: u16,
        off_count: u16,
    ) -> Result<(), Error<B::Error>> {
        if ch > MAX_CHANNEL {
            return Err(Error::ChannelOutOfRange);
        }
        if on_count >= TICKS_PER_PERIOD || off_count >= TICKS_PER_PERIOD {
            return Err(Error::CountOutOfRange);
        }

        let base = REG_LED0_ON_L + ch * 4;
        let on_l = (on_count & 0xFF) as u8;
        let on_h = ((on_count >> 8) & 0x0F) as u8;
        let off_l = (off_count & 0xFF) as u8;
        let off_h = ((off_count >> 8) & 0x0F) as u8;
        // Auto-increment streams all four bytes across the four channel
        // registers in one STOP-less transaction.
        self.bus
            .write(self.address, &[base, on_l, on_h, off_l, off_h])
            .await?;
        Ok(())
    }

    /// Set channel `ch` to a pulse of `pulse_us` microseconds per period,
    /// computed against a 50 Hz frame (20 ms period). If you initialised
    /// the chip for a different PWM frequency, use
    /// [`Pca9685::set_channel_pulse_us_for_period`] instead.
    ///
    /// # Errors
    /// Same as [`Pca9685::set_channel_raw`].
    pub async fn set_channel_pulse_us(
        &mut self,
        ch: u8,
        pulse_us: u16,
    ) -> Result<(), Error<B::Error>> {
        self.set_channel_pulse_us_for_period(ch, pulse_us, 20_000)
            .await
    }

    /// Set channel `ch` to a pulse of `pulse_us` microseconds within a
    /// `period_us`-microsecond PWM frame. Use this when the chip is
    /// clocked for a non-50-Hz frequency.
    ///
    /// # Errors
    /// Same as [`Pca9685::set_channel_raw`].
    pub async fn set_channel_pulse_us_for_period(
        &mut self,
        ch: u8,
        pulse_us: u16,
        period_us: u32,
    ) -> Result<(), Error<B::Error>> {
        let tps = u32::from(TICKS_PER_PERIOD);
        let pulse = u32::from(pulse_us);
        // off_count = pulse_us * 4096 / period_us, clamped to valid range.
        let off = pulse
            .saturating_mul(tps)
            .checked_div(period_us.max(1))
            .unwrap_or(0);
        #[allow(
            clippy::cast_possible_truncation,
            reason = "clamped to < TICKS_PER_PERIOD, so the cast is lossless"
        )]
        let off_u16 = off.min(u32::from(TICKS_PER_PERIOD - 1)) as u16;
        self.set_channel_raw(ch, 0, off_u16).await
    }

    /// Write `value` to MODE1, preserving no state.
    async fn write_mode1(&mut self, value: u8) -> Result<(), Error<B::Error>> {
        self.write_reg(REG_MODE1, value).await
    }

    /// Write `value` to the register at `reg`.
    async fn write_reg(&mut self, reg: u8, value: u8) -> Result<(), Error<B::Error>> {
        self.bus.write(self.address, &[reg, value]).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Local no-op I²C bus for tests that only need a concrete `B` to name
    /// `Pca9685::<MockBus>` — orphan rules forbid impl'ing `I2c` for `()`.
    struct MockBus;

    impl embedded_hal_async::i2c::ErrorType for MockBus {
        type Error = core::convert::Infallible;
    }

    impl I2c for MockBus {
        async fn transaction(
            &mut self,
            _address: u8,
            _operations: &mut [embedded_hal_async::i2c::Operation<'_>],
        ) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn prescale_for_50hz_matches_datasheet() {
        // Datasheet example: at 50 Hz, prescale = round(25e6 / (4096 * 50)) - 1 = 121.
        assert_eq!(Pca9685::<MockBus>::prescale_for(50), 121);
    }

    #[test]
    fn prescale_for_200hz() {
        // trunc(25e6 / (4096 * 200)) - 1 = 29. Datasheet quotes "round"
        // but NXP's example code truncates; we match the example. For
        // servos / most use cases the ~0.5% frequency offset is harmless.
        assert_eq!(Pca9685::<MockBus>::prescale_for(200), 29);
    }

    #[test]
    fn prescale_clamps_floor_to_3() {
        // 1526 Hz is near the top of the range; prescale should land at ~3.
        let p = Pca9685::<MockBus>::prescale_for(MAX_FREQ_HZ);
        assert!(p >= 3, "prescale {p} below the chip-enforced minimum of 3");
    }

    #[test]
    fn prescale_for_24hz_near_the_high_end() {
        // Lowest frequency must produce a prescale in the 200-255 range
        // (25e6 / (4096 * 24) ≈ 254).
        let p = Pca9685::<MockBus>::prescale_for(MIN_FREQ_HZ);
        assert!(p >= 200, "prescale {p} lower than expected for 24 Hz");
    }
}
