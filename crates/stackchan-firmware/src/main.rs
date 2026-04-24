//! StackChan firmware for the M5Stack CoreS3.
//!
//! v0.2.0 minimal boot: esp-hal init → esp-rtos embassy → AXP2101 LDO
//! bring-up → defmt "hello" over RTT. The LCD SPI init, mipidsi driver,
//! and avatar render loop land in the next PR — this PR proves the
//! toolchain, power-on sequencing, and telemetry path end-to-end on
//! real hardware.

#![no_std]
#![no_main]
// Firmware main is the hardware boundary: init failures can't be bubbled
// to a caller, so `panic!` IS the error-handling layer. The workspace-wide
// `panic`/`expect`/`unwrap` lints are a library-code rule; they don't
// fit at the top of a `#[no_main]` binary.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
// esp-rtos runs a single-core executor on this chip; `Send`-bounded
// futures aren't meaningful here. The nursery lint fires on every task.
#![allow(clippy::future_not_send)]

extern crate alloc;

use axp2101::Axp2101;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{
    clock::CpuClock,
    i2c::master::{Config as I2cConfig, I2c},
    time::Rate,
    timer::timg::TimerGroup,
};

// The ESP-IDF second-stage bootloader reads an `app_desc` struct at a
// fixed offset; the macro emits one in a dedicated linker section.
esp_bootloader_esp_idf::esp_app_desc!();

/// Panic handler. Halts the core; esp-rtos emits the trace over RTT
/// before we arrive here (via `--catch-hardfault` on the probe-rs side).
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Internal SRAM heap for embassy task arena + defmt buffers. Kept small:
/// PSRAM comes online in the next PR when the 320×240 RGB565 framebuffer
/// (~150 KiB) needs a home. 72 KiB matches the esp-generate default and
/// leaves ample margin for `esp-rtos` internal state.
const HEAP_SIZE: usize = 72 * 1024;

/// Retry delay between failed AXP2101 init attempts. Covers transient
/// I²C glitches during cold boot — no forward progress is possible
/// without the LDOs, so halting here is the wrong answer.
const PMIC_RETRY_MS: u64 = 500;

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // Spawner is unused in v0.2.0 (no background tasks yet); the LCD
    // render task in the next PR will consume it. Drop explicitly so
    // the unused-var warning doesn't mask real issues.
    let _ = spawner;

    rtt_target::rtt_init_defmt!();

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-firmware v{} — CoreS3 boot",
        env!("CARGO_PKG_VERSION")
    );

    // CoreS3 internal I²C bus: GPIO11 = SDA, GPIO12 = SCL.
    // AXP2101 (0x34), AW9523 IO expander, and the touch controller all
    // sit on this bus. 100 kHz is the conservative standard-mode rate
    // that works from cold boot before PLLs are fully settled.
    let i2c_cfg = I2cConfig::default().with_frequency(Rate::from_khz(100));
    let i2c = match I2c::new(peripherals.I2C0, i2c_cfg) {
        Ok(bus) => bus
            .with_sda(peripherals.GPIO11)
            .with_scl(peripherals.GPIO12)
            .into_async(),
        Err(e) => defmt::panic!("I2C0 config rejected: {}", defmt::Debug2Format(&e)),
    };

    let mut pmic = Axp2101::new(i2c);

    loop {
        match pmic.enable_lcd_rails().await {
            Ok(()) => {
                defmt::info!("AXP2101: ALDO1/BLDO1/BLDO2 @ 3.3V — LCD rails up");
                break;
            }
            Err(e) => {
                defmt::error!(
                    "AXP2101 init failed (retrying in {=u64} ms): {}",
                    PMIC_RETRY_MS,
                    defmt::Debug2Format(&e)
                );
                Timer::after(Duration::from_millis(PMIC_RETRY_MS)).await;
            }
        }
    }

    defmt::info!("boot complete — idle heartbeat");
    loop {
        Timer::after(Duration::from_secs(5)).await;
        defmt::debug!("heartbeat");
    }
}
