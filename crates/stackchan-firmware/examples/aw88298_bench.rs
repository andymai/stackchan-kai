//! AW88298 bench: brings up the shared I²C bus (which releases the
//! amplifier's external `RST` via AW9523), runs the driver's full init
//! sequence (reset → enable → configure I2S 16 kHz mono → mute →
//! disable boost), and holds the chip in that configured state while
//! logging a heartbeat.
//!
//! Does **not** stream audio — the I2S peripheral wiring lands in a
//! follow-up PR. This bench verifies the control-path: that the amp
//! answers at `0x36`, returns the expected `0x1852` chip ID, and
//! accepts the canonical esp-adf register sequence without error.
//!
//! Expected defmt log on a healthy chip:
//!
//! ```text
//! aw88298-bench: detected chip ID 0x1852
//! aw88298-bench: init OK (muted, I2S 16 kHz, boost off)
//! aw88298-bench: heartbeat (tick 0)
//! aw88298-bench: heartbeat (tick 1)
//! ...
//! ```

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use aw88298::Aw88298;
use embassy_time::{Delay, Duration, Ticker};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use stackchan_firmware::board;

use esp_println as _;

defmt::timestamp!("{=u64} ms", embassy_time::Instant::now().as_millis());

esp_bootloader_esp_idf::esp_app_desc!();

/// LTO anchor preventing `ESP_APP_DESC` from being stripped.
#[used]
static _APP_DESC_ANCHOR: &esp_bootloader_esp_idf::EspAppDesc = &ESP_APP_DESC;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("aw88298-bench panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Heap size: no framebuffer, minimal drivers + embassy task arena.
const HEAP_SIZE: usize = 32 * 1024;

/// Heartbeat cadence, in milliseconds.
const HEARTBEAT_PERIOD_MS: u64 = 1_000;

#[esp_rtos::main]
#[allow(
    clippy::used_underscore_binding,
    reason = "esp_rtos::main requires the spawner arg; this bench doesn't spawn"
)]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-aw88298-bench v{} — CoreS3 boot",
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

    let mut amp = Aw88298::new(board_io.i2c);
    match amp.read_chip_id().await {
        Ok(id) => defmt::info!("aw88298-bench: detected chip ID 0x{=u16:04X}", id),
        Err(e) => defmt::panic!(
            "aw88298-bench: chip-ID read failed: {}",
            defmt::Debug2Format(&e)
        ),
    }

    if let Err(e) = amp.init(&mut delay).await {
        defmt::panic!("aw88298-bench: init failed: {}", defmt::Debug2Format(&e));
    }
    defmt::info!("aw88298-bench: init OK (muted, I2S 16 kHz, boost off)");

    let mut ticker = Ticker::every(Duration::from_millis(HEARTBEAT_PERIOD_MS));
    let mut tick: u32 = 0;
    loop {
        defmt::info!("aw88298-bench: heartbeat (tick {=u32})", tick);
        tick = tick.wrapping_add(1);
        ticker.next().await;
    }
}
