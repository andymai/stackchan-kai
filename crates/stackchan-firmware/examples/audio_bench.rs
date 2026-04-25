//! Audio bench: brings up the full audio stack (I²S0 + AW88298 + ES7210)
//! exactly as `main.rs` does, then runs a fixed playlist through the
//! TX clip queue so every clip in the chirp library can be ear-tested
//! in isolation.
//!
//! Use this bench when iterating on:
//! - boot-greeting amplitude / duration / pitch
//! - chirp distinguishability (wake vs pickup vs low-battery)
//! - time-of-day greeting variants
//! - any new `AudioClip` you add to `audio.rs`
//!
//! Expected log after bring-up (one playlist run):
//!
//! ```text
//! audio-bench: playing BOOT_GREETING (default)
//! audio-bench: playing BOOT_GREETING_MORNING
//! audio-bench: playing BOOT_GREETING_EVENING
//! audio-bench: playing BOOT_GREETING_NIGHT
//! audio-bench: playing WAKE_CHIRP
//! audio-bench: playing pickup chirp (2 clips)
//! audio-bench: playing low-battery alert (3 clips)
//! audio-bench: playlist done — looping in 5 s
//! ```
//!
//! The playlist loops forever so a single flash gives you continuous
//! sample tones for tuning the AW88298's volume / boost / amplitude.

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Timer};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use stackchan_firmware::{audio, board};

use esp_println as _;

defmt::timestamp!("{=u64} ms", embassy_time::Instant::now().as_millis());

esp_bootloader_esp_idf::esp_app_desc!();

/// LTO anchor preventing `ESP_APP_DESC` from being stripped.
#[used]
static _APP_DESC_ANCHOR: &esp_bootloader_esp_idf::EspAppDesc = &ESP_APP_DESC;

/// Panic handler. Halts the core; esp-rtos emits the trace over RTT
/// before we arrive here.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("audio-bench panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Heap size: the audio task's circular DMA buffers are static, but
/// we still need a reasonable arena for embassy + defmt.
const HEAP_SIZE: usize = 64 * 1024;

/// Settle delay after audio bring-up before the first clip enqueues.
/// The audio task itself takes ~250 ms to bring up; 500 ms is comfortable
/// margin so the queue is being read by the time we start producing.
const POST_BRINGUP_DELAY_MS: u64 = 500;

/// Gap between clips in the playlist. Long enough that adjacent clips
/// are unmistakably distinct to the ear (no run-together).
const INTER_CLIP_GAP_MS: u64 = 800;

/// Pause at the end of a full playlist run before looping.
const PLAYLIST_GAP_MS: u64 = 5_000;

/// Audio task wrapper, identical to the one in `main.rs`. Lives here
/// in the bench so we don't pull in main.rs's other tasks.
#[embassy_executor::task]
async fn audio_task(peripherals: audio::AudioPeripherals) -> ! {
    audio::run_audio_task(peripherals).await
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);
    esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-audio-bench v{} — CoreS3 boot",
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

    let audio_periph = audio::AudioPeripherals {
        i2s: peripherals.I2S0,
        dma: peripherals.DMA_CH0,
        mclk: peripherals.GPIO0,
        bclk: peripherals.GPIO34,
        ws: peripherals.GPIO33,
        din: peripherals.GPIO14,
        dout: peripherals.GPIO13,
        amp_bus: I2cDevice::new(board_io.i2c_bus),
        adc_bus: I2cDevice::new(board_io.i2c_bus),
    };
    if let Err(e) = spawner.spawn(audio_task(audio_periph)) {
        defmt::panic!(
            "audio-bench: spawn audio_task failed: {}",
            defmt::Debug2Format(&e)
        );
    }

    Timer::after(Duration::from_millis(POST_BRINGUP_DELAY_MS)).await;
    defmt::info!("audio-bench: audio task spawned, starting playlist");

    loop {
        play_clip("BOOT_GREETING (default)", audio::BOOT_GREETING).await;
        play_clip("BOOT_GREETING_MORNING", audio::BOOT_GREETING_MORNING).await;
        play_clip("BOOT_GREETING_EVENING", audio::BOOT_GREETING_EVENING).await;
        play_clip("BOOT_GREETING_NIGHT", audio::BOOT_GREETING_NIGHT).await;
        play_clip("WAKE_CHIRP", audio::WAKE_CHIRP).await;

        play_helper("pickup chirp (2 clips)", audio::try_enqueue_pickup_chirp()).await;
        play_helper(
            "low-battery alert (3 clips)",
            audio::try_enqueue_low_battery_alert(),
        )
        .await;

        defmt::info!(
            "audio-bench: playlist done — looping in {=u64} ms",
            PLAYLIST_GAP_MS,
        );
        Timer::after(Duration::from_millis(PLAYLIST_GAP_MS)).await;
    }
}

/// Enqueue a single clip with a label log line and post-clip gap.
async fn play_clip(label: &str, clip: audio::AudioClip) {
    defmt::info!("audio-bench: playing {=str}", label);
    if let Err(e) = audio::try_enqueue_clip(clip) {
        defmt::warn!(
            "audio-bench: {=str} dropped, queue full ({:?})",
            label,
            defmt::Debug2Format(&e),
        );
    }
    Timer::after(Duration::from_millis(INTER_CLIP_GAP_MS)).await;
}

/// Run a multi-clip helper (e.g. `try_enqueue_pickup_chirp`) and apply
/// the same labelling + gap as `play_clip`. The helper has already
/// returned by the time we wait, so the gap measures from the
/// enqueue, not from playback completion — fine for a tuning bench.
async fn play_helper(
    label: &str,
    result: Result<(), embassy_sync::channel::TrySendError<audio::AudioClip>>,
) {
    defmt::info!("audio-bench: playing {=str}", label);
    if let Err(e) = result {
        defmt::warn!(
            "audio-bench: {=str} (partially) dropped ({:?})",
            label,
            defmt::Debug2Format(&e),
        );
    }
    Timer::after(Duration::from_millis(INTER_CLIP_GAP_MS)).await;
}
