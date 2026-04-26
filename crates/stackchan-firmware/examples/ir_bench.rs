//! IR bench: log every decoded NEC frame so the operator can populate
//! `EmotionFromRemote`'s mapping table for their specific remote.
//!
//! Skips every other peripheral (LCD, servos, IMU, ambient) — just
//! initialises the RMT RX path and prints each `(address, command)`
//! pair as the operator mashes buttons on a remote.
//!
//! Output per button:
//!
//! ```text
//! ir-bench: addr=0x00EF cmd=0x5A
//! ```

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use stackchan_firmware::ir;

use esp_println as _;

defmt::timestamp!("{=u64} ms", embassy_time::Instant::now().as_millis());

esp_bootloader_esp_idf::esp_app_desc!();

/// LTO anchor preventing `ESP_APP_DESC` from being stripped.
#[used]
static _APP_DESC_ANCHOR: &esp_bootloader_esp_idf::EspAppDesc = &ESP_APP_DESC;

/// Panic handler for the bench binary.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("ir-bench panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Heap size: no framebuffer, tiny decoder, embassy task arena.
const HEAP_SIZE: usize = 32 * 1024;

#[esp_rtos::main]
#[allow(
    clippy::used_underscore_binding,
    reason = "the esp_rtos::main macro requires the `spawner` arg; ir-bench doesn't spawn"
)]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-ir-bench v{} — CoreS3 boot, will dump decoded NEC frames",
        env!("CARGO_PKG_VERSION")
    );
    defmt::info!(
        "ir-bench: point a remote at the device and press buttons. Lines with `addr=... cmd=...` go in EmotionFromRemote's mapping table.",
    );

    // ir::run_ir_loop configures the RMT channel + decodes into
    // ir::REMOTE_SIGNAL. We don't drain the signal here — the decoder
    // also logs every decoded command at info level, which is all the
    // operator needs to read the codes off.
    ir::run_ir_loop(peripherals.RMT, peripherals.GPIO21).await
}
