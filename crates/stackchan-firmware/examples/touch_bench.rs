//! Touch-controller bench.
//!
//! Standalone firmware binary that brings up only the pieces of the
//! CoreS3 needed to reach the FT6336U — AXP2101 → AW9523 → shared
//! I²C0 — and then enters a tight polling loop logging each touch
//! report via defmt. Intentionally skips the LCD init (no `mipidsi`)
//! so the bench binary is small and fast to flash.
//!
//! Output per touch tick:
//!
//! ```text
//! touch-bench: fingers=1 x=134 y=98
//! ```
//!
//! `fingers=0` ticks are suppressed — the log would drown otherwise —
//! but the *first* no-touch tick after a release is logged so you can
//! see the release edge. A bus error is logged once per occurrence.

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use embassy_time::{Delay, Duration, Ticker};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use ft6336u::{Ft6336u, VENDOR_ID_FOCALTECH};
use stackchan_firmware::board;

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
    defmt::error!("touch-bench panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Smaller heap than main (no framebuffer). 32 KiB comfortably covers
/// the embassy task arena + defmt buffers.
const HEAP_SIZE: usize = 32 * 1024;

/// Poll cadence. 60 Hz matches the FT6336U's native sample rate —
/// reading faster returns duplicates of the same internal sample.
const POLL_PERIOD_MS: u64 = 16;

/// Main entry. Runs the shared CoreS3 bringup (servos + shared I²C),
/// constructs a `Ft6336u`, reads the vendor ID once, then polls
/// forever.
#[esp_rtos::main]
#[allow(
    clippy::used_underscore_binding,
    reason = "the esp_rtos::main macro requires the `spawner` arg; touch-bench doesn't spawn"
)]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-touch-bench v{} — CoreS3 boot, will poll FT6336U over I²C0",
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

    let mut touch = Ft6336u::new(board_io.i2c);

    match touch.read_vendor_id().await {
        Ok(id) if id == VENDOR_ID_FOCALTECH => {
            defmt::info!("FT6336U: vendor ID 0x{=u8:02X} (FocalTech, expected)", id);
        }
        Ok(id) => defmt::warn!(
            "FT6336U: unexpected vendor ID 0x{=u8:02X} (expected 0x{=u8:02X})",
            id,
            VENDOR_ID_FOCALTECH,
        ),
        Err(e) => defmt::error!(
            "FT6336U: vendor-ID read failed — touch-bench will still poll, but chip may be absent: {}",
            defmt::Debug2Format(&e),
        ),
    }

    defmt::info!(
        "touch-bench: polling @ {=u64} ms tick — touch the screen to stream reports",
        POLL_PERIOD_MS
    );

    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));
    let mut was_touched = false;
    loop {
        match touch.read_touch().await {
            Ok(report) => {
                if report.is_touched() {
                    if let Some((x, y)) = report.first {
                        defmt::info!(
                            "touch-bench: fingers={=u8} x={=u16} y={=u16}",
                            report.fingers,
                            x,
                            y,
                        );
                    } else {
                        defmt::warn!(
                            "touch-bench: fingers={=u8} but no first-point data",
                            report.fingers,
                        );
                    }
                    was_touched = true;
                } else if was_touched {
                    // First idle tick after a release — useful to
                    // confirm we're seeing release edges too.
                    defmt::info!("touch-bench: release");
                    was_touched = false;
                }
            }
            Err(e) => defmt::warn!(
                "touch-bench: read_touch failed: {}",
                defmt::Debug2Format(&e)
            ),
        }
        ticker.next().await;
    }
}
