//! IMU bench: brings up the shared I²C, runs BMI270 init, streams raw
//! accel + gyro at 20 Hz via defmt.
//!
//! Intended for threshold calibration. Lift, drop, or tilt the device
//! and grep the log to see what magnitudes real motions produce before
//! locking in `stackchan_core::modifiers::PICKUP_DEVIATION_G`.
//!
//! Output per tick (example):
//!
//! ```text
//! imu-bench: ax=+0.02 ay=-0.01 az=+1.00 |a|^2=1.00 gx=+0.1 gy=-0.2 gz=+0.0
//! ```

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use bmi270::Bmi270;
use embassy_time::{Delay, Duration, Ticker};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use stackchan_firmware::board;

// esp-println registers the global defmt logger via USB-Serial-JTAG.
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
    defmt::error!("imu-bench panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Heap size: no framebuffer, small driver, embassy task arena.
const HEAP_SIZE: usize = 32 * 1024;

/// Poll cadence. 20 Hz is slow enough to read comfortably in a defmt
/// log stream, fast enough to see lift/drop transients.
const POLL_PERIOD_MS: u64 = 50;

/// Bench entry. Runs the shared bringup, init the IMU, stream samples.
#[esp_rtos::main]
#[allow(
    clippy::used_underscore_binding,
    reason = "the esp_rtos::main macro requires the `spawner` arg; imu-bench doesn't spawn"
)]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-imu-bench v{} — CoreS3 boot, will stream raw BMI270 readings",
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

    let mut imu = match Bmi270::detect(board_io.i2c, &mut delay).await {
        Ok(d) => {
            defmt::info!("BMI270 detected at 0x{=u8:02X}", d.address());
            d
        }
        Err(e) => {
            defmt::panic!(
                "BMI270 detection failed — check I²C wiring + power: {}",
                defmt::Debug2Format(&e),
            );
        }
    };
    if let Err(e) = imu.init(&mut delay).await {
        defmt::panic!("BMI270 init failed: {}", defmt::Debug2Format(&e));
    }
    defmt::info!(
        "imu-bench: streaming @ {=u64} ms tick — lift / drop / tilt the device to see magnitudes",
        POLL_PERIOD_MS,
    );

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    loop {
        match imu.read_measurement().await {
            Ok(m) => {
                let (ax, ay, az) = m.accel_g;
                let (gx, gy, gz) = m.gyro_dps;
                // |a|² is what PickupReaction actually compares
                // against; print it so the operator can read pickup
                // thresholds directly off the log.
                #[allow(
                    clippy::suboptimal_flops,
                    reason = "f32::mul_add needs libm on no_std; keep consistent with stackchan-core"
                )]
                let accel_mag_squared = ax * ax + ay * ay + az * az;
                defmt::info!(
                    "imu-bench: ax={=f32} ay={=f32} az={=f32} |a|^2={=f32} gx={=f32} gy={=f32} gz={=f32}",
                    ax,
                    ay,
                    az,
                    accel_mag_squared,
                    gx,
                    gy,
                    gz,
                );
            }
            Err(e) => defmt::warn!("imu-bench: read failed: {}", defmt::Debug2Format(&e)),
        }
        ticker.next().await;
    }
}
