//! LED-ring bench: brings up shared I²C, initialises the PY32 LED
//! fan-out, and cycles through each [`Emotion`] palette entry every
//! 2 seconds to verify on-device that:
//!
//! 1. The PY32 responds at I²C `0x6F` and accepts `set_led_count(12)`.
//! 2. `write_led_pixels` + `refresh_leds` produce a visible change on
//!    the WS2812 ring.
//! 3. `stackchan_core::render_leds` produces distinguishable colours
//!    through the warm/cool palette with breath-envelope modulation.
//!
//! The bench does **not** depend on the render pipeline; it drives the
//! LED path directly so a regression in the firmware's modifier stack
//! won't mask a hardware or transport fault.
//!
//! Output per tick (via defmt):
//!
//! ```text
//! leds-bench: emotion=Happy frame[0]=0xFBE0 brightness=102
//! ```

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use embassy_time::{Delay, Duration, Ticker};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use stackchan_core::{Avatar, Emotion, Instant as CoreInstant, LED_COUNT, LedFrame, render_leds};
use stackchan_firmware::board;

use esp_println as _;

defmt::timestamp!("{=u64} ms", embassy_time::Instant::now().as_millis());

esp_bootloader_esp_idf::esp_app_desc!();

/// LTO anchor preventing `ESP_APP_DESC` from being stripped. See
/// `main.rs` for the full rationale.
#[used]
static _APP_DESC_ANCHOR: &esp_bootloader_esp_idf::EspAppDesc = &ESP_APP_DESC;

/// Panic handler for the bench binary.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("leds-bench panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Heap size: no framebuffer, small drivers, embassy task arena.
const HEAP_SIZE: usize = 32 * 1024;

/// Per-frame push cadence. 30 Hz matches main render task so the
/// breath envelope looks identical to the real firmware.
const PUSH_PERIOD_MS: u64 = 33;

/// Hold each emotion for 2 seconds before advancing. Long enough to
/// visually confirm the palette entry is correct.
const EMOTION_HOLD_MS: u64 = 2_000;

/// Fixed cycle of emotions driven by this bench. Neutral → Happy →
/// Sad → Sleepy → Surprised → (loop).
const EMOTIONS: [Emotion; 5] = [
    Emotion::Neutral,
    Emotion::Happy,
    Emotion::Sad,
    Emotion::Sleepy,
    Emotion::Surprised,
];

#[esp_rtos::main]
#[allow(
    clippy::used_underscore_binding,
    reason = "the esp_rtos::main macro requires the `spawner` arg; leds-bench doesn't spawn"
)]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-leds-bench v{} — CoreS3 boot, cycling emotion palette",
        env!("CARGO_PKG_VERSION")
    );

    let mut delay = Delay;
    let board_io = board::bringup(
        peripherals.I2C0,
        peripherals.UART1,
        peripherals.GPIO12,
        peripherals.GPIO11,
        peripherals.GPIO6,
        peripherals.GPIO7,
        &mut delay,
    )
    .await;

    let mut py = py32::Py32::new(board_io.i2c);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "LED_COUNT is a compile-time constant well under u8::MAX"
    )]
    let led_count = LED_COUNT as u8;
    match py.set_led_count(led_count).await {
        Ok(()) => defmt::info!("leds-bench: PY32 set_led_count({=u8}) ok", led_count),
        Err(e) => defmt::panic!(
            "leds-bench: PY32 set_led_count failed: {}",
            defmt::Debug2Format(&e)
        ),
    }

    let mut ticker = Ticker::every(Duration::from_millis(PUSH_PERIOD_MS));
    let mut frame = LedFrame::default();
    let mut avatar = Avatar::default();
    let mut emotion_idx: usize = 0;
    let mut hold_counter: u64 = 0;
    let hold_ticks = EMOTION_HOLD_MS.div_ceil(PUSH_PERIOD_MS);

    loop {
        // `now` drives the breath envelope so brightness pulses the
        // same way it does in the real firmware.
        let now = CoreInstant::from_millis(embassy_time::Instant::now().as_millis());
        avatar.emotion = EMOTIONS[emotion_idx];
        render_leds(&avatar, now, &mut frame);

        if let Err(e) = py.write_led_pixels(frame.as_u16_slice()).await {
            defmt::warn!(
                "leds-bench: write_led_pixels failed: {}",
                defmt::Debug2Format(&e)
            );
        } else if let Err(e) = py.refresh_leds().await {
            defmt::warn!(
                "leds-bench: refresh_leds failed: {}",
                defmt::Debug2Format(&e)
            );
        }

        hold_counter = hold_counter.saturating_add(1);
        if hold_counter >= hold_ticks {
            hold_counter = 0;
            emotion_idx = (emotion_idx + 1) % EMOTIONS.len();
            defmt::info!(
                "leds-bench: emotion={=?} frame[0]=0x{=u16:04X}",
                defmt::Debug2Format(&avatar.emotion),
                frame.0[0],
            );
        }

        ticker.next().await;
    }
}
