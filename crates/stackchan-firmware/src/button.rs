//! AXP2101 power-button polling task.
//!
//! The AXP2101 latches power-key events into its `IRQ_STATUS_1`
//! register. This task polls at 50 ms (20 Hz) for the short-press
//! edge, clears it atomically (write-1-to-clear), and forwards each
//! edge to [`touch::TAP_SIGNAL`] so the power button becomes a second
//! tap source — user-facing behavior is identical to a touchscreen
//! tap (cycle emotion + pin for 30 s).
//!
//! Why polling and not the AXP2101's IRQ pin? The IRQ line isn't
//! broken out on CoreS3 in a way that's documented in our memory /
//! datasheet cheat-sheet, and a 20 Hz polling bus lookup costs
//! essentially nothing on the shared I²C bus (one register read per
//! tick, shared across other consumers by the async `I2cDevice`
//! mutex).
//!
//! ## Error handling
//!
//! Init failure (IRQ enable) logs at `error` and parks the task —
//! behavior silently degrades to "no button input," matching the
//! pattern used by `ambient` and `imu`. Runtime bus glitches log at
//! `warn` and skip the tick.

use axp2101::Axp2101;
use embassy_time::{Duration, Ticker, Timer};
use embedded_hal_async::i2c::I2c as AsyncI2c;

use crate::touch;

/// Poll cadence. 50 ms = 20 Hz; responsive enough for a button,
/// light enough on the bus.
const POLL_PERIOD_MS: u64 = 50;

/// Enable the short-press IRQ then loop forever forwarding edges to
/// [`touch::TAP_SIGNAL`].
pub async fn run_button_loop<I: AsyncI2c>(bus: I) -> ! {
    let mut pmic = Axp2101::new(bus);
    if let Err(e) = pmic.enable_power_key_short_press_irq().await {
        defmt::error!(
            "AXP2101 button: enable short-press IRQ failed ({}); power button disabled",
            defmt::Debug2Format(&e),
        );
        park().await;
    }
    defmt::info!(
        "AXP2101 button: short-press IRQ enabled; polling @ {=u64} ms tick",
        POLL_PERIOD_MS,
    );

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    loop {
        match pmic.check_short_press_edge().await {
            Ok(true) => {
                defmt::debug!("AXP2101: power-key short-press edge -> TAP_SIGNAL");
                touch::TAP_SIGNAL.signal(());
            }
            Ok(false) => {}
            Err(e) => defmt::warn!(
                "AXP2101 button: status read failed: {}",
                defmt::Debug2Format(&e),
            ),
        }
        ticker.next().await;
    }
}

/// Idle loop for the post-failure path.
async fn park() -> ! {
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
