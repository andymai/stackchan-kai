//! Si12T body-touch task.
//!
//! Polls the back-of-head 3-zone capacitive controller over the shared
//! I²C0 bus at the same 50 ms cadence the M5Stack reference firmware
//! uses, and publishes the per-zone state on [`BODY_TOUCH_SIGNAL`].
//! The render task drains the signal each tick into
//! `entity.perception.body_touch`.
//!
//! Continuous state, not edges — modifiers / skills do their own
//! tap / hold / swipe detection on top.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Delay, Duration, Ticker, Timer};
use embedded_hal_async::i2c::I2c as AsyncI2c;
use si12t::{Intensity, Si12t};
use stackchan_core::BodyTouch;

/// Latest body-touch reading: body-touch task → render task.
///
/// The task replaces the value once per poll. Render-side `try_take`
/// drops misses (latest wins) — same Signal pattern as the other
/// sensor tasks.
pub static BODY_TOUCH_SIGNAL: Signal<CriticalSectionRawMutex, BodyTouch> = Signal::new();

/// Poll cadence — matches the M5Stack reference body-touch task.
const POLL_PERIOD_MS: u64 = 50;

/// Si12T post-init settle. The reference task waits this long after
/// `si12t_setup` before the first read; the chip is in an undefined
/// state until then and reports `0xFF` / `0x3F`.
const POST_INIT_SETTLE_MS: u64 = 200;

/// Drive the Si12T body-touch polling loop.
///
/// Takes an owned I²C device so the task outlives any stack frame in
/// `main`. Init failures log at `warn` and the loop continues — bus
/// errors at poll time also log + skip, never panic. A stuck Si12T
/// must not blank the face.
pub async fn run_body_touch_loop<I: AsyncI2c>(bus: I) -> ! {
    let mut chip = Si12t::new(bus);
    let mut delay = Delay;
    if let Err(e) = chip.init(&mut delay).await {
        defmt::warn!(
            "Si12T: init failed — body-touch will publish stale defaults: {}",
            defmt::Debug2Format(&e),
        );
    } else {
        defmt::info!(
            "Si12T: body-touch task ready (poll {=u64} ms)",
            POLL_PERIOD_MS
        );
    }
    Timer::after_millis(POST_INIT_SETTLE_MS).await;

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    loop {
        match chip.read_touch().await {
            Ok(touch) => {
                BODY_TOUCH_SIGNAL.signal(BodyTouch {
                    left: intensity_u8(touch.intensity.0),
                    centre: intensity_u8(touch.intensity.1),
                    right: intensity_u8(touch.intensity.2),
                });
            }
            Err(e) => defmt::warn!("Si12T: read_touch failed: {}", defmt::Debug2Format(&e),),
        }
        ticker.next().await;
    }
}

/// Convert the driver's `Intensity` enum to the engine's `0..=3`
/// numeric encoding.
const fn intensity_u8(i: Intensity) -> u8 {
    match i {
        Intensity::None => 0,
        Intensity::Low => 1,
        Intensity::Mid => 2,
        Intensity::High => 3,
    }
}
