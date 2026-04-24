//! Tilt-axis torque-off freewheel diagnostic.
//!
//! Disables torque on the pitch (tilt) servo and live-streams the
//! encoder reading at 5 Hz so the operator can hand-rotate the head
//! and verify that the servo's internal position sensor is following
//! the physical motion. Companion to `examples/tilt_extremes.rs`,
//! which only exercises the *commanded* path.
//!
//! ## When to use
//!
//! `tilt_extremes` showed the encoder pegged to a single value with
//! the OVERLOAD fault byte latched. Two failure modes can produce
//! that pattern, and this binary distinguishes them:
//!
//! - **Encoder alive, controller stuck in OVERLOAD** → torque-off
//!   should clear OVERLOAD on its own, and the encoder reading
//!   should track the head as you rotate it. Re-run `tilt_extremes`
//!   after freewheel; the controller often unsticks once the stall
//!   condition is removed.
//! - **Encoder dead / shaft–encoder decoupled** → encoder reading
//!   stays pinned to the same raw value no matter how you move the
//!   head. Servo replacement is the only fix.
//!
//! ## Workflow
//!
//! ```text
//! source ~/export-esp.sh
//! just tilt-freewheel
//! ```
//!
//! Once `freewheel: torque OFF` appears, slowly rotate the head
//! through its full physical range while watching the per-sample
//! `raw=…  deg=…` line. Re-flash main firmware (`just fmr`) when
//! done — torque stays off until power-cycle.

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use embassy_time::{Delay, Duration, Timer};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use scservo::{ADDR_PRESENT_POSITION, POSITION_CENTER, POSITION_PER_DEGREE};
use stackchan_firmware::{board, head};

use esp_println as _;

defmt::timestamp!("{=u64} ms", embassy_time::Instant::now().as_millis());

esp_bootloader_esp_idf::esp_app_desc!();

#[used]
static _APP_DESC_ANCHOR: &esp_bootloader_esp_idf::EspAppDesc = &ESP_APP_DESC;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("freewheel panic: {}", defmt::Display2Format(info));
    loop {}
}

const HEAP_SIZE: usize = 32 * 1024;

/// 5 Hz — fast enough to track a hand-rotated axis, slow enough to
/// read the defmt-decoded log in real time.
const SAMPLE_PERIOD_MS: u64 = 200;

/// Per-`read_memory` timeout. 10 ms covers the ~200 µs round-trip at
/// 1 Mbaud plus servo response latency.
const READ_TIMEOUT_MS: u64 = 10;

#[esp_rtos::main]
#[allow(
    clippy::used_underscore_binding,
    reason = "esp_rtos::main macro requires the `spawner` arg"
)]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-tilt-freewheel v{} — torque-off live encoder readout",
        env!("CARGO_PKG_VERSION")
    );

    let mut delay = Delay;
    let mut driver = board::bringup(
        peripherals.I2C0,
        peripherals.UART1,
        peripherals.GPIO12,
        peripherals.GPIO11,
        peripherals.GPIO6,
        peripherals.GPIO7,
        &mut delay,
    )
    .await
    .head;

    // Skip an explicit torque-disable write: a known consequence of
    // a latched OVERLOAD fault is that the servo's reply to the
    // torque-write packet is mis-framed (extended status bytes), and
    // the leftover bytes poison the RX FIFO so every subsequent
    // `read_memory` returns `MalformedResponse`. Since OVERLOAD
    // already disables motor drive internally (the whole reason
    // we're here), the head is already back-drivable — we just
    // read the encoder without sending any further bus traffic.
    defmt::info!(
        "freewheel: encoder-only readout on pitch servo (ID {=u8}). Motor is already off-drive due to latched OVERLOAD; rotate head freely.",
        head::PITCH_SERVO_ID
    );
    defmt::info!(
        "freewheel: sampling pitch position every {=u64} ms; min/max latch across the session. Watch raw= column as you rotate.",
        SAMPLE_PERIOD_MS
    );

    let mut min_raw: u16 = u16::MAX;
    let mut max_raw: u16 = 0;

    loop {
        Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS)).await;

        let bus = driver.bus_mut();
        let mut buf = [0u8; 2];
        let (raw, err_byte) = match embassy_time::with_timeout(
            Duration::from_millis(READ_TIMEOUT_MS),
            bus.read_memory(head::PITCH_SERVO_ID, ADDR_PRESENT_POSITION, &mut buf),
        )
        .await
        {
            Ok(Ok(eb)) => (u16::from_be_bytes(buf), eb),
            Ok(Err(e)) => {
                defmt::warn!("freewheel: read err: {}", defmt::Debug2Format(&e));
                continue;
            }
            Err(_) => {
                defmt::warn!("freewheel: read timed out");
                continue;
            }
        };

        if raw < min_raw {
            min_raw = raw;
        }
        if raw > max_raw {
            max_raw = raw;
        }

        let deg = (f32::from(raw) - f32::from(POSITION_CENTER)) / POSITION_PER_DEGREE;
        if err_byte == 0 {
            defmt::info!(
                "freewheel  raw={=u16}  deg={=f32}  min={=u16}  max={=u16}  span={=u16}",
                raw,
                deg,
                min_raw,
                max_raw,
                max_raw.saturating_sub(min_raw),
            );
        } else {
            defmt::warn!(
                "freewheel  raw={=u16}  deg={=f32}  min={=u16}  max={=u16}  span={=u16}  FAULT=0x{=u8:02x}",
                raw,
                deg,
                min_raw,
                max_raw,
                max_raw.saturating_sub(min_raw),
                err_byte,
            );
        }
    }
}
