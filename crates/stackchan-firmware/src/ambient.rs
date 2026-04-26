//! LTR-553 ambient-light polling task.
//!
//! Polls the chip at 2 Hz (plenty for ambient — the human eye doesn't
//! adapt fast enough to care about higher rates) and publishes the
//! latest lux estimate on [`AMBIENT_LUX_SIGNAL`]. The render task
//! drains the signal, writes `avatar.ambient_lux`, and runs
//! [`stackchan_core::modifiers::EmotionFromAmbient`].
//!
//! ## Boot probe
//!
//! Reads `PART_ID` once at startup and logs the outcome. An unexpected
//! part identifier logs at `warn`; I²C transport failures log at
//! `error` and drop the task into an idle loop. The rest of the
//! firmware keeps running — ambient-driven behavior silently
//! degrades into "never sleepy" without affecting touch, pickup, or
//! the render stack.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Ticker, Timer};
use embedded_hal_async::i2c::I2c as AsyncI2c;
use ltr553::Ltr553;

/// Latest ambient-light reading: ambient task → render task. Lux only
/// — the render-side consumer doesn't care about raw channels (those
/// are available via `examples/ambient_bench.rs` for calibration).
pub static AMBIENT_LUX_SIGNAL: Signal<CriticalSectionRawMutex, f32> = Signal::new();

/// Poll cadence. 500 ms matches the LTR-553's default measurement
/// rate (the chip updates its internal channels every 500 ms), so
/// reading faster just returns duplicates.
const POLL_PERIOD_MS: u64 = 500;

/// Run the LTR-553 init sequence then loop forever publishing lux
/// values.
pub async fn run_ambient_loop<I: AsyncI2c>(bus: I) -> ! {
    let mut als = Ltr553::new(bus);

    if let Err(e) = als.init().await {
        defmt::error!(
            "LTR-553: init failed ({}); ambient-driven behaviors disabled",
            defmt::Debug2Format(&e),
        );
        park().await;
    }
    defmt::info!(
        "LTR-553: init complete — polling @ {=u64} ms tick",
        POLL_PERIOD_MS,
    );

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    loop {
        crate::watchdog::AMBIENT.beat();
        match als.read_ambient().await {
            Ok(reading) => {
                AMBIENT_LUX_SIGNAL.signal(reading.lux);
                defmt::debug!(
                    "LTR-553: ch0={=u16} ch1={=u16} lux={=f32}",
                    reading.ch0,
                    reading.ch1,
                    reading.lux,
                );
            }
            Err(e) => defmt::warn!("LTR-553: read failed: {}", defmt::Debug2Format(&e),),
        }
        ticker.next().await;
    }
}

/// Idle loop for the post-failure path. Keeps the task alive without
/// further bus traffic.
async fn park() -> ! {
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
