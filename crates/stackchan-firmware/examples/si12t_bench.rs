//! Si12T body-touch bench.
//!
//! Brings up the shared I²C bus, initialises the Si12T 3-zone
//! capacitive touch controller on the back of the head, and polls
//! `OUTPUT1` at 50 ms (matches the M5Stack reference cadence). Logs a
//! report whenever the touch state changes; suppresses idle ticks so
//! the log isn't drowned. Verify on hardware by tapping each pad and
//! watching for the corresponding zone to assert.
//!
//! Output:
//!
//! ```text
//! si12t-bench: left=0 centre=2 right=0  (Mid touch on centre)
//! si12t-bench: release
//! ```

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use embassy_time::{Delay, Duration, Ticker};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use si12t::Si12t;
use stackchan_firmware::board;

use esp_println as _;

defmt::timestamp!("{=u64} ms", embassy_time::Instant::now().as_millis());

esp_bootloader_esp_idf::esp_app_desc!();

#[used]
static _APP_DESC_ANCHOR: &esp_bootloader_esp_idf::EspAppDesc = &ESP_APP_DESC;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("si12t-bench panic: {}", defmt::Display2Format(info));
    loop {}
}

const HEAP_SIZE: usize = 32 * 1024;

/// Poll cadence — matches the upstream M5Stack reference task.
const POLL_PERIOD_MS: u64 = 50;

#[esp_rtos::main]
#[allow(
    clippy::used_underscore_binding,
    reason = "esp_rtos::main signature requires the spawner; this bench doesn't spawn"
)]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-si12t-bench v{} — boot, will init Si12T at 0x{=u8:02X}",
        env!("CARGO_PKG_VERSION"),
        si12t::ADDRESS,
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

    let mut chip = Si12t::new(board_io.i2c);
    match chip.init(&mut delay).await {
        Ok(()) => defmt::info!("Si12T: init OK"),
        Err(e) => defmt::error!(
            "Si12T: init failed — bench will still poll, but reads may all read 0: {}",
            defmt::Debug2Format(&e),
        ),
    }
    // Match the 200 ms post-setup wait the M5Stack reference uses
    // before the first read — chip needs to settle after init.
    embassy_time::Timer::after_millis(200).await;

    defmt::info!(
        "si12t-bench: polling @ {=u64} ms — touch a body pad to stream reports",
        POLL_PERIOD_MS,
    );

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    let mut last_byte = 0u8;
    let mut was_touched = false;
    loop {
        match chip.read_touch().await {
            Ok(touch) => {
                let raw = pack(&touch);
                let touched = touch.left() || touch.centre() || touch.right();
                if touched {
                    if raw != last_byte {
                        defmt::info!(
                            "si12t-bench: left={=u8} centre={=u8} right={=u8} (raw=0x{=u8:02X})",
                            level(touch.intensity.0),
                            level(touch.intensity.1),
                            level(touch.intensity.2),
                            raw,
                        );
                    }
                    was_touched = true;
                } else if was_touched {
                    defmt::info!("si12t-bench: release");
                    was_touched = false;
                }
                last_byte = raw;
            }
            Err(e) => defmt::warn!(
                "si12t-bench: read_touch failed: {}",
                defmt::Debug2Format(&e),
            ),
        }
        ticker.next().await;
    }
}

/// Numeric encoding for log lines: 0=None, 1=Low, 2=Mid, 3=High.
const fn level(i: si12t::Intensity) -> u8 {
    match i {
        si12t::Intensity::None => 0,
        si12t::Intensity::Low => 1,
        si12t::Intensity::Mid => 2,
        si12t::Intensity::High => 3,
    }
}

/// Re-pack a [`si12t::Touch`] into the original output byte for the
/// "raw=0xXX" log field — useful when sanity-checking the parser
/// against the reference firmware.
const fn pack(t: &si12t::Touch) -> u8 {
    level(t.intensity.0) | (level(t.intensity.1) << 2) | (level(t.intensity.2) << 4)
}
