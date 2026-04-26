//! Face cascade bench.
//!
//! Standalone firmware binary that brings up the camera + tracker
//! stack, runs the Haar face cascade on each motion candidate, and
//! logs the per-step timings via defmt. Sole purpose is to validate
//! the FPS budget on real CoreS3 hardware before face detection
//! lands in `main.rs`.
//!
//! Sample log line:
//!
//! ```text
//! face-bench: motion=Tracking fired=12 step_us=812 cascade_us=4_120 \
//!             face=YES rect=(60,80,72,72) centroid=( -150/1000,  20/1000)
//! ```
//!
//! `step_us` is the block-grid tracker step time; `cascade_us` is the
//! Haar-cascade scoring time over the largest motion candidate's ROI.
//! The bench never commands servos — it just measures and logs.

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]
// Bench-specific: the panic handler doesn't need a doc comment, the
// const-after-statements pattern matches `tracker_bench`, and the
// `map_or` replacements would make the bench harder to read for the
// "did the cascade fire?" decision flow it documents.
#![allow(
    clippy::missing_docs_in_private_items,
    clippy::items_after_statements,
    clippy::option_if_let_else,
    clippy::similar_names
)]

extern crate alloc;

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Instant, Timer};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use stackchan_firmware::{board, camera};
use tracker::{FRONTAL_FACE, Motion, Tracker, TrackerConfig, cascade::CascadeScratch};

use esp_println as _;

defmt::timestamp!("{=u64} ms", embassy_time::Instant::now().as_millis());

esp_bootloader_esp_idf::esp_app_desc!();

/// LTO anchor preventing `ESP_APP_DESC` from being stripped by
/// `lto = "fat"`. See `main.rs` for the full rationale.
#[used]
static _APP_DESC_ANCHOR: &esp_bootloader_esp_idf::EspAppDesc = &ESP_APP_DESC;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("face-bench panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Heap size. Camera task allocates DMA + scratch buffers in PSRAM;
/// internal SRAM only needs to cover the embassy task arena and defmt.
const HEAP_SIZE: usize = 32 * 1024;

/// Camera task entry. Same shape as `main.rs`'s `camera_task` — the
/// bench reuses the production task so its cascade integration is
/// also exercised end-to-end.
#[embassy_executor::task]
async fn camera_task(peripherals: camera::CameraPeripherals) -> ! {
    camera::run_camera_task(peripherals).await
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);
    esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "face-bench v{} — CoreS3 boot, will stream GC0308 frames through tracker + cascade",
        env!("CARGO_PKG_VERSION"),
    );
    defmt::info!(
        "face-bench: cascade has {=usize} stages, base window {=u8}x{=u8}",
        FRONTAL_FACE.stages.len(),
        FRONTAL_FACE.window_w,
        FRONTAL_FACE.window_h,
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

    let camera_periph = camera::CameraPeripherals {
        lcd_cam: peripherals.LCD_CAM,
        dma: peripherals.DMA_CH1,
        i2c: I2cDevice::new(board_io.i2c_bus),
        pclk: peripherals.GPIO45,
        href: peripherals.GPIO38,
        vsync: peripherals.GPIO46,
        d0: peripherals.GPIO39,
        d1: peripherals.GPIO40,
        d2: peripherals.GPIO41,
        d3: peripherals.GPIO42,
        d4: peripherals.GPIO15,
        d5: peripherals.GPIO16,
        d6: peripherals.GPIO48,
        d7: peripherals.GPIO47,
    };
    if let Err(e) = spawner.spawn(camera_task(camera_periph)) {
        defmt::panic!("spawn camera_task failed: {}", defmt::Debug2Format(&e));
    }

    // Force the camera task into preview-streaming mode so the LCD
    // mirrors what the cascade is seeing. The bench never toggles
    // back to avatar — we want eyes on the camera's view.
    camera::CAMERA_MODE_SIGNAL.signal(true);
    defmt::info!("face-bench: camera preview ON; consuming frames");

    // Independent tracker + scratch. The production camera task runs
    // its own cascade; this bench drives a second independent pass on
    // the published frame so step / cascade timings are isolated.
    let mut tracker = Tracker::new(TrackerConfig::DEFAULT);
    let cascade_scratch: &'static mut CascadeScratch =
        alloc::boxed::Box::leak(alloc::boxed::Box::new(CascadeScratch::new()));

    /// ROI side length. Matches the production camera task default.
    const ROI_DIM: u16 = 96;

    /// Wallclock interval for the tracker's idle / return-to-centre
    /// arithmetic. We measure the real step duration but don't need
    /// the camera frame interval — feed a nominal 33 ms (~30 FPS).
    const NOMINAL_FRAME_DT_MS: u32 = 33;

    loop {
        let frame = camera::CAMERA_FRAME_SIGNAL.wait().await;

        // Time the block-grid tracker step.
        let t0 = Instant::now();
        let outcome = tracker.step(frame, NOMINAL_FRAME_DT_MS);
        let step_us = t0.elapsed().as_micros();

        // Run cascade only when the tracker surfaced at least one
        // motion candidate — matches the production code path.
        let (cascade_us, face_rect, face_centroid) = match outcome.candidates.first() {
            Some(c) => {
                let t1 = Instant::now();
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "FRAME_WIDTH=320, FRAME_HEIGHT=240; both fit u16 trivially"
                )]
                let det = FRONTAL_FACE.scan_around_centroid(
                    frame,
                    camera::FRAME_WIDTH as u16,
                    camera::FRAME_HEIGHT as u16,
                    c.centroid,
                    ROI_DIM,
                    cascade_scratch,
                );
                let cascade_us = t1.elapsed().as_micros();
                match det {
                    Some(d) => (cascade_us, Some(d.frame_rect), Some(d.centroid)),
                    None => (cascade_us, None, None),
                }
            }
            None => (0, None, None),
        };

        log_step(&outcome, step_us, cascade_us, face_rect, face_centroid);

        Timer::after(Duration::from_millis(1)).await;
    }
}

/// Format one bench step over defmt. Centroid is emitted as a scaled
/// integer because the firmware defmt build does not enable the
/// `float` feature.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "centroid is bounded to [-1, 1] so the * 1000 scaled value fits in i32; \
              microsecond durations from `Instant::elapsed` are u64 but bench frames \
              never exceed a few hundred ms — formatting truncation is harmless"
)]
fn log_step(
    outcome: &tracker::Outcome,
    step_us: u64,
    cascade_us: u64,
    face_rect: Option<(u16, u16, u16, u16)>,
    face_centroid: Option<(f32, f32)>,
) {
    let motion_label = match outcome.motion {
        Motion::Warmup => "Warmup",
        Motion::Tracking => "Tracking",
        Motion::Holding => "Holding",
        Motion::Returning => "Returning",
        Motion::GlobalEvent => "GlobalEvent",
    };
    let (rx, ry, rw, rh) = face_rect.unwrap_or((0, 0, 0, 0));
    let (cx_milli, cy_milli) = match face_centroid {
        Some((nx, ny)) => ((nx * 1000.0) as i32, (ny * 1000.0) as i32),
        None => (0, 0),
    };
    let face_str = if face_centroid.is_some() { "YES" } else { "no" };
    defmt::info!(
        "face-bench: motion={=str} fired={=u16} step_us={=u64} cascade_us={=u64} \
         face={=str} rect=({=u16},{=u16},{=u16},{=u16}) centroid=({=i32}/1000, {=i32}/1000)",
        motion_label,
        outcome.fired_cells,
        step_us,
        cascade_us,
        face_str,
        rx,
        ry,
        rw,
        rh,
        cx_milli,
        cy_milli,
    );
}
