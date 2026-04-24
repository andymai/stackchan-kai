//! ES7210 bench: brings up the shared I²C bus, runs the driver's full
//! init sequence (reset → clock tree for 12.288 MHz → 16 kHz → mic1+2
//! power-on at default gain → latch reset), and holds the ADC in that
//! configured state while logging a heartbeat.
//!
//! Does **not** capture audio — the I2S peripheral wiring lands in a
//! follow-up PR. This bench verifies the control-path: that the chip
//! answers at `0x40`, returns the expected `(0x72, 0x10)` chip ID, and
//! accepts the esp-adf-derived register sequence for our fixed audio
//! shape.
//!
//! Expected defmt log on a healthy chip:
//!
//! ```text
//! es7210-bench: detected chip ID (0x72, 0x10)
//! es7210-bench: init OK (12.288 MHz MCLK, 16 kHz mono, mic1+2 on)
//! es7210-bench: heartbeat (tick 0)
//! ...
//! ```

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use embassy_time::{Delay, Duration, Ticker};
use es7210::Es7210;
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
    defmt::error!("es7210-bench panic: {}", defmt::Display2Format(info));
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
        "stackchan-es7210-bench v{} — CoreS3 boot",
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

    let mut adc = Es7210::new(board_io.i2c);
    match adc.read_chip_id().await {
        Ok((lo, hi)) => defmt::info!(
            "es7210-bench: detected chip ID (0x{=u8:02X}, 0x{=u8:02X})",
            lo,
            hi
        ),
        Err(e) => defmt::panic!(
            "es7210-bench: chip-ID read failed: {}",
            defmt::Debug2Format(&e)
        ),
    }

    if let Err(e) = adc.init(&mut delay).await {
        defmt::panic!("es7210-bench: init failed: {}", defmt::Debug2Format(&e));
    }
    defmt::info!("es7210-bench: init OK (12.288 MHz MCLK, 16 kHz mono, mic1+2 on)");

    let mut ticker = Ticker::every(Duration::from_millis(HEARTBEAT_PERIOD_MS));
    let mut tick: u32 = 0;
    loop {
        defmt::info!("es7210-bench: heartbeat (tick {=u32})", tick);
        tick = tick.wrapping_add(1);
        ticker.next().await;
    }
}
