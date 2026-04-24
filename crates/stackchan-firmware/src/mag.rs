//! BMM150 magnetometer polling task.
//!
//! Runs the BMM150 init sequence (soft-reset → trim readout → regular
//! preset at 10 Hz) on startup, then polls the compensated `(x, y, z)`
//! field in microtesla and publishes each reading on [`MAG_SIGNAL`].
//! The render task drains the signal and writes `avatar.mag_ut`
//! before running the modifier stack. No modifier consumes it yet —
//! this is a data-only landing; the mag-bench example +
//! `Avatar::mag_ut` on-device trace are the characterisation path.
//!
//! ## Error handling
//!
//! - Boot: detect / init failure logs at `error` and parks the task.
//!   Other firmware (render, head, touch, IMU, ambient, LED) keeps
//!   running — the magnetometer is purely informational.
//! - Runtime: transient bus errors log at `warn` and skip the tick;
//!   [`bmm150::Error::Overflow`] logs at `debug` (overflow is a real
//!   chip condition when the field exceeds ADC range, not a fault).

use bmm150::Bmm150;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Delay, Duration, Ticker};
use embedded_hal_async::i2c::I2c as AsyncI2c;

/// Latest compensated magnetometer reading in microtesla: mag task →
/// render task. Tuple layout matches `Avatar::mag_ut` for a trivial
/// `avatar.mag_ut = Some(signal.try_take()?)` in the render loop.
pub static MAG_SIGNAL: Signal<CriticalSectionRawMutex, (f32, f32, f32)> = Signal::new();

/// Poll cadence. 100 ms = 10 Hz, matching the driver's NORMAL-mode
/// regular-preset data rate. Polling faster just returns duplicate
/// internal samples.
const POLL_PERIOD_MS: u64 = 100;

/// Run the BMM150 init sequence, then loop forever polling samples
/// and publishing them via [`MAG_SIGNAL`].
pub async fn run_mag_loop<I: AsyncI2c>(bus: I) -> !
where
    I::Error: core::fmt::Debug,
{
    let mut delay = Delay;
    let mut mag = match Bmm150::detect(bus, &mut delay).await {
        Ok(m) => m,
        Err(e) => {
            defmt::error!(
                "BMM150: detect failed ({}); magnetometer disabled",
                defmt::Debug2Format(&e),
            );
            park().await;
        }
    };
    if let Err(e) = mag.init(&mut delay).await {
        defmt::error!(
            "BMM150: init failed ({}); magnetometer disabled",
            defmt::Debug2Format(&e),
        );
        park().await;
    }
    defmt::info!(
        "BMM150: init complete — polling @ {=u64} ms tick, publishing to MAG_SIGNAL",
        POLL_PERIOD_MS,
    );

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    loop {
        match mag.read_measurement().await {
            Ok(m) => MAG_SIGNAL.signal(m.mag_ut),
            Err(bmm150::Error::Overflow) => {
                defmt::debug!("BMM150: ADC overflow on one axis; sample skipped");
            }
            Err(e) => defmt::warn!("BMM150: read failed: {}", defmt::Debug2Format(&e)),
        }
        ticker.next().await;
    }
}

/// Idle loop for the post-failure path.
async fn park() -> ! {
    loop {
        embassy_time::Timer::after(Duration::from_secs(60)).await;
    }
}
