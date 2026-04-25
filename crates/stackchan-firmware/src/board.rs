//! CoreS3 board bring-up — power + servo bus init shared between the
//! main firmware binary and the `examples/` binaries.
//!
//! [`bringup`] owns the full sequence:
//!
//! 1. Internal I²C0 on GPIO11/12 → AXP2101 → AW9523 → PY32 (enable
//!    servo power pin via the [`py32`] crate, 200 ms settle).
//! 2. External UART1 on GPIO6/7 at 1 Mbaud for the `SCServo` bus.
//! 3. `SCServo::ping` each configured servo ID, log presence.
//! 4. Run the boot-nod gesture so the head visibly exercises both
//!    axes before the main control pipeline takes over.
//! 5. Park the I²C0 bus in a shared-bus wrapper so post-boot consumers
//!    (touch, future RTC / IMU) can hold handles to it concurrently.
//!
//! The function returns a [`BoardIo`] with both the fully-initialised
//! [`HeadDriverImpl`] **and** a [`SharedI2c`] handle on the internal
//! bus. It does **not** touch the LCD peripherals — that stays in the
//! caller so the bench examples can skip the SPI init.
//!
//! ## Philosophy
//!
//! Errors at each step are handled in place: a failing sub-step logs
//! `warn` or `error` via defmt and carries on. A missing PY32 or
//! disconnected servos should never blank the face. The only step
//! that panics on failure is AXP2101 init, and only because no
//! forward progress is possible without the LDOs.

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Delay, Duration, Timer, with_timeout};
use esp_hal::{
    i2c::master::{Config as I2cConfig, I2c},
    peripherals::{GPIO6, GPIO7, GPIO11, GPIO12, I2C0, UART1},
    time::Rate,
    uart::{Config as UartConfig, Uart},
};
use scservo::Scservo;
use stackchan_core::{Clock, HeadDriver, Pose};
use static_cell::StaticCell;

use crate::clock::HalClock;
use crate::head;

/// Concrete type of the `SCServo`-backed head driver, needed so
/// `#[embassy_executor::task]` (which forbids generic tasks) can accept
/// it as an argument.
pub type HeadDriverImpl = head::ScsHead<Uart<'static, esp_hal::Async>>;

/// Internal I²C0 bus wrapped in an async [`Mutex`].
///
/// Consumers never name this directly; they hold [`SharedI2c`] handles
/// instead. The `NoopRawMutex` is correct here because the esp-rtos
/// executor on this chip is single-threaded — cross-core / cross-
/// executor safety is not meaningful.
pub type SharedI2cBus = Mutex<NoopRawMutex, I2c<'static, esp_hal::Async>>;

/// Cloneable handle onto [`SharedI2cBus`].
///
/// Every I²C consumer (touch task, future RTC / IMU tasks) receives
/// its own `SharedI2c` and uses it exactly like a directly-owned bus:
/// the [`I2cDevice`] wrapper transparently locks the mutex around
/// each transaction.
pub type SharedI2c = I2cDevice<'static, NoopRawMutex, I2c<'static, esp_hal::Async>>;

/// Values [`bringup`] hands back to the caller.
///
/// Contains the servo bus driver + a shared-handle onto the internal
/// I²C0 bus. Further handles onto the same bus can be cloned by
/// calling [`I2cDevice::new`] on the [`SharedI2cBus`] reference
/// returned via [`BoardIo::i2c_bus`].
pub struct BoardIo {
    /// The initialised servo-bus driver, already `PINGed` and
    /// post-boot-nod.
    pub head: HeadDriverImpl,
    /// One shared-bus handle onto internal I²C0, ready to be consumed
    /// by the first I²C task (typically the touch poll task).
    pub i2c: SharedI2c,
    /// Reference to the underlying shared-bus mutex so callers can
    /// spawn additional [`I2cDevice`] handles for further peripherals
    /// (future RTC / IMU / sensors).
    pub i2c_bus: &'static SharedI2cBus,
}

/// Milliseconds to wait after raising the servo-power pin on the PY32.
/// Matches the 200 ms delay in `hal_io_expander.cpp`.
const SERVO_POWER_SETTLE_MS: u64 = 200;

/// PY32 pin the servo-power rail MOSFET gate is wired to. HIGH enables
/// the rail — see `device_specs.md`.
const PY32_SERVO_POWER_PIN: u8 = 0;

/// Retry delay between failed AXP2101 init attempts. Covers transient
/// I²C glitches during cold boot.
const PMIC_RETRY_MS: u64 = 500;

/// `SCServo` UART baud rate. Feetech `SCSCL` family default is 1 Mbaud.
const SERVO_UART_BAUD: u32 = 1_000_000;

/// PING timeout per servo, in ms. 10 ms is well past the ~200 µs
/// round-trip of a 1 Mbaud PING packet.
const PING_TIMEOUT_MS: u64 = 10;

/// Wall-clock pace between boot-nod steps. Matches the servos'
/// single-leg move budget (`MOVE_TIME_MS` in `head.rs`).
const BOOT_NOD_STEP_MS: u64 = 170;

/// Full CoreS3 bring-up for the servo subsystem.
///
/// Consumes the I²C / UART peripherals + their GPIOs, returns a
/// ready-to-drive [`HeadDriverImpl`] with both servos `PINGed` and the
/// boot-nod gesture already executed. Caller keeps ownership of
/// unrelated peripherals (SPI2, LCD GPIOs, etc.) for further init.
///
/// # Panics
///
/// Panics if AXP2101 init can't eventually succeed — the LDOs feed
/// the rest of the board, so halting here surfaces a wiring failure
/// loudly rather than silently limping.
pub async fn bringup(
    i2c_periph: I2C0<'static>,
    uart_periph: UART1<'static>,
    sda: GPIO12<'static>,
    scl: GPIO11<'static>,
    uart_tx: GPIO6<'static>,
    uart_rx: GPIO7<'static>,
    delay: &mut Delay,
) -> BoardIo {
    // Internal I²C0: 100 kHz standard-mode. The 400 kHz bump from
    // commit 41325ee assumed every device on the bus could hold
    // fast-mode timing, but empirically the CoreS3's shared bus (12
    // devices, heavy trace capacitance, single pull-up network) makes
    // PY32 (0x6F) NACK data bytes, BMI270 time out, BMM150 undetectable,
    // and FT6336U drop a bit out of its vendor-ID read — all signal-
    // integrity failures that vanish at 100 kHz. The original 400 kHz
    // motivation was BMI270's 64-chunk config-blob upload tripping
    // timeouts mid-init; PR #35's per-chunk settle delay covers that
    // at the driver edge, so we can go back to the safe bus speed.
    let i2c_cfg = I2cConfig::default().with_frequency(Rate::from_khz(100));
    let i2c = match I2c::new(i2c_periph, i2c_cfg) {
        Ok(bus) => bus.with_sda(sda).with_scl(scl).into_async(),
        Err(e) => defmt::panic!("I2C0 config rejected: {}", defmt::Debug2Format(&e)),
    };
    defmt::debug!("I2C0 ready on GPIO12/11 @ 100 kHz");

    let mut pmic = axp2101::Axp2101::new(i2c);
    loop {
        match pmic.init_cores3().await {
            Ok(()) => {
                defmt::info!(
                    "AXP2101: CoreS3 power defaults applied — LCD rails + power-key timing set"
                );
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

    let mut i2c = pmic.into_inner();
    match aw9523::init_cores3(&mut i2c, delay).await {
        Ok(()) => defmt::info!("AW9523: CoreS3 defaults applied, LCD reset pulsed (P1_1)"),
        Err(e) => defmt::panic!("AW9523 init failed: {}", defmt::Debug2Format(&e)),
    }

    enable_servo_power(&mut i2c).await;
    Timer::after(Duration::from_millis(SERVO_POWER_SETTLE_MS)).await;

    // Park the I²C0 bus in a shared-bus mutex. Any post-boot consumer
    // (touch, future RTC / IMU) gets its own cheap-to-create
    // `I2cDevice` handle by calling `I2cDevice::new(i2c_bus)`.
    //
    // The `StaticCell` lives for the entire binary, so its
    // `Mutex<...>` reference is `'static` — which every `I2cDevice<'_,
    // ...>` handle also needs to be to cross a task boundary. Each
    // firmware binary calls `bringup` exactly once; double-init
    // `StaticCell::init` is defensively caught by the runtime.
    #[allow(clippy::items_after_statements)]
    static I2C_BUS: StaticCell<SharedI2cBus> = StaticCell::new();
    let i2c_bus: &'static SharedI2cBus = I2C_BUS.init(Mutex::new(i2c));
    let shared_i2c = I2cDevice::new(i2c_bus);
    defmt::debug!("I2C0 shared-bus wrapper ready (post-boot consumers may now attach)");

    // External UART1 for the SCServo bus.
    let uart_cfg = UartConfig::default().with_baudrate(SERVO_UART_BAUD);
    let servo_uart = match Uart::new(uart_periph, uart_cfg) {
        Ok(uart) => uart.with_tx(uart_tx).with_rx(uart_rx).into_async(),
        Err(e) => defmt::panic!("UART1 config rejected: {}", defmt::Debug2Format(&e)),
    };
    defmt::info!(
        "SCServo bus ready on UART1 (TX=GPIO6, RX=GPIO7) @ {=u32} baud",
        SERVO_UART_BAUD
    );
    let mut scs_head = head::ScsHead::new(Scservo::new(servo_uart));

    // Servo presence check. UART writes don't NACK on missing slaves,
    // so without this probe we can't tell the bus is alive.
    ping_servo(&mut scs_head, head::YAW_SERVO_ID).await;
    ping_servo(&mut scs_head, head::PITCH_SERVO_ID).await;

    // Explicitly enable torque on both servos. Factory default is
    // usually enabled, but some kits ship with torque off on one axis
    // (e.g. the tilt servo left loose so the head can be manually
    // positioned during assembly). Without torque, `write_position`
    // is silently ignored and the axis sags under gravity — commands
    // succeeded on the wire but produced no motion.
    enable_torque(&mut scs_head, head::YAW_SERVO_ID).await;
    enable_torque(&mut scs_head, head::PITCH_SERVO_ID).await;

    // Log the servos' EEPROM angle-limit registers. If either servo
    // has limits narrower than the mechanical range (a common
    // consequence of past calibration experiments writing to
    // MIN/MAX_ANGLE_LIMIT), commanded positions are silently clipped
    // at those boundaries and the axis appears stuck. Visible in
    // the log so we know whether to rewrite the EEPROM.
    log_angle_limits(&mut scs_head, head::YAW_SERVO_ID).await;
    log_angle_limits(&mut scs_head, head::PITCH_SERVO_ID).await;

    // Visible proof of life: deliberate pan-then-tilt gesture before
    // the main control pipeline takes over.
    boot_nod(&mut scs_head).await;

    BoardIo {
        head: scs_head,
        i2c: shared_i2c,
        i2c_bus,
    }
}

/// Drive the PY32's servo-power pin HIGH via the `py32` crate.
///
/// Uses [`py32::Py32::configure_output_pin`], which does read-modify-
/// write on the direction / pull-up / output registers. That's a change
/// from v0.1.0's inline three-write sequence: it cooperates cleanly
/// with any future multi-pin PY32 traffic (e.g. LED fan-out on pin 13),
/// whereas the old path only worked because the chip's GPIO registers
/// happen to reset to zero.
///
/// `Py32::new` takes ownership of its bus, but the blanket impl
/// `impl<T: I2c> I2c for &mut T` in embedded-hal-async 1.0 lets us
/// lend the bus via a mutable borrow and drop the `Py32` handle at
/// end of scope, releasing the borrow for the shared-bus wrapper.
///
/// Failures are logged at `warn` and the function returns; the servo
/// bus will still initialise but `ping_servo` will time out, which is
/// the already-handled path for "servos missing".
async fn enable_servo_power(i2c: &mut I2c<'static, esp_hal::Async>) {
    let mut py = py32::Py32::new(i2c);
    match py.configure_output_pin(PY32_SERVO_POWER_PIN, true).await {
        Ok(()) => defmt::info!(
            "PY32: servo power enabled (pin {=u8} HIGH)",
            PY32_SERVO_POWER_PIN
        ),
        Err(e) => defmt::warn!(
            "PY32: servo-power enable incomplete ({}) — servos may stay unpowered",
            defmt::Debug2Format(&e)
        ),
    }
}

/// Probe one `SCServo` ID and log the outcome. 10 ms is well past the
/// ~200 µs round-trip of a 1 Mbaud PING packet, so a miss here is a
/// real "servo not responding" signal.
async fn ping_servo(driver: &mut HeadDriverImpl, id: u8) {
    let bus = driver.bus_mut();
    match embassy_time::with_timeout(
        embassy_time::Duration::from_millis(PING_TIMEOUT_MS),
        bus.ping(id),
    )
    .await
    {
        Ok(Ok(())) => defmt::info!("SCServo[{=u8}]: present", id),
        Ok(Err(e)) => defmt::warn!(
            "SCServo[{=u8}]: malformed response: {}",
            id,
            defmt::Debug2Format(&e)
        ),
        Err(_) => defmt::warn!(
            "SCServo[{=u8}]: no response within {=u64} ms (disconnected or unpowered?)",
            id,
            PING_TIMEOUT_MS
        ),
    }
}

/// Enable torque on one `SCServo` ID so it actually holds the
/// commanded position. Without this, the servo accepts
/// `write_position` frames but applies no force — the axis sags
/// under gravity and visible motion never happens. Failures log
/// at `warn` and return; the downstream pipeline will still try
/// to drive the axis (and its writes will keep being ignored,
/// which is an observable failure mode at least).
/// Read and log the servo's EEPROM angle-limit window so we can
/// tell the difference between a mechanically jammed axis and one
/// that's being silently clipped by factory / leftover-calibration
/// register values.
///
/// Feetech SCSCL memory table (big-endian u16 each):
/// - `MIN_ANGLE_LIMIT` = address `0x09`
/// - `MAX_ANGLE_LIMIT` = address `0x0B`
///
/// Read via a single 4-byte `read_memory` starting at the MIN addr;
/// returns [min, max] in raw position counts (0..=1023 space).
/// Failure logs at `warn` — the diagnostic isn't load-bearing.
async fn log_angle_limits(driver: &mut HeadDriverImpl, id: u8) {
    const ADDR_MIN_ANGLE_LIMIT: u8 = 0x09;
    let mut buf = [0u8; 4];
    let bus = driver.bus_mut();
    match bus.read_memory(id, ADDR_MIN_ANGLE_LIMIT, &mut buf).await {
        Ok(_err_byte) => {
            let min = (u16::from(buf[0]) << 8) | u16::from(buf[1]);
            let max = (u16::from(buf[2]) << 8) | u16::from(buf[3]);
            defmt::debug!(
                "SCServo[{=u8}]: angle limits MIN={=u16} MAX={=u16} (pos counts; full range is 0..=1023)",
                id,
                min,
                max,
            );
        }
        Err(e) => defmt::warn!(
            "SCServo[{=u8}]: read angle-limits failed: {}",
            id,
            defmt::Debug2Format(&e)
        ),
    }
}

/// Enable the servo's torque output, then drain the post-write status
/// frame so it does not collide with the next command. Logs and
/// continues on error — a silent servo must not wedge boot.
async fn enable_torque(driver: &mut HeadDriverImpl, id: u8) {
    let bus = driver.bus_mut();
    match bus.write_torque_enable(id, true).await {
        Ok(()) => defmt::debug!("SCServo[{=u8}]: torque enabled", id),
        Err(e) => defmt::warn!(
            "SCServo[{=u8}]: torque-enable failed: {}",
            id,
            defmt::Debug2Format(&e)
        ),
    }
    // Consume the servo's post-write status response (same RX
    // housekeeping as `set_pose` — see `head.rs` for the rationale).
    // A silent servo makes the drain hang, so it's time-bounded.
    let _ = with_timeout(Duration::from_millis(2), bus.drain_write_status()).await;
}

/// Boot-time "yes" nod: tilt up → tilt down → center over ~0.5 s.
/// Each step waits [`BOOT_NOD_STEP_MS`] for the servo's internal
/// interpolation to complete before commanding the next. Write
/// errors log + continue.
///
/// Pure tilt motion — a nod is the up/down "yes" gesture by
/// definition. The asymmetric amplitudes (+8° up, -3° down) account
/// for the factory horn offset: many kits ship with position 512
/// biased a few degrees below horizontal (head weight on the pitch
/// linkage), so aggressive downward travel risks hitting the hard
/// stop while upward travel has room. Users who want a symmetric
/// nod can calibrate `TILT_TRIM_DEG` in `head.rs` via `just bench`
/// and restore a ±X°/±X° pattern.
async fn boot_nod(driver: &mut HeadDriverImpl) {
    let clock = HalClock;
    defmt::info!("boot-nod: yes nod start");
    for (pan, tilt, label) in [
        (0.0, 8.0, "tilt+8"),
        (0.0, -3.0, "tilt-3"),
        (0.0, 0.0, "tilt 0"),
    ] {
        if let Err(e) = driver.set_pose(Pose::new(pan, tilt), clock.now()).await {
            defmt::warn!("boot-nod step {}: {}", label, defmt::Debug2Format(&e));
        }
        Timer::after(Duration::from_millis(BOOT_NOD_STEP_MS)).await;
    }
    defmt::info!("boot-nod: complete");
}
