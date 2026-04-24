//! BMI270 IMU polling task.
//!
//! Runs the BMI270 init sequence (soft-reset → blob upload → sensor
//! config) on startup, then polls the 12-byte data block at ~100 Hz
//! and publishes each [`Measurement`] on [`IMU_SIGNAL`]. The render
//! task drains the signal and writes `avatar.accel_g` /
//! `avatar.gyro_dps` before running the modifier stack, where
//! [`stackchan_core::modifiers::PickupReaction`] consumes the values.
//!
//! ## Error handling
//!
//! - Boot: init failure is logged at `error` level and the task
//!   halts with an idle loop. Other firmware tasks (render, head,
//!   touch) keep running — motion reactions silently degrade without
//!   blanking the face.
//! - Runtime: transient bus errors log at `warn` and are skipped.
//!   The BMI270 is a polling-only consumer (no interrupts in this
//!   driver), so a missed sample is visible only as one frozen tick.

use bmi270::{Bmi270, Measurement};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Delay, Duration, Ticker};
use embedded_hal_async::i2c::I2c as AsyncI2c;

/// Latest IMU measurement: IMU task → render task.
///
/// Every sample is published here; the render task drains with
/// [`Signal::try_take`] and writes the raw values to the `Avatar`
/// before running modifiers. Signal semantics (latest wins, no
/// backlog) mean the render task always sees the freshest reading.
pub static IMU_SIGNAL: Signal<CriticalSectionRawMutex, Measurement> = Signal::new();

/// Poll cadence for the IMU task. 10 ms = 100 Hz, matching BMI270's
/// default accel ODR configured in `Bmi270::init`. Polling faster
/// than the chip's internal sampling just returns duplicate values.
const POLL_PERIOD_MS: u64 = 10;

/// Run the BMI270 init sequence, then loop forever polling samples
/// and publishing them via [`IMU_SIGNAL`].
///
/// Takes ownership of the wrapped I²C device so the task outlives any
/// stack frame in `main`. Uses [`embassy_time::Delay`] internally for
/// the init-sequence timing requirements.
pub async fn run_imu_loop<I: AsyncI2c>(bus: I) -> ! {
    let mut delay = Delay;
    let Ok(mut imu) = Bmi270::detect(bus, &mut delay).await.inspect(|d| {
        defmt::info!("BMI270: detected at 0x{=u8:02X}", d.address());
    }) else {
        defmt::error!("BMI270: detection failed; IMU-driven behaviors disabled");
        park().await;
    };

    if let Err(e) = imu.init(&mut delay).await {
        defmt::error!(
            "BMI270: init failed ({}); IMU-driven behaviors disabled",
            defmt::Debug2Format(&e),
        );
        park().await;
    }
    defmt::info!(
        "BMI270: init complete — polling @ {=u64} ms tick, publishing to IMU_SIGNAL",
        POLL_PERIOD_MS,
    );

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    loop {
        match imu.read_measurement().await {
            Ok(m) => IMU_SIGNAL.signal(m),
            Err(e) => defmt::warn!("BMI270: read failed: {}", defmt::Debug2Format(&e)),
        }
        ticker.next().await;
    }
}

/// Idle loop for the post-failure path. Keeps the task alive (the
/// embassy executor requires tasks never return unless declared `-> !`
/// explicitly) without any further bus traffic.
async fn park() -> ! {
    loop {
        embassy_time::Timer::after(Duration::from_secs(60)).await;
    }
}
