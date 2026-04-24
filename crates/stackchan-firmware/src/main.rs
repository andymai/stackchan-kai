//! StackChan firmware for the M5Stack CoreS3.
//!
//! Boot sequence: esp-hal init → esp-rtos embassy → AXP2101 LDOs →
//! AW9523 releases LCD reset → SPI2 + ILI9342C via mipidsi → one-shot
//! render of the default `Avatar`. The animated render loop (BlinkModifier
//! et al. @ 30 FPS) lands in a follow-up PR; isolating the draw pipeline
//! from the animation state machine keeps the regression surface small.

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
// Firmware needs unsafe for one narrow reason: a `Sync` promise on a
// raw-pointer newtype used as an LTO anchor for the ESP-IDF app
// descriptor. No pointer dereference happens here. The workspace-wide
// `unsafe_code = deny` rule still applies to the host crates.
#![allow(unsafe_code)]

extern crate alloc;

mod aw9523;

use axp2101::Axp2101;
use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::{
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig},
    i2c::master::{Config as I2cConfig, I2c},
    spi::{
        Mode as SpiMode,
        master::{Config as SpiConfig, Spi},
    },
    time::Rate,
    timer::timg::TimerGroup,
};
use mipidsi::{
    Builder,
    interface::SpiInterface,
    models::ILI9342CRgb565,
    options::{ColorInversion, ColorOrder},
};
use stackchan_core::Avatar;
use static_cell::StaticCell;

// esp-println registers a `#[defmt::global_logger]` that writes
// defmt-encoded bytes to the USB-Serial-JTAG peripheral. Importing for
// side effects only — no init call needed.
use esp_println as _;

// defmt 1.0 requires a timestamp provider linked into the binary.
// Embassy's `Instant::now()` reads the timer driver that esp-rtos has
// already started by the time the first log macro fires.
defmt::timestamp!("{=u64} ms", embassy_time::Instant::now().as_millis());

// The ESP-IDF second-stage bootloader reads an `app_desc` struct at a
// fixed offset; the macro emits one in a dedicated linker section.
esp_bootloader_esp_idf::esp_app_desc!();

// The macro above emits `pub static ESP_APP_DESC` without `#[used]`,
// so `lto = "fat"` strips it and espflash refuses the image. Anchor
// its address in a `#[used]` static to keep it in .rodata_desc.appdesc.
// Raw pointers aren't `Sync` by default; wrap in a newtype and promise
// the invariant ourselves — the address is never read through this.
#[repr(transparent)]
struct AppDescAnchor(*const esp_bootloader_esp_idf::EspAppDesc);
// SAFETY: the anchor is never dereferenced; its only purpose is to hold
// a symbol reference so LTO cannot discard ESP_APP_DESC.
unsafe impl Sync for AppDescAnchor {}
#[used]
static _APP_DESC_ANCHOR: AppDescAnchor = AppDescAnchor(core::ptr::addr_of!(ESP_APP_DESC));

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
    // Spawner is unused in the static-face PR; the 30 FPS render task in the
    // next PR will consume it. Drop explicitly so the unused-var warning
    // can't mask a real regression.
    let _ = spawner;

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-firmware v{} — CoreS3 boot",
        env!("CARGO_PKG_VERSION")
    );

    // CoreS3 internal I²C bus: GPIO11 = SCL, GPIO12 = SDA.
    // (Confirmed against M5Unified source: `In SCL, SDA = GPIO_NUM_11, GPIO_NUM_12`.)
    // AXP2101 (0x34), AW9523 IO expander, and the touch controller all
    // sit on this bus. 100 kHz is the conservative standard-mode rate
    // that works from cold boot before PLLs are fully settled.
    let i2c_cfg = I2cConfig::default().with_frequency(Rate::from_khz(100));
    let i2c = match I2c::new(peripherals.I2C0, i2c_cfg) {
        Ok(bus) => bus
            .with_sda(peripherals.GPIO12)
            .with_scl(peripherals.GPIO11)
            .into_async(),
        Err(e) => defmt::panic!("I2C0 config rejected: {}", defmt::Debug2Format(&e)),
    };
    defmt::debug!("I2C0 ready on GPIO12/11 @ 100 kHz");

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

    // Reclaim the I²C bus from the PMIC driver; the AW9523 is the next (and
    // only other) I²C consumer in this PR, so a sequential hand-off avoids
    // pulling in embedded-hal-bus shared-bus machinery.
    let mut i2c = pmic.into_inner();
    match aw9523::release_lcd_reset(&mut i2c).await {
        Ok(()) => defmt::info!("AW9523: LCD reset released (P0_0 high)"),
        Err(e) => defmt::panic!("AW9523 reset-release failed: {}", defmt::Debug2Format(&e)),
    }
    // I²C is no longer needed in this PR; drop it so the compiler catches
    // any accidental later uses (touch/battery drivers come in future PRs).
    drop(i2c);

    // CoreS3 LCD (ILI9342C) on SPI2.
    //   SCK  = GPIO36
    //   MOSI = GPIO37
    //   CS   = GPIO3  (active low)
    //   DC   = GPIO35 (0 = command, 1 = data)
    //   RST  = AW9523 P0_0 (handled above — mipidsi sees `NoResetPin`)
    //   BL   = AXP2101 BLDO1 (handled by `enable_lcd_rails`)
    let spi_cfg = SpiConfig::default()
        .with_frequency(Rate::from_mhz(40))
        .with_mode(SpiMode::_0);
    let spi_bus = match Spi::new(peripherals.SPI2, spi_cfg) {
        Ok(bus) => bus
            .with_sck(peripherals.GPIO36)
            .with_mosi(peripherals.GPIO37),
        Err(e) => defmt::panic!("SPI2 config rejected: {}", defmt::Debug2Format(&e)),
    };
    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO35, Level::Low, OutputConfig::default());

    let spi_device = match ExclusiveDevice::new(spi_bus, cs, Delay) {
        Ok(dev) => dev,
        Err(e) => defmt::panic!("ExclusiveDevice init failed: {}", defmt::Debug2Format(&e)),
    };

    // mipidsi's `SpiInterface` batches pixel writes through a caller-owned
    // buffer. 4 KiB ≈ 2048 px per SPI transaction — a good speed/RAM balance
    // for 320x240 clears on the internal SRAM heap. Parked in a `StaticCell`
    // so the buffer outlives this frame and never needs reallocation.
    static SPI_DI_BUF: StaticCell<[u8; 4096]> = StaticCell::new();
    let spi_di_buf = SPI_DI_BUF.init([0u8; 4096]);
    let di = SpiInterface::new(spi_device, dc, spi_di_buf);

    // `NoResetPin` is implied by omitting `.reset_pin(...)` — the hardware
    // reset is already done via AW9523 above. BGR color order matches the
    // CoreS3 panel wiring; without `invert_colors` the image appears as a
    // color-inverted negative on this specific module.
    let mut display = match Builder::new(ILI9342CRgb565, di)
        .display_size(320, 240)
        .color_order(ColorOrder::Bgr)
        .invert_colors(ColorInversion::Inverted)
        .init(&mut Delay)
    {
        Ok(d) => d,
        Err(e) => defmt::panic!("mipidsi init failed: {}", defmt::Debug2Format(&e)),
    };
    defmt::info!("ILI9342C ready — rendering default Avatar");

    match Avatar::default().draw(&mut display) {
        Ok(()) => defmt::info!("Avatar drawn to LCD"),
        Err(e) => defmt::error!("Avatar::draw failed: {}", defmt::Debug2Format(&e)),
    }

    defmt::info!("boot complete — idle heartbeat");
    loop {
        Timer::after(Duration::from_secs(5)).await;
        defmt::debug!("heartbeat");
    }
}
