//! FT6336U touch-polling task + rising-edge tap signal.
//!
//! Polls the capacitive touch controller over the shared I²C0 bus at
//! ~60 Hz (matching the FT6336U's native sample rate), detects the
//! 0→1 finger-count rising edge, and publishes a pulse on
//! [`TAP_SIGNAL`]. The render task consumes the signal on each tick
//! and feeds it into [`stackchan_core::modifiers::EmotionTouch`].
//!
//! ## Why signal and not a richer event?
//!
//! The MVP treats every tap as equivalent: the `EmotionTouch` modifier
//! only cares that *a* tap happened, not where on the screen. Sending
//! `Signal<_, ()>` keeps this decoupling explicit — if we ever want
//! zone-aware behaviour, growing the signal to `TouchEvent { x, y }`
//! is a local change and the render-side consumer is still a single
//! `try_take` on the signal.
//!
//! ## Boot probe
//!
//! On startup the task reads the FT6336U vendor ID once and logs the
//! result. A miswired bus or missing part surfaces as a warn-level
//! line here, but the task keeps running — the rest of the firmware
//! (render, head) doesn't depend on touch, so silent degradation is
//! acceptable.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Ticker};
use embedded_hal_async::i2c::I2c as AsyncI2c;
use ft6336u::{Ft6336u, VENDOR_ID_FOCALTECH};

/// Rising-edge tap signal: touch task → render task.
///
/// The touch task calls [`Signal::signal`] once per detected
/// 0→1 finger-count transition. The render task consumes via
/// [`Signal::try_take`] on each render tick. Signal semantics (latest
/// wins, no backlog) mean multiple taps between render ticks collapse
/// into a single `tap()` call — fine for tap-to-cycle UX; a future
/// queue could replace the signal if buffering is needed.
pub static TAP_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Poll cadence for the touch task. 60 Hz matches the controller's
/// native sample rate; polling faster buys no latency (reads would
/// just return the same value twice) and wastes bus time that the
/// future RTC / IMU tasks will need.
const POLL_PERIOD_MS: u64 = 16;

/// Drive the FT6336U touch-polling loop.
///
/// Takes an owned I²C device (a shared-bus [`I2cDevice`](embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice)
/// handle is fine), not a borrow, so the task outlives any stack frame
/// in `main`. Runs the boot probe, then loops forever publishing tap
/// edges.
///
/// Bus errors at poll time log at `warn` and are skipped — a single
/// dropped I²C transaction must not blank the face.
pub async fn run_touch_loop<I: AsyncI2c>(mut touch: Ft6336u<I>) -> ! {
    match touch.read_vendor_id().await {
        // Genuine FocalTech part.
        Ok(VENDOR_ID_FOCALTECH) => {
            defmt::info!(
                "FT6336U: vendor ID 0x{=u8:02X} (FocalTech, expected)",
                VENDOR_ID_FOCALTECH,
            );
        }
        // CoreS3 variants ship with register-compatible touch silicon
        // that reports a different vendor byte (0x01 has been observed
        // in the wild). The touch-coordinate registers are identical,
        // so taps work fine — no need to alarm the log.
        Ok(0x01) => {
            defmt::info!("FT6336U: vendor ID 0x01 (CoreS3 variant, register-compatible)");
        }
        Ok(id) => defmt::warn!(
            "FT6336U: unexpected vendor ID 0x{=u8:02X} (expected 0x{=u8:02X} or 0x01); taps may misbehave",
            id,
            VENDOR_ID_FOCALTECH,
        ),
        Err(e) => defmt::warn!(
            "FT6336U: vendor-ID read failed (is the chip present?): {}",
            defmt::Debug2Format(&e),
        ),
    }

    defmt::info!(
        "touch task: {=u64} ms tick, publishing rising-edge taps to TAP_SIGNAL",
        POLL_PERIOD_MS,
    );

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    let mut previous_fingers: u8 = 0;
    loop {
        match touch.read_touch().await {
            Ok(report) => {
                // Rising edge: previous tick had no finger down, this
                // tick has ≥1. Held fingers stay at `fingers >= 1` and
                // don't re-fire; release-and-retouch counts as a new
                // rising edge.
                if previous_fingers == 0 && report.fingers > 0 {
                    TAP_SIGNAL.signal(());
                    if let Some((x, y)) = report.first {
                        defmt::debug!("touch: tap at ({=u16}, {=u16})", x, y);
                    } else {
                        defmt::debug!("touch: tap (no coord in report)");
                    }
                }
                previous_fingers = report.fingers;
            }
            Err(e) => {
                defmt::warn!("touch: read_touch failed: {}", defmt::Debug2Format(&e));
                // Don't latch a transient bus error as "finger still
                // down" — that would swallow the next real release.
                previous_fingers = 0;
            }
        }
        ticker.next().await;
    }
}
