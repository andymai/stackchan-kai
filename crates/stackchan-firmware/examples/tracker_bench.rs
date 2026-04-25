//! Look-toward-motion tracker bench.
//!
//! Standalone firmware binary that brings up only the pieces of the
//! CoreS3 needed to feed the new `tracker` crate — AXP2101 → AW9523 →
//! shared I²C0 → GC0308 + LCD\_CAM camera task — and then runs the
//! [`tracker::Tracker`] over every published camera frame, logging each
//! decision via defmt. **No servos are commanded** in this bench: the
//! bench's job is to validate the algorithm and tuning on real hardware
//! before a follow-up PR wires the tracker into `main.rs`.
//!
//! The standard `board::bringup` is reused for the power / I²C dance,
//! which means the boot-nod gesture still runs once at startup
//! (servos exercise both axes briefly). After that the head holds at
//! its rest pose for the duration of the bench.
//!
//! Sample log line:
//!
//! ```text
//! tracker-bench: motion=Tracking fired=12 nx=0.42 ny=-0.31 \
//!                target_pan=2.4 target_tilt=1.6
//! ```

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Timer};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use stackchan_firmware::{board, camera};
use tracker::{Motion, Tracker, TrackerConfig};

use esp_println as _;

defmt::timestamp!("{=u64} ms", embassy_time::Instant::now().as_millis());

esp_bootloader_esp_idf::esp_app_desc!();

/// LTO anchor preventing `ESP_APP_DESC` from being stripped by
/// `lto = "fat"`. See `main.rs` for the full rationale.
#[used]
static _APP_DESC_ANCHOR: &esp_bootloader_esp_idf::EspAppDesc = &ESP_APP_DESC;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("tracker-bench panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Heap size. The camera task allocates two 150 KiB DMA buffers + two
/// 150 KiB scratch slots in PSRAM, and a small embassy task arena
/// + defmt buffers in internal SRAM. 32 KiB internal is plenty for the
/// latter; PSRAM is registered separately by `psram_allocator!`.
const HEAP_SIZE: usize = 32 * 1024;

/// Camera task entry. Same shape as `main.rs`'s `camera_task` so the
/// bench reuses the production task.
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
        "tracker-bench v{} — CoreS3 boot, will stream GC0308 frames into the tracker",
        env!("CARGO_PKG_VERSION"),
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

    // Force the camera task into streaming mode. The signal is sticky;
    // we publish it once and the camera task picks it up on its next
    // wait edge. The bench never publishes `false`, so streaming runs
    // forever.
    camera::CAMERA_MODE_SIGNAL.signal(true);
    defmt::info!("tracker-bench: camera mode forced ON; consuming frames");

    // One Tracker per bench run. Default config matches the QVGA
    // GC0308 + Stack-chan SCServo head.
    let mut tracker = Tracker::new(TrackerConfig::DEFAULT);

    // Frame publish rate is ~30 FPS; nominal dt = 33 ms. We could
    // measure the real interval, but this is fine for the idle-timeout
    // arithmetic at this scope.
    const NOMINAL_FRAME_DT_MS: u32 = 33;
    let mut last_motion: Option<Motion> = None;
    loop {
        let frame = camera::CAMERA_FRAME_SIGNAL.wait().await;
        let outcome = tracker.step(frame, NOMINAL_FRAME_DT_MS);

        // De-noise: only log on motion-class transitions and tracking
        // updates. The Holding / Returning steady states would otherwise
        // drown the RTT.
        let log_now = match outcome.motion {
            Motion::Tracking | Motion::GlobalEvent => true,
            other => last_motion != Some(other),
        };
        if log_now {
            log_outcome(&outcome);
            last_motion = Some(outcome.motion);
        }

        // Tiny yield so other tasks (audio, button, etc) run.
        // `Signal::wait` already yielded; this is a paranoia tick to
        // bound the wakeup rate if the camera ever publishes faster
        // than 30 FPS.
        Timer::after(Duration::from_millis(1)).await;
    }
}

/// Format the tracker outcome over defmt. Centroid / pose are emitted
/// as scaled integers because defmt doesn't support `f32` formatters
/// without the `float` feature, which the firmware's defmt build does
/// not enable.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "centroid is bounded to [-1, 1] and pose to ±MAX_PAN_DEG / [0, MAX_TILT_DEG]; \
              i32 trivially holds the * 1000 / * 100 scaled values"
)]
fn log_outcome(outcome: &tracker::Outcome) {
    let motion_label = match outcome.motion {
        Motion::Warmup => "Warmup",
        Motion::Tracking => "Tracking",
        Motion::Holding => "Holding",
        Motion::Returning => "Returning",
        Motion::GlobalEvent => "GlobalEvent",
    };
    let (cx_milli, cy_milli) = match outcome.centroid {
        Some((nx, ny)) => ((nx * 1000.0) as i32, (ny * 1000.0) as i32),
        None => (0, 0),
    };
    let pan_centi = (outcome.target.pan_deg * 100.0) as i32;
    let tilt_centi = (outcome.target.tilt_deg * 100.0) as i32;
    defmt::info!(
        "tracker-bench: motion={=str} fired={=u16} centroid=({=i32}/1000, {=i32}/1000) \
         target_pose=({=i32}/100°, {=i32}/100°)",
        motion_label,
        outcome.fired_cells,
        cx_milli,
        cy_milli,
        pan_centi,
        tilt_centi,
    );
}
