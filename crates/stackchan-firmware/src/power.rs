//! AXP2101 battery / power-monitor polling task.
//!
//! Polls the PMIC at 1 Hz for battery `SoC` percent and USB-power
//! presence, publishing both as one [`PowerStatus`] struct on
//! [`POWER_STATUS_SIGNAL`]. The render task drains it into
//! `avatar.battery_percent` and `avatar.usb_power_present`, where
//! [`stackchan_core::modifiers::EmotionFromBattery`] picks up both:
//! the percent drives the hysteresis state machine, and the USB-good
//! bit suppresses the Sleepy override while the unit is charging.
//!
//! The AXP2101 is already initialised (`init_cores3`) by the main
//! boot sequence — this task only consumes the chip via reads and
//! never touches LDO / battery-detect config.
//!
//! Failure mode: I²C read errors log at `warn` and we keep polling.
//! A persistently-failing AXP2101 means `POWER_STATUS_SIGNAL` never
//! publishes, so the avatar fields stay `None` and the
//! `EmotionFromBattery` modifier silently no-ops. Other power-related
//! features (LDO rails) are unaffected — the chip itself is fine,
//! just the gauge / VBUS readback isn't reaching the avatar.

use axp2101::Axp2101;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Ticker};
use embedded_hal_async::i2c::I2c as AsyncI2c;

/// Snapshot of the AXP2101 power state, published once per poll tick.
///
/// `usb_power` reflects the chip's `VBUSGD` flag; it asserts whenever
/// the USB-C input has a valid voltage, regardless of whether the
/// charger is actively pushing current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerStatus {
    /// Battery state-of-charge in percent (`0..=100`).
    pub battery_percent: u8,
    /// `true` when valid USB voltage is present.
    pub usb_power: bool,
}

/// Latest [`PowerStatus`] reading, single-producer (this task) →
/// single-consumer (the render task's drain at the top of each tick).
///
/// Latest-wins matches the consumer's need: a render task that misses
/// one publish should see the next one, not a backlog.
pub static POWER_STATUS_SIGNAL: Signal<CriticalSectionRawMutex, PowerStatus> = Signal::new();

/// Poll cadence. The AXP2101's gauge register only changes on the
/// minute-scale during normal discharge — 1 Hz is luxurious for
/// charge-tracking and lets the modifier react within ~1 s of a
/// threshold crossing or USB plug/unplug without spamming the bus.
const POLL_PERIOD_MS: u64 = 1_000;

/// Run the AXP2101 battery-monitor loop forever. `init_cores3` must
/// have already run on this bus — that's wired up in the main boot
/// sequence before any task spawns.
pub async fn run_power_loop<I: AsyncI2c>(bus: I) -> ! {
    let mut pmic = Axp2101::new(bus);
    defmt::info!(
        "AXP2101 power monitor: polling battery + USB @ {=u64} ms tick",
        POLL_PERIOD_MS,
    );

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    // Track last-published so the diagnostic log emits only on change
    // (otherwise this would log every poll tick).
    let mut last_published: Option<PowerStatus> = None;
    loop {
        crate::watchdog::POWER.beat();
        let Some(status) = read_status(&mut pmic).await else {
            ticker.next().await;
            continue;
        };

        POWER_STATUS_SIGNAL.signal(status);

        if last_published != Some(status) {
            defmt::info!(
                "AXP2101: battery {=u8}%, USB {=bool}",
                status.battery_percent,
                status.usb_power,
            );
            last_published = Some(status);
        }

        ticker.next().await;
    }
}

/// Read both battery and USB-power state in one tick, returning
/// `None` on any I²C failure. Logged-and-degraded inside the helper
/// so the caller's loop body stays tight.
async fn read_status<B: AsyncI2c>(pmic: &mut Axp2101<B>) -> Option<PowerStatus> {
    let battery_percent = pmic
        .read_battery_percent()
        .await
        .inspect_err(|e| {
            defmt::warn!(
                "AXP2101: battery-gauge read failed ({:?})",
                defmt::Debug2Format(e),
            );
        })
        .ok()?;
    let usb_power = pmic
        .read_usb_power_good()
        .await
        .inspect_err(|e| {
            defmt::warn!(
                "AXP2101: USB-power read failed ({:?})",
                defmt::Debug2Format(e),
            );
        })
        .ok()?;
    Some(PowerStatus {
        battery_percent,
        usb_power,
    })
}
