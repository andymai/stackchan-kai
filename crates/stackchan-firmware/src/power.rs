//! AXP2101 battery / power-monitor polling task.
//!
//! Polls the PMIC's state-of-charge gauge at 1 Hz and publishes the
//! latest percentage on [`BATTERY_PERCENT_SIGNAL`]. The render task
//! drains the signal into `avatar.battery_percent`, where
//! [`stackchan_core::modifiers::LowBatteryEmotion`] picks it up and
//! flips emotion to Sleepy when the `SoC` drops below threshold.
//!
//! The AXP2101 is already initialised (`init_cores3`) by the main
//! boot sequence — this task only consumes the chip via reads and
//! never touches LDO / battery-detect config.
//!
//! Failure mode: I²C read errors log at `warn` and we keep polling.
//! A persistently-failing AXP2101 means `BATTERY_PERCENT_SIGNAL`
//! never publishes, so `avatar.battery_percent` stays `None` and the
//! `LowBatteryEmotion` modifier silently no-ops. Other power-related
//! features (LDO rails) are unaffected — the chip itself is fine,
//! just the gauge readback isn't reaching the avatar.

use axp2101::Axp2101;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Ticker};
use embedded_hal_async::i2c::I2c as AsyncI2c;

/// Latest battery state-of-charge in percent, `0..=100`.
///
/// Single producer (this task) → single consumer (the render task's
/// drain at the top of each tick). Latest-wins matches the consumer's
/// need: a render task that misses one publish should see the next
/// one, not a backlog.
pub static BATTERY_PERCENT_SIGNAL: Signal<CriticalSectionRawMutex, u8> = Signal::new();

/// Poll cadence. The AXP2101's gauge register only changes on the
/// minute-scale during normal discharge — 1 Hz is luxurious for
/// charge-tracking and lets the modifier react within ~1 s of a
/// threshold crossing without spamming the I²C bus.
const POLL_PERIOD_MS: u64 = 1_000;

/// Run the AXP2101 battery-monitor loop forever. `init_cores3` must
/// have already run on this bus — that's wired up in the main boot
/// sequence before any task spawns.
pub async fn run_power_loop<I: AsyncI2c>(bus: I) -> ! {
    let mut pmic = Axp2101::new(bus);
    defmt::info!(
        "AXP2101 power monitor: polling battery gauge @ {=u64} ms tick",
        POLL_PERIOD_MS,
    );

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    let mut last_published: Option<u8> = None;
    loop {
        match pmic.read_battery_percent().await {
            Ok(percent) => {
                BATTERY_PERCENT_SIGNAL.signal(percent);
                // Log only when the value changes by a noticeable
                // amount, otherwise this would emit one info per
                // second. 1% steps are plenty for human inspection.
                if last_published != Some(percent) {
                    defmt::info!("AXP2101: battery {=u8}%", percent);
                    last_published = Some(percent);
                }
            }
            Err(e) => defmt::warn!(
                "AXP2101: battery-gauge read failed ({:?})",
                defmt::Debug2Format(&e),
            ),
        }
        ticker.next().await;
    }
}
