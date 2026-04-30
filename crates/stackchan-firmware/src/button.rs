//! AXP2101 power-button polling task — short-tap + long-press routing.
//!
//! Polls the AXP2101's `IRQ_STATUS_1` at 50 ms (20 Hz) for the
//! [`IRQ_PRESS_EDGE_BIT`] / [`IRQ_RELEASE_EDGE_BIT`] flags
//! ([`axp2101::IRQ_PRESS_EDGE_BIT`] etc), times the gap in software,
//! and routes the result:
//!
//! - **Short tap** (< [`LONG_PRESS_MS`]): forwarded to
//!   [`crate::touch::TAP_SIGNAL`] so the power button behaves as a
//!   second tap source — UX-identical to a touchscreen tap (cycle
//!   emotion + 30 s pin).
//! - **Long press** (≥ [`LONG_PRESS_MS`]): published to
//!   [`crate::camera::CAMERA_MODE_SIGNAL`] as the inverse of the
//!   currently-active mode (toggle). Fires *the moment the threshold
//!   is crossed* while still pressed, so the user gets immediate
//!   feedback rather than waiting for release.
//!
//! Why software timing? The chip's built-in long-press IRQ has a
//! ≥ 2 s minimum threshold (`SYS_CTL2[3] = 0` → 2 s, `1` → 3 s) — too
//! slow for a UI toggle. Polling both edges and timing the gap costs
//! one extra register read per tick and gives us arbitrary granularity.
//!
//! ## Error handling
//!
//! Init failure (IRQ enable) logs at `error` and parks the task —
//! behavior silently degrades to "no button input." Runtime bus
//! glitches log at `warn` and skip the tick.

use axp2101::Axp2101;
use embassy_time::{Duration, Instant, Ticker, Timer};
use embedded_hal_async::i2c::I2c as AsyncI2c;

use crate::camera::CAMERA_MODE_SIGNAL;
use crate::touch;

/// Poll cadence. 50 ms = 20 Hz; responsive enough for a button,
/// light enough on the bus.
const POLL_PERIOD_MS: u64 = 50;

/// Long-press threshold.
///
/// Holding the power button this long while the task observes a
/// press-edge fires a camera-mode toggle. 600 ms is the standard
/// mobile-UX long-press window — well clear of an accidental hold
/// and well below AXP2101's ~4 s hardware shutdown timer at
/// `SYS_CTL[2]`.
pub const LONG_PRESS_MS: u64 = 600;

/// Enable the press- and release-edge IRQs, then loop forever
/// classifying each press as either a short tap or a long press.
pub async fn run_button_loop<I: AsyncI2c>(bus: I) -> ! {
    let mut pmic = Axp2101::new(bus);
    if let Err(e) = pmic.enable_power_key_edge_irqs().await {
        defmt::error!(
            "AXP2101 button: enable edge IRQs failed ({}); power button disabled",
            defmt::Debug2Format(&e),
        );
        park().await;
    }
    defmt::info!(
        "AXP2101 button: edge IRQs enabled; polling @ {=u64} ms tick, long-press @ {=u64} ms",
        POLL_PERIOD_MS,
        LONG_PRESS_MS,
    );

    let mut press_started: Option<Instant> = None;
    let mut long_press_fired = false;
    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));

    loop {
        match pmic.take_power_key_edges().await {
            Ok((press, release)) => {
                if press {
                    press_started = Some(Instant::now());
                    long_press_fired = false;
                }

                // While the button is held: check whether the
                // long-press threshold has been crossed and we haven't
                // already fired the toggle for this hold. Firing on
                // crossing (rather than waiting for release) gives
                // immediate feedback the moment the user has held
                // long enough — the existing tap pipeline never sees
                // this press because release is still in the future.
                if let Some(started) = press_started
                    && !long_press_fired
                    && started.elapsed().as_millis() >= LONG_PRESS_MS
                {
                    // Read the canonical state from the snapshot rather
                    // than tracking a local mirror — HTTP `POST
                    // /camera/mode` and the BLE view-service write also
                    // flip this field, and a stale local would invert
                    // their value on the next long-press, swallowing
                    // the user's button press.
                    let next = !crate::net::snapshot::read().camera_mode;
                    defmt::info!("AXP2101 button: long-press → camera_mode={=bool}", next);
                    crate::net::snapshot::update_camera_mode(next);
                    CAMERA_MODE_SIGNAL.signal(next);
                    long_press_fired = true;
                }

                if release {
                    let duration_ms = press_started.map_or(0, |t| t.elapsed().as_millis());
                    if !long_press_fired {
                        defmt::debug!(
                            "AXP2101 button: short-press ({=u64} ms) → TAP_SIGNAL",
                            duration_ms,
                        );
                        touch::TAP_SIGNAL.signal(());
                    }
                    press_started = None;
                    long_press_fired = false;
                }
            }
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
