//! Tilt-axis extremes calibration bench.
//!
//! Drives the pitch (tilt) servo to a graduated sequence of commanded
//! angles past the firmware's normal `MAX_TILT_DEG` (±30°) safety
//! window — up to ±50° — and reads back the live encoder position at
//! each step. Wherever the readback stops climbing, that's the
//! effective upward stop; same for downward. Companion to
//! `examples/bench.rs`, which only exercises the conservative
//! ±20° trim-discovery range.
//!
//! The "stop" we measure is whichever of the two clips the servo
//! first: the EEPROM `MIN_ANGLE_LIMIT` / `MAX_ANGLE_LIMIT` registers
//! (logged at boot by `board::bringup`), or the physical hard stop in
//! the linkage. Compare the SUMMARY line below against the boot-log
//! angle limits to disambiguate:
//!
//! - SUMMARY plateau matches angle-limit raw count → EEPROM is
//!   clipping; rewrite the limits to widen the window.
//! - SUMMARY plateau is well inside the angle-limit raw count → it's
//!   the physical linkage hitting a stop; calibrate `TILT_TRIM_DEG`
//!   to the observed midpoint or check for mechanical interference.
//!
//! ## Workflow
//!
//! ```text
//! source ~/export-esp.sh
//! just tilt-extremes
//! ```
//!
//! The binary sweeps once and halts; re-flash main firmware with
//! `just fmr` when done. Output per step:
//!
//! ```text
//! tilt-extremes  cmd=+25.00  raw=597  actual_deg=+24.93  delta=-0.07
//! ```
//!
//! Final SUMMARY line reports the observed `actual_deg` extremes plus
//! suggested EEPROM angle-limit values + a `TILT_TRIM_DEG` to centre
//! commanded zero on the mechanical midpoint.

#![no_std]
#![no_main]
// Same allowances as `examples/bench.rs`.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::future_not_send)]

extern crate alloc;

use embassy_time::{Delay, Duration, Timer};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};
use scservo::{ADDR_PRESENT_POSITION, POSITION_CENTER, POSITION_PER_DEGREE};
use stackchan_core::{Clock, HeadDriver, Pose};
use stackchan_firmware::{board, clock::HalClock, head};

// esp-println registers the global defmt logger via USB-Serial-JTAG.
use esp_println as _;

defmt::timestamp!("{=u64} ms", embassy_time::Instant::now().as_millis());

esp_bootloader_esp_idf::esp_app_desc!();

/// LTO anchor — see `examples/bench.rs` for the rationale.
#[used]
static _APP_DESC_ANCHOR: &esp_bootloader_esp_idf::EspAppDesc = &ESP_APP_DESC;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("tilt-extremes panic: {}", defmt::Display2Format(info));
    loop {}
}

/// 32 KiB matches `examples/bench.rs` — no framebuffer here.
const HEAP_SIZE: usize = 32 * 1024;

/// Dwell after each commanded angle before reading the encoder. 400 ms
/// is twice bench's 300 ms — extreme commands need extra settle time
/// because the servo's interpolation may stall against a hard stop
/// before the moving-flag clears.
const DWELL_MS: u64 = 400;

/// Per-`read_position` timeout. 10 ms covers the ~200 µs round-trip
/// at 1 Mbaud plus the servo's response latency, with margin.
const READ_TIMEOUT_MS: u64 = 10;

/// "Plateau" threshold in degrees: if successive `actual_deg` readings
/// differ by less than this, we've hit a stop. 1.0° corresponds to
/// ~3.4 raw counts — well above encoder noise + interpolation jitter.
const PLATEAU_DELTA_DEG: f32 = 1.0;

/// Sweep step in degrees. 5° gives 10 samples between 0° and ±50° —
/// fine enough to localise a stop within the noise budget without
/// needlessly hammering the servo.
const STEP_DEG: f32 = 5.0;

/// Outer-bound commanded angle. The Stack-chan tilt servo has ~90°
/// of mechanical range; setting the probe ceiling to 95° pushes
/// just past the expected hard stop so the plateau detector reliably
/// catches it. SCSCL position counts saturate at 0 / 1023 well past
/// this (~±150°), so the value is safe for the wire protocol.
const MAX_PROBE_DEG: f32 = 95.0;

/// Step count from 0° to `MAX_PROBE_DEG` inclusive, in one direction.
/// Hardcoded rather than computed because `f32::ceil` / `f32::abs`
/// aren't in core no_std without pulling in `libm`. Update if
/// `MAX_PROBE_DEG` / `STEP_DEG` change — the runtime assert below
/// catches drift.
const MAX_STEPS: u32 = 19;

#[esp_rtos::main]
#[allow(
    clippy::used_underscore_binding,
    reason = "esp_rtos::main macro requires the `spawner` arg; extremes doesn't spawn"
)]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-tilt-extremes v{} — commanded sweep to find tilt mechanical / EEPROM stops",
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

    // --- Servo health snapshot before the sweep ---------------------------
    // Reads voltage / temperature / present-position-with-error-byte once,
    // so the operator can see if the pitch servo has latched a fault
    // (overload, undervoltage, overheat) that would prevent it from
    // moving regardless of commanded position. Failures here are non-
    // fatal — log and proceed.
    log_health(&mut driver, head::PITCH_SERVO_ID).await;

    // --- Up-sweep: 0° → +MAX_PROBE_DEG, stop early if encoder plateaus -----
    defmt::info!(
        "tilt-extremes: starting UP sweep (0° → +{=f32}°)",
        MAX_PROBE_DEG
    );
    let upper = sweep(&mut driver, "up", STEP_DEG).await;

    // Return to centre before reversing direction so we don't whip
    // through 0° at full servo speed.
    let _ = command(&mut driver, Pose::new(0.0, 0.0)).await;
    Timer::after(Duration::from_millis(DWELL_MS)).await;

    // --- Down-sweep: 0° → -MAX_PROBE_DEG, same plateau exit -----------------
    defmt::info!(
        "tilt-extremes: starting DOWN sweep (0° → -{=f32}°)",
        MAX_PROBE_DEG
    );
    let lower = sweep(&mut driver, "down", -STEP_DEG).await;

    // Centre, log summary, halt.
    let _ = command(&mut driver, Pose::NEUTRAL).await;
    Timer::after(Duration::from_millis(DWELL_MS)).await;

    print_summary(lower, upper);

    defmt::info!(
        "tilt-extremes complete — re-flash main firmware with `just fmr` to resume normal boot"
    );
    loop {
        Timer::after(Duration::from_secs(5)).await;
    }
}

/// Drive tilt from 0° outward in `step_deg` increments (positive for
/// up-sweep, negative for down-sweep) until either the encoder
/// plateaus or we reach `MAX_PROBE_DEG`. Returns the last *actual*
/// angle observed before the plateau (or at the last step, if no
/// plateau was hit).
async fn sweep(driver: &mut board::HeadDriverImpl, label: &'static str, step_deg: f32) -> f32 {
    let mut prev_actual_deg: f32 = 0.0;
    let mut last_actual_deg: f32 = 0.0;
    let mut cmd_deg: f32 = 0.0;
    debug_assert!(
        f32::from(u16::try_from(MAX_STEPS).unwrap_or(u16::MAX)) * STEP_DEG
            >= MAX_PROBE_DEG - f32::EPSILON,
        "MAX_STEPS out of sync with MAX_PROBE_DEG / STEP_DEG"
    );

    for i in 0..=MAX_STEPS {
        // u32 ≤ 10 is exact in f32. step_deg is ±STEP_DEG, so cmd_deg
        // ranges 0..±MAX_PROBE_DEG inclusive without overshoot.
        #[allow(
            clippy::cast_precision_loss,
            reason = "i ≤ MAX_STEPS (10), losslessly representable in f32"
        )]
        let i_f = i as f32;
        cmd_deg = step_deg * i_f;

        if let Err(e) = command(driver, Pose::new(0.0, cmd_deg)).await {
            defmt::warn!(
                "tilt-extremes {}: set_pose err at cmd={=f32}: {}",
                label,
                cmd_deg,
                defmt::Debug2Format(&e)
            );
            return last_actual_deg;
        }
        Timer::after(Duration::from_millis(DWELL_MS)).await;

        let raw = match read_pitch(driver).await {
            Some(p) => p,
            None => continue,
        };
        let actual_deg = raw_to_deg(raw);
        let delta = actual_deg - cmd_deg;
        defmt::info!(
            "tilt-extremes {}: cmd={=f32}  raw={=u16}  actual_deg={=f32}  delta={=f32}",
            label,
            cmd_deg,
            raw,
            actual_deg,
            delta,
        );

        // Plateau detection: skip the first sample (no prior reading
        // to compare against) and the centring sample at i=0.
        if i > 0 && (actual_deg - prev_actual_deg).abs() < PLATEAU_DELTA_DEG {
            defmt::info!(
                "tilt-extremes {}: PLATEAU detected at cmd={=f32} (actual={=f32}, prev={=f32}); stopping sweep",
                label,
                cmd_deg,
                actual_deg,
                prev_actual_deg,
            );
            return actual_deg;
        }
        prev_actual_deg = actual_deg;
        last_actual_deg = actual_deg;
    }

    defmt::info!(
        "tilt-extremes {}: reached MAX_PROBE_DEG cap (cmd={=f32}, actual={=f32}) without plateau",
        label,
        cmd_deg,
        last_actual_deg,
    );
    last_actual_deg
}

/// Issue a `set_pose` and surface the driver error directly.
async fn command(
    driver: &mut board::HeadDriverImpl,
    pose: Pose,
) -> Result<(), <board::HeadDriverImpl as HeadDriver>::Error> {
    driver.set_pose(pose, HalClock.now()).await
}

/// Read the live pitch encoder *and* the servo's status error byte,
/// returning `None` (with a logged warn) on transport / timeout
/// failure so the sweep can keep going. The error byte is logged
/// inline (with decoded flag names) any time it's non-zero — that's
/// the canonical "why isn't this servo moving" signal.
async fn read_pitch(driver: &mut board::HeadDriverImpl) -> Option<u16> {
    let bus = driver.bus_mut();
    let mut buf = [0u8; 2];
    match embassy_time::with_timeout(
        Duration::from_millis(READ_TIMEOUT_MS),
        bus.read_memory(head::PITCH_SERVO_ID, ADDR_PRESENT_POSITION, &mut buf),
    )
    .await
    {
        Ok(Ok(err_byte)) => {
            if err_byte != 0 {
                defmt::warn!(
                    "tilt-extremes: pitch FAULT byte=0x{=u8:02x} ({=str})",
                    err_byte,
                    decode_fault_byte(err_byte),
                );
            }
            Some(u16::from_be_bytes(buf))
        }
        Ok(Err(e)) => {
            defmt::warn!(
                "tilt-extremes: read_memory err: {}",
                defmt::Debug2Format(&e)
            );
            None
        }
        Err(_) => {
            defmt::warn!("tilt-extremes: read_memory timed out");
            None
        }
    }
}

/// Decode the SCSCL status-packet error byte into a comma-separated
/// flag name list. Bit definitions per Feetech SCSCL memory-table
/// documentation. Returns `"OK"` for a zero byte (caller normally
/// checks before calling, but harmless either way).
fn decode_fault_byte(b: u8) -> &'static str {
    // Table-driven would be cleaner with `alloc`, but we want a
    // `&'static str` for defmt's `{=str}` formatter, so enumerate
    // the common single- and double-flag combos. Anything past
    // a few simultaneous flags is exotic enough to warrant looking
    // at the raw hex.
    match b {
        0x00 => "OK",
        0x01 => "VOLTAGE",
        0x02 => "ANGLE",
        0x04 => "OVERHEAT",
        0x08 => "RANGE",
        0x10 => "CHECKSUM",
        0x20 => "OVERLOAD",
        0x40 => "INSTRUCTION",
        0x21 => "OVERLOAD+VOLTAGE",
        0x24 => "OVERLOAD+OVERHEAT",
        0x25 => "OVERLOAD+OVERHEAT+VOLTAGE",
        _ => "MULTIPLE (see hex)",
    }
}

/// One-shot servo health snapshot: voltage, temperature, and a
/// position-read that surfaces the error byte. All failures are
/// logged at `warn` and ignored — this is diagnostic-only.
async fn log_health(driver: &mut board::HeadDriverImpl, id: u8) {
    let bus = driver.bus_mut();

    match embassy_time::with_timeout(Duration::from_millis(READ_TIMEOUT_MS), bus.read_voltage(id))
        .await
    {
        Ok(Ok(v)) => defmt::info!(
            "tilt-extremes HEALTH: SCServo[{=u8}] voltage = {=u8} (= {=f32} V; servos brown-out below ~5.5 V)",
            id,
            v,
            f32::from(v) / 10.0,
        ),
        Ok(Err(e)) => defmt::warn!(
            "tilt-extremes HEALTH: voltage read err: {}",
            defmt::Debug2Format(&e)
        ),
        Err(_) => defmt::warn!("tilt-extremes HEALTH: voltage read timed out"),
    }

    match embassy_time::with_timeout(
        Duration::from_millis(READ_TIMEOUT_MS),
        bus.read_temperature(id),
    )
    .await
    {
        Ok(Ok(t)) => defmt::info!(
            "tilt-extremes HEALTH: SCServo[{=u8}] temperature = {=u8} °C (overheat trip is ~70 °C)",
            id,
            t,
        ),
        Ok(Err(e)) => defmt::warn!(
            "tilt-extremes HEALTH: temperature read err: {}",
            defmt::Debug2Format(&e)
        ),
        Err(_) => defmt::warn!("tilt-extremes HEALTH: temperature read timed out"),
    }

    let mut buf = [0u8; 2];
    match embassy_time::with_timeout(
        Duration::from_millis(READ_TIMEOUT_MS),
        bus.read_memory(id, ADDR_PRESENT_POSITION, &mut buf),
    )
    .await
    {
        Ok(Ok(err_byte)) => {
            let raw = u16::from_be_bytes(buf);
            defmt::info!(
                "tilt-extremes HEALTH: SCServo[{=u8}] position raw={=u16} ({=f32}°), error byte = 0x{=u8:02x} ({=str})",
                id,
                raw,
                raw_to_deg(raw),
                err_byte,
                decode_fault_byte(err_byte),
            );
        }
        Ok(Err(e)) => defmt::warn!(
            "tilt-extremes HEALTH: status read err: {}",
            defmt::Debug2Format(&e)
        ),
        Err(_) => defmt::warn!("tilt-extremes HEALTH: status read timed out"),
    }
}

/// Convert a raw step count into degrees in the firmware's commanded
/// reference frame: applies `head::TILT_TRIM_DEG` so `actual_deg`
/// directly compares against `cmd_deg` (delta = 0 when the servo
/// tracks). Mirrors `head::ScsHead::deg_for` with `direction = +1.0`.
fn raw_to_deg(raw: u16) -> f32 {
    let untrimmed = (f32::from(raw) - f32::from(POSITION_CENTER)) / POSITION_PER_DEGREE;
    untrimmed - head::TILT_TRIM_DEG
}

/// Inverse of `raw_to_deg` — also applies `head::TILT_TRIM_DEG` so
/// the operator-facing suggested EEPROM angle-limit raw counts match
/// what `head::ScsHead::position_for` would actually command.
fn deg_to_raw(deg: f32) -> u16 {
    let raw = f32::from(POSITION_CENTER) + (deg + head::TILT_TRIM_DEG) * POSITION_PER_DEGREE;
    let clamped = raw.clamp(0.0, 1023.0);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to [0, 1023] above"
    )]
    let pos = clamped as u16;
    pos
}

/// Threshold below which the sweep is treated as "no real motion" —
/// just encoder quantisation noise (≈0.3° per count, so ±1 count
/// of jitter is already ~0.6° span). Anything tighter than 2° means
/// the servo never actually moved between the up- and down-extremes
/// and the SUGGEST line would be nonsense.
const MIN_SPAN_FOR_SUGGEST_DEG: f32 = 2.0;

/// Emit one `SUMMARY` line + a `SUGGEST` line with EEPROM angle-limit
/// raw counts and a `TILT_TRIM_DEG` value to paste into `head.rs`.
/// If the observed span is too small for the SUGGEST math to be
/// meaningful (servo stuck), prints a STUCK verdict instead so the
/// operator doesn't paste garbage trim values into the firmware.
fn print_summary(lower_deg: f32, upper_deg: f32) {
    let span_deg = upper_deg - lower_deg;
    let center_deg = (upper_deg + lower_deg) / 2.0;

    defmt::info!(
        "tilt-extremes SUMMARY: actual range = {=f32}° .. {=f32}° ({=f32}° span); midpoint = {=f32}°",
        lower_deg,
        upper_deg,
        span_deg,
        center_deg,
    );

    if span_deg.abs() < MIN_SPAN_FOR_SUGGEST_DEG {
        defmt::warn!(
            "tilt-extremes STUCK: encoder span {=f32}° is below the {=f32}° noise floor — the pitch servo is not responding to commanded motion. No calibration values can fix this; check HEALTH log above for fault flags, then inspect linkage / replace servo.",
            span_deg.abs(),
            MIN_SPAN_FOR_SUGGEST_DEG,
        );
        return;
    }

    // To make `cmd_deg = 0` produce the mechanical centre, set
    //   trim = (center_deg - 0) → firmware adds it before scaling.
    // Inversion sign matches `head::ScsHead::position_for`.
    let suggested_trim_deg = center_deg;
    let suggested_min_raw = deg_to_raw(lower_deg);
    let suggested_max_raw = deg_to_raw(upper_deg);

    defmt::info!(
        "tilt-extremes SUGGEST: TILT_TRIM_DEG = {=f32}; EEPROM angle-limit raw counts MIN={=u16} MAX={=u16} (compare to boot-log MIN/MAX to tell EEPROM-clip from mech-stop)",
        suggested_trim_deg,
        suggested_min_raw,
        suggested_max_raw,
    );
}
