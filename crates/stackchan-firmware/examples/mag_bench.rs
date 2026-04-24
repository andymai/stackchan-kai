//! BMM150 magnetometer bench: brings up shared I²C, runs the driver's
//! full init sequence, then streams compensated `(x, y, z)` µT + total
//! magnitude at 5 Hz via defmt.
//!
//! Used to characterise the real field readings on-device: total
//! earth-field magnitude should land in the 25-65 µT range; deviation
//! suggests hard-iron offsets from the nearby `SCServo` motors or the
//! LED-ring's switching currents. Any future heading/compass modifier
//! needs this calibration baseline in hand first.
//!
//! Output per tick:
//!
//! ```text
//! mag-bench: x=12.34 y=-5.67 z=41.2 |B|=43.87 uT
//! ```

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use bmm150::Bmm150;
use embassy_time::{Delay, Duration, Ticker};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
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
    defmt::error!("mag-bench panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Heap size: no framebuffer, small drivers, embassy task arena.
const HEAP_SIZE: usize = 32 * 1024;

/// Poll cadence. 200 ms = 5 Hz; slow enough to read comfortably in
/// defmt output while Andy rotates the device for characterisation.
const POLL_PERIOD_MS: u64 = 200;

#[esp_rtos::main]
#[allow(
    clippy::used_underscore_binding,
    reason = "the esp_rtos::main macro requires the `spawner` arg; mag-bench doesn't spawn"
)]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-mag-bench v{} — CoreS3 boot, streaming BMM150 readings",
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

    let mut mag = match Bmm150::detect(board_io.i2c, &mut delay).await {
        Ok(m) => m,
        Err(e) => defmt::panic!("BMM150 detect failed: {}", defmt::Debug2Format(&e)),
    };
    defmt::info!("BMM150: detected at expected address");
    if let Err(e) = mag.init(&mut delay).await {
        defmt::panic!("BMM150 init failed: {}", defmt::Debug2Format(&e));
    }
    defmt::info!("mag-bench: streaming @ {=u64} ms tick", POLL_PERIOD_MS);

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    loop {
        match mag.read_measurement().await {
            Ok(m) => {
                let (x, y, z) = m.mag_ut;
                // Magnitude via the vector norm — useful for the
                // "is the field in the earth-field range?" sanity
                // check. `sqrt` via multiplication-free approximation:
                // use the squared magnitude in logs; defmt lets the
                // host compute sqrt if wanted. Keeps firmware libm-free.
                let mag_sq = x * x + y * y + z * z;
                defmt::info!(
                    "mag-bench: x={=f32} y={=f32} z={=f32} |B|^2={=f32} uT^2",
                    x,
                    y,
                    z,
                    mag_sq,
                );
            }
            Err(bmm150::Error::Overflow) => {
                defmt::warn!("mag-bench: ADC overflow — field may exceed dynamic range");
            }
            Err(e) => defmt::warn!("mag-bench: read failed: {}", defmt::Debug2Format(&e)),
        }
        ticker.next().await;
    }
}
