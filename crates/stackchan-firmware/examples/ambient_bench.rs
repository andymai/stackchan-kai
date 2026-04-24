//! Ambient-light bench: brings up shared I²C, runs LTR-553 init,
//! streams raw channel values + estimated lux at 5 Hz via defmt.
//!
//! Used to calibrate the 20 / 50 lux thresholds in
//! `stackchan_core::modifiers::AmbientSleepy`. Move the device under
//! various lighting conditions and grep the log for matching lux
//! values.
//!
//! Output per tick:
//!
//! ```text
//! ambient-bench: ch0=1234 ch1=123 lux=1234.56 status=0b0100
//! ```

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use embassy_time::{Delay, Duration, Ticker};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use ltr553::Ltr553;
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
    defmt::error!("ambient-bench panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Heap size: no framebuffer, small driver, embassy task arena.
const HEAP_SIZE: usize = 32 * 1024;

/// Poll cadence. 200 ms = 5 Hz; slow enough to read comfortably, fast
/// enough to see real-time lux changes as a lamp switches.
const POLL_PERIOD_MS: u64 = 200;

#[esp_rtos::main]
#[allow(
    clippy::used_underscore_binding,
    reason = "the esp_rtos::main macro requires the `spawner` arg; ambient-bench doesn't spawn"
)]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-ambient-bench v{} — CoreS3 boot, streaming LTR-553 readings",
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

    let mut als = Ltr553::new(board_io.i2c);
    match als.read_part_id().await {
        Ok(id) => defmt::info!("LTR-553 part_id=0x{=u8:02X}", id),
        Err(e) => defmt::warn!("LTR-553 part_id read failed: {}", defmt::Debug2Format(&e)),
    }
    if let Err(e) = als.init().await {
        defmt::panic!("LTR-553 init failed: {}", defmt::Debug2Format(&e));
    }
    defmt::info!("ambient-bench: streaming @ {=u64} ms tick", POLL_PERIOD_MS,);

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    loop {
        match als.read_ambient().await {
            Ok(r) => defmt::info!(
                "ambient-bench: ch0={=u16} ch1={=u16} lux={=f32}",
                r.ch0,
                r.ch1,
                r.lux,
            ),
            Err(e) => defmt::warn!("ambient-bench: read failed: {}", defmt::Debug2Format(&e)),
        }
        ticker.next().await;
    }
}
