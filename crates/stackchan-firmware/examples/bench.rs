//! Servo calibration bench.
//!
//! Standalone firmware binary that sweeps each axis through a fixed
//! pattern, reads the live servo position at each stop, and logs a
//! one-line-per-step CSV-ish row via defmt so the operator can grep
//! out a trim table. Designed for flashing via `just bench` — when
//! the sweep finishes the binary centres the servos, logs
//! `bench complete`, and halts forever.
//!
//! Sweep (conservative, matches the defaults in the parent PR):
//!
//! - Pan: -30 → -20 → -10 → 0 → +10 → +20 → +30 (7 steps)
//! - Tilt: -20 → -10 → 0 → +10 → +20 (5 steps)
//! - 300 ms dwell per step — enough for the servo's internal
//!   interpolation to complete before the readback.
//!
//! Output per step:
//!
//! ```text
//! bench pan:  cmd=-30.00 raw_pos=547 actual_deg=+14.37 delta=+44.37
//! ```
//!
//! `delta = actual - cmd`. The expected value is 0 after proper trim
//! calibration; systematic offsets suggest a trim value; changes in
//! sign across the range suggest flipping `*_DIRECTION`.

#![no_std]
#![no_main]
// Example binary inherits the same ESP-IDF app-descriptor requirement
// as the main firmware; anchoring a `&'static` reference (rather than
// a raw-pointer newtype) keeps the symbol live without needing `unsafe`.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
// Single-core executor; `Send`-bounded futures aren't meaningful here.
#![allow(clippy::future_not_send)]

extern crate alloc;

use embassy_time::{Delay, Duration, Timer};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use stackchan_core::{Clock, HeadDriver, Pose};
use stackchan_firmware::{board, clock::HalClock};

// esp-println registers the global defmt logger via USB-Serial-JTAG.
use esp_println as _;

defmt::timestamp!("{=u64} ms", embassy_time::Instant::now().as_millis());

esp_bootloader_esp_idf::esp_app_desc!();

/// LTO anchor preventing `ESP_APP_DESC` from being stripped. See
/// `main.rs` for the full rationale — short version: `lto = "fat"`
/// strips the bootloader-readable descriptor unless a `#[used]` static
/// holds a reference to it. A `&'static` reference is auto-Sync (the
/// pointee is plain POD) and avoids the raw-pointer newtype the
/// workspace `unsafe_code = deny` rule would otherwise force.
#[used]
static _APP_DESC_ANCHOR: &esp_bootloader_esp_idf::EspAppDesc = &ESP_APP_DESC;

/// Panic handler for the bench binary. Halts the core; the trace has
/// already been emitted via defmt before we arrive here.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("bench panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Smaller heap than main (no framebuffer), but the embassy task arena
/// and defmt buffers still need SRAM. 32 KiB is comfortable.
const HEAP_SIZE: usize = 32 * 1024;

/// Dwell per sweep step before reading the actual position.
const DWELL_MS: u64 = 300;

/// Timeout for each `read_position` call.
const READ_TIMEOUT_MS: u64 = 10;

/// Main entry point. Runs a single imperative sweep in the embassy
/// executor and halts. The `_spawner` argument is required by the
/// `esp_rtos::main` macro signature but we never spawn background
/// tasks — bench is intentionally single-threaded.
#[esp_rtos::main]
#[allow(
    clippy::used_underscore_binding,
    reason = "the esp_rtos::main macro requires the `spawner` arg; bench doesn't spawn"
)]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-bench v{} — CoreS3 boot, will sweep + log deltas",
        env!("CARGO_PKG_VERSION")
    );

    let mut delay = Delay;
    let mut driver = board::bringup(
        peripherals.I2C0,
        peripherals.UART1,
        peripherals.GPIO12,
        peripherals.GPIO11,
        peripherals.GPIO6,
        peripherals.GPIO7,
        &mut delay,
    )
    .await
    .head;

    // Pan sweep: -30 .. +30 in 10° steps.
    for cmd in [-30.0_f32, -20.0, -10.0, 0.0, 10.0, 20.0, 30.0] {
        run_step("pan", &mut driver, Pose::new(cmd, 0.0), cmd, true).await;
    }

    // Tilt sweep: -20 .. +20 in 10° steps. Return pan to 0 first so
    // the tilt readings aren't entangled with an off-axis pan target.
    for cmd in [-20.0_f32, -10.0, 0.0, 10.0, 20.0] {
        run_step("tilt", &mut driver, Pose::new(0.0, cmd), cmd, false).await;
    }

    // Centre, log completion, halt.
    if let Err(e) = driver.set_pose(Pose::NEUTRAL, HalClock.now()).await {
        defmt::warn!("bench: centre command failed: {}", defmt::Debug2Format(&e));
    }
    Timer::after(Duration::from_millis(DWELL_MS)).await;
    defmt::info!("bench complete — re-flash main firmware with `just flash` to resume normal boot");
    loop {
        Timer::after(Duration::from_secs(5)).await;
    }
}

/// Command a single pose, wait for the servo to settle, read back its
/// live position, and log one line with `cmd_deg`, `raw_pos`,
/// `actual_deg`, `delta_deg`. `axis` picks which servo to read; `is_pan`
/// true = yaw servo, false = pitch servo.
async fn run_step(
    label: &str,
    driver: &mut board::HeadDriverImpl,
    pose: Pose,
    cmd_deg: f32,
    is_pan: bool,
) {
    if let Err(e) = driver.set_pose(pose, HalClock.now()).await {
        defmt::warn!(
            "bench {}: set_pose failed: {}",
            label,
            defmt::Debug2Format(&e)
        );
        return;
    }
    Timer::after(Duration::from_millis(DWELL_MS)).await;

    let id = if is_pan {
        stackchan_firmware::head::YAW_SERVO_ID
    } else {
        stackchan_firmware::head::PITCH_SERVO_ID
    };
    let bus = driver.bus_mut();
    let raw = match embassy_time::with_timeout(
        Duration::from_millis(READ_TIMEOUT_MS),
        bus.read_position(id),
    )
    .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            defmt::warn!(
                "bench {}: read_position err at cmd={=f32}: {}",
                label,
                cmd_deg,
                defmt::Debug2Format(&e)
            );
            return;
        }
        Err(_) => {
            defmt::warn!(
                "bench {}: read_position timed out at cmd={=f32}",
                label,
                cmd_deg
            );
            return;
        }
    };

    // Inverse mapping: actual_deg = (raw - 512) / POSITION_PER_DEGREE,
    // assuming direction = +1 and trim = 0 (bench's job is to discover
    // the actual trim, so don't apply one here).
    let actual_deg =
        (f32::from(raw) - f32::from(scservo::POSITION_CENTER)) / scservo::POSITION_PER_DEGREE;
    let delta = actual_deg - cmd_deg;
    defmt::info!(
        "bench {}: cmd={=f32} raw_pos={=u16} actual_deg={=f32} delta={=f32}",
        label,
        cmd_deg,
        raw,
        actual_deg,
        delta,
    );
}
