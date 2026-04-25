//! I²C bus address probe.
//!
//! Brings up the shared I²C bus (AXP2101 → AW9523 release sequence),
//! sweeps every 7-bit address from `0x08` to `0x77`, and logs which
//! addresses ACK a 1-byte read. Used to confirm chip presence before
//! committing to address constants in driver crates that don't have a
//! published datasheet (`si12t`, `st25r3916`).
//!
//! Output (one pass, then halt):
//!
//! ```text
//! i2c-probe: sweep 0x08..=0x77
//! i2c-probe: 0x20 ACK (AW9523 expected)
//! i2c-probe: 0x34 ACK (AXP2101 expected)
//! i2c-probe: 0x36 ACK (AW88298 expected)
//! i2c-probe: ...
//! i2c-probe: sweep complete — 9 addresses ACKed
//! ```
//!
//! Addresses match the chip catalog when known; unrecognised ACKs are
//! flagged `(UNKNOWN)` so a hidden chip — or a misconfigured probe —
//! stands out.

#![no_std]
#![no_main]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use embassy_time::{Delay, Timer};
use embedded_hal_async::i2c::I2c;
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use stackchan_firmware::board;

use esp_println as _;

defmt::timestamp!("{=u64} ms", embassy_time::Instant::now().as_millis());

esp_bootloader_esp_idf::esp_app_desc!();

#[used]
static _APP_DESC_ANCHOR: &esp_bootloader_esp_idf::EspAppDesc = &ESP_APP_DESC;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("i2c-probe panic: {}", defmt::Display2Format(info));
    loop {}
}

const HEAP_SIZE: usize = 32 * 1024;

/// Lower bound of the standard 7-bit user address range. `0x00..=0x07`
/// are reserved (general call, CBUS, etc.).
const ADDR_LO: u8 = 0x08;
/// Upper bound of the standard 7-bit user address range. `0x78..=0x7F`
/// are reserved for 10-bit-addressing extension.
const ADDR_HI: u8 = 0x77;

/// Known-chip lookup. Each entry: `(7-bit address, label)`. Labels
/// describe the chip we EXPECT at that address based on this firmware's
/// driver crates and a verified probe of the M5Stack CoreS3 +
/// Stack-chan body BOM. An ACK at an address NOT in this table is
/// logged as `(UNKNOWN)`.
///
/// `BMM150` is intentionally absent: it sits behind BMI270's auxiliary
/// I²C interface (Bosch reference topology) and is invisible to a
/// main-bus probe by design. See the unit-hardware memory entry.
const KNOWN: &[(u8, &str)] = &[
    (0x23, "LTR-553"),
    (0x34, "AXP2101"),
    (0x36, "AW88298"),
    (0x38, "FT6336U"),
    (0x40, "ES7210"),
    (0x50, "Si12T"),
    (0x51, "BM8563"),
    (0x58, "AW9523"),
    (0x68, "BMI270"),
    (0x69, "BMI270 (SDO=high alt)"),
];

/// Look up the known-chip label for an address, or `None` if no entry.
fn label_for(addr: u8) -> Option<&'static str> {
    KNOWN
        .iter()
        .find(|(a, _)| *a == addr)
        .map(|(_, label)| *label)
}

/// Probe a single 7-bit address with a 1-byte read. Returns `true` if
/// the chip ACKed the address byte (regardless of read content), `false`
/// on bus error / NACK.
async fn probe<B: I2c>(bus: &mut B, addr: u8) -> bool {
    let mut buf = [0u8; 1];
    bus.read(addr, &mut buf).await.is_ok()
}

/// Read register `0x00` from the chip at `addr` (write-then-read).
/// Many chips expose `WHO_AM_I` / `CHIP_ID` here (BMI270 = 0x24,
/// BMM150 = 0x32, LIS3DH = 0x33, MPU6050 = 0x68, etc.) — useful for
/// disambiguating UNKNOWN ACKs without a separate datasheet hunt.
/// Returns `Some(byte)` on success, `None` on bus error.
async fn read_reg0<B: I2c>(bus: &mut B, addr: u8) -> Option<u8> {
    let mut buf = [0u8; 1];
    bus.write_read(addr, &[0x00], &mut buf).await.ok()?;
    Some(buf[0])
}

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
        "stackchan-i2c-probe v{} — boot, will sweep I²C0",
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

    // Brief settle after the AXP2101 / AW9523 init so power rails are
    // up before we start hitting addresses.
    Timer::after_millis(50).await;

    defmt::info!(
        "i2c-probe: sweep 0x{=u8:02X}..=0x{=u8:02X}",
        ADDR_LO,
        ADDR_HI
    );

    let mut i2c = board_io.i2c;
    let mut acked = 0u32;
    for addr in ADDR_LO..=ADDR_HI {
        if probe(&mut i2c, addr).await {
            acked += 1;
            // Read register 0x00 to capture WHO_AM_I / CHIP_ID where
            // the chip exposes one. Logged as "id=??" if the read
            // fails (some chips refuse register-read after a bare
            // address-byte read; not fatal).
            let id = read_reg0(&mut i2c, addr).await;
            match (label_for(addr), id) {
                (Some(label), Some(byte)) => defmt::info!(
                    "i2c-probe: 0x{=u8:02X} ACK ({=str} expected) reg0=0x{=u8:02X}",
                    addr,
                    label,
                    byte,
                ),
                (Some(label), None) => defmt::info!(
                    "i2c-probe: 0x{=u8:02X} ACK ({=str} expected) reg0=??",
                    addr,
                    label,
                ),
                (None, Some(byte)) => defmt::info!(
                    "i2c-probe: 0x{=u8:02X} ACK (UNKNOWN) reg0=0x{=u8:02X}",
                    addr,
                    byte,
                ),
                (None, None) => {
                    defmt::info!("i2c-probe: 0x{=u8:02X} ACK (UNKNOWN) reg0=??", addr);
                }
            }
        }
    }

    defmt::info!(
        "i2c-probe: sweep complete — {=u32} addresses ACKed in 0x{=u8:02X}..=0x{=u8:02X}",
        acked,
        ADDR_LO,
        ADDR_HI,
    );

    // Halt — no point looping; the bus state is static.
    loop {
        Timer::after_millis(60_000).await;
    }
}
