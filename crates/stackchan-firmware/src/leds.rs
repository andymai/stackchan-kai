//! LED-ring transport task.
//!
//! Owns the `py32::Py32` driver for the 12× WS2812C ring on CoreS3.
//! Drains [`LED_FRAME_SIGNAL`] at 30 Hz and pushes each frame to the
//! PY32 IO expander, which fans out WS2812 timing internally. The
//! render task (the authoritative owner of `Avatar`) runs
//! `stackchan_core::render_leds` after the modifier stack and publishes
//! into the signal — this task is pure transport.
//!
//! ## Boot
//!
//! On first entry the task runs a brief fade-in from black to the
//! Neutral palette peak (~250 ms, 8 steps) so there's no harsh LED
//! flash at power-up, and so "boot completed cleanly" has a visible
//! signal even if the LCD is slow to light.
//!
//! ## Error handling
//!
//! Every I²C transaction failure is logged at `warn` and the loop
//! continues. A briefly absent PY32 shouldn't drag the rest of the
//! firmware down — modifiers, head motion, and the LCD render task
//! keep running whether or not the ring is alive.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Ticker, Timer};
use stackchan_core::{Avatar, Instant, LED_COUNT, LedFrame, render_leds};

use crate::board::SharedI2c;

/// Latest `LedFrame`, render task → led task.
///
/// Latest-wins semantics: the render task publishes every tick via
/// [`Signal::signal`], which overwrites any un-consumed value. The led
/// task drains with [`Signal::try_take`] and ships the frame over I²C.
/// Dropped frames are fine — the next one is already in flight.
///
/// The render task publishes unconditionally; during the boot fade-in
/// the led task simply doesn't call `try_take`, so published values
/// are overwritten until fade completes. No gating flag needed.
pub static LED_FRAME_SIGNAL: Signal<CriticalSectionRawMutex, LedFrame> = Signal::new();

/// Number of fade-in steps from black to Neutral palette peak. 8 steps
/// over 250 ms lands one step per ~31 ms, matching the 30 Hz cadence
/// of the main render loop.
const FADE_STEPS: u8 = 8;
/// Total duration of the boot fade-in, in milliseconds.
const FADE_TOTAL_MS: u64 = 250;
/// LED ring push rate after fade-in. Matches the LCD render task.
const PUSH_RATE_HZ: u64 = 30;

/// Run the LED-ring transport loop forever. Spawned from `main` as an
/// embassy task.
pub async fn run_led_loop(i2c: SharedI2c) -> ! {
    let mut py = py32::Py32::new(i2c);

    // Set LED count once. Failure here isn't fatal — the PY32 might be
    // sluggish on the first write; we'll retry implicitly on the next
    // bulk-write call (the count field stays at its last value).
    let count = u8::try_from(LED_COUNT).unwrap_or(u8::MAX);
    if let Err(e) = py.set_led_count(count).await {
        defmt::warn!("PY32: set_led_count failed: {}", defmt::Debug2Format(&e));
    } else {
        defmt::info!("PY32: LED count set to {=u8}", count);
    }

    // Boot fade-in: black → Neutral palette at peak brightness.
    let target = neutral_peak_frame();
    for step in 1..=FADE_STEPS {
        let faded = scale_frame(&target, step, FADE_STEPS);
        push_frame(&mut py, &faded).await;
        Timer::after(Duration::from_millis(FADE_TOTAL_MS / u64::from(FADE_STEPS))).await;
    }
    defmt::info!("LED ring: boot fade complete, joining render pipeline");

    // Steady-state: drain the signal at 30 Hz and push whatever the
    // render task published. If nothing was published this tick, skip
    // the wire write — the ring holds the last frame.
    let mut ticker = Ticker::every(Duration::from_hz(PUSH_RATE_HZ));
    loop {
        ticker.next().await;
        if let Some(frame) = LED_FRAME_SIGNAL.try_take() {
            push_frame(&mut py, frame.as_u16_slice()).await;
        }
    }
}

/// Push one frame to the ring: bulk write pixel RAM, then latch.
/// Logs `warn` on either transport failure and returns (loop continues).
async fn push_frame<B>(py: &mut py32::Py32<B>, pixels: &[u16])
where
    B: embedded_hal_async::i2c::I2c,
    B::Error: core::fmt::Debug,
{
    if let Err(e) = py.write_led_pixels(pixels).await {
        defmt::warn!("PY32: write_led_pixels failed: {}", defmt::Debug2Format(&e));
        return;
    }
    if let Err(e) = py.refresh_leds().await {
        defmt::warn!("PY32: refresh_leds failed: {}", defmt::Debug2Format(&e));
    }
}

/// Build the "Neutral emotion at breath-peak brightness" frame used as
/// the target colour for the boot fade-in.
fn neutral_peak_frame() -> LedFrame {
    let mut frame = LedFrame::default();
    // Breath peaks at the halfway point of the 6 s cycle.
    render_leds(&Avatar::default(), Instant::from_millis(3_000), &mut frame);
    frame
}

/// Scale every RGB565 channel of `target` by `step / denom` into a
/// fresh output frame. Used for boot fade.
fn scale_frame(target: &LedFrame, step: u8, denom: u8) -> [u16; LED_COUNT] {
    let mut out = [0u16; LED_COUNT];
    let num = u32::from(step);
    let den = u32::from(denom.max(1));
    for (i, &px) in target.0.iter().enumerate() {
        let r = (u32::from(px >> 11) & 0x1F) * num / den;
        let g = (u32::from(px >> 5) & 0x3F) * num / den;
        let b = (u32::from(px) & 0x1F) * num / den;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "packed value fits in 16 bits by construction"
        )]
        {
            out[i] = ((r << 11) | (g << 5) | b) as u16;
        }
    }
    out
}
