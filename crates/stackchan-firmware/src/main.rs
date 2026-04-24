//! StackChan firmware for the M5Stack CoreS3.
//!
//! Boot sequence: esp-hal init → internal SRAM + PSRAM heaps registered
//! with `esp_alloc` → esp-rtos embassy → AXP2101 LDOs → AW9523 drives the
//! board-level enables and pulses LCD reset → SPI2 + ILI9342C via mipidsi.
//! Main then spawns a ~30 FPS
//! embassy task that runs the full modifier stack
//! (`EmotionCycle` → `EmotionStyle` → `Blink` → `Breath` → `IdleDrift`)
//! against an `Avatar`, draws into a PSRAM-backed framebuffer, and
//! blits the whole frame to the LCD in one `fill_contiguous` call.
//! Main drops into a heartbeat loop so "render task alive" and "main
//! alive" show up as separate signals in the defmt log.

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

use stackchan_firmware::{
    ambient, audio, board, button, clock, framebuffer, head, imu, ir, leds, mag, touch, wallclock,
};

use board::{HeadDriverImpl, SharedI2c};
use clock::HalClock;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Ticker, Timer};
use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::{Point as EgPoint, Size},
    primitives::Rectangle,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::{
    Blocking,
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig},
    spi::{
        Mode as SpiMode,
        master::{Config as SpiConfig, Spi},
    },
    time::Rate,
    timer::timg::TimerGroup,
};
use framebuffer::{Framebuffer, HEIGHT as FB_HEIGHT, WIDTH as FB_WIDTH};
use mipidsi::{
    Builder, NoResetPin,
    interface::SpiInterface,
    models::ILI9342CRgb565,
    options::{ColorInversion, ColorOrder},
};
use stackchan_core::{
    Avatar, Clock, HeadDriver, LedFrame, Modifier,
    modifiers::{
        AmbientSleepy, Blink, Breath, EmotionCycle, EmotionHead, EmotionStyle, EmotionTouch,
        IdleDrift, IdleSway, PickupReaction, RemoteCommand,
    },
    render_leds,
};
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

/// LTO anchor for `ESP_APP_DESC`.
///
/// The macro above emits `pub static ESP_APP_DESC` without `#[used]`, so
/// `lto = "fat"` strips it and espflash refuses the image. Anchoring a
/// `&'static` reference in a `#[used]` static keeps the symbol live in
/// `.rodata_desc.appdesc` without any raw pointer or `unsafe impl Sync`
/// (`EspAppDesc` is plain POD, so the reference is auto-Sync).
#[used]
static _APP_DESC_ANCHOR: &esp_bootloader_esp_idf::EspAppDesc = &ESP_APP_DESC;

/// Panic handler. Halts the core; esp-rtos emits the trace over RTT
/// before we arrive here (via `--catch-hardfault` on the probe-rs side).
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("panic: {}", defmt::Display2Format(info));
    loop {}
}

/// Internal SRAM heap, registered first so short-lived allocations (embassy
/// task arena, defmt buffers) stay in fast on-chip RAM. The 150 KiB
/// framebuffer spills to the PSRAM region registered below. 72 KiB matches
/// the esp-generate default and leaves ample margin for `esp-rtos` state.
const HEAP_SIZE: usize = 72 * 1024;

/// Render cadence. 33 ms ≈ 30 FPS; `Ticker` corrects drift automatically
/// if a single frame's `Avatar::draw` runs long (e.g. during a blink
/// transition where every pixel changes).
const FRAME_PERIOD_MS: u64 = 33;

/// Head-update cadence. 20 ms = 50 Hz, matching the `SCServo`
/// recommended command rate. Running faster buys nothing — the servo's
/// internal interpolation smooths between commands — and keeps the
/// UART bus utilisation below 2% even with two servos per tick.
const HEAD_PERIOD_MS: u64 = 20;

/// Concrete type of the assembled LCD display. Spelled out once here so the
/// render task (which the `#[embassy_executor::task]` macro requires to be
/// non-generic) can name it cleanly. The `'static` lifetimes flow from
/// esp-hal peripheral ownership and the `StaticCell`-parked SPI buffer.
type LcdDisplay = mipidsi::Display<
    SpiInterface<
        'static,
        ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, Delay>,
        Output<'static>,
    >,
    ILI9342CRgb565,
    NoResetPin,
>;

/// 30 FPS render task. Double-buffered via a PSRAM-backed [`Framebuffer`]:
/// each frame we run the modifier stack, draw the avatar into the off-screen
/// buffer (no LCD traffic), then blit the whole buffer to the LCD in one
/// `fill_contiguous` call — which mipidsi lowers to a single
/// `CASET`/`RASET`/`RAMWR` + bulk SPI write. The LCD only ever sees a
/// complete frame, so the white-clear flicker from direct-draw is gone.
///
/// Modifier order is the canonical stackchan-core stack:
/// `EmotionTouch` → `EmotionCycle` → `EmotionStyle` → `Blink` →
/// `Breath` → `IdleDrift` → `IdleSway` → `EmotionHead`. `EmotionTouch`
/// runs first so a tap queued from the touch task becomes the active
/// emotion before `EmotionCycle` checks the `manual_until` gate.
/// `IdleSway` writes the base `avatar.head_pose` (slow wander);
/// `EmotionHead` adds an emotion-keyed bias on top (layered compose).
/// The final pose is published to the 50 Hz head task via
/// [`head::POSE_SIGNAL`]. `frame_eq` short-circuits blits when no
/// pixel-affecting modifier changed anything — pose updates alone never
/// trigger a redundant LCD blit because `head_pose` is excluded from
/// `frame_eq`.
#[embassy_executor::task]
async fn render_task(mut display: LcdDisplay) {
    let clock = HalClock;
    let mut fb = Framebuffer::new();
    defmt::info!(
        "framebuffer allocated in PSRAM: {=u32}x{=u32} Rgb565",
        FB_WIDTH,
        FB_HEIGHT
    );
    let mut avatar = Avatar::default();
    let mut emotion_touch = EmotionTouch::new();
    // Empty remote mapping by default. Populate with your specific
    // NEC `(address, command, emotion)` tuples after running
    // `examples/ir_bench.rs` to discover your remote's codes.
    let mut remote = RemoteCommand::new();
    let mut pickup = PickupReaction::new();
    let mut ambient_sleepy = AmbientSleepy::new();
    let mut cycle = EmotionCycle::new();
    let mut style = EmotionStyle::new();
    let mut blink = Blink::new();
    let mut breath = Breath::new();
    // Fixed seed keeps boot-to-boot drifts identical; a future RNG-backed
    // source (e.g. reading a voltage-derived seed from the AXP2101) can
    // swap in without touching the task shape.
    let mut drift =
        IdleDrift::with_seed(const { core::num::NonZeroU32::new(0xDEAD_BEEF).unwrap() });
    let mut sway = IdleSway::new();
    let mut emotion_head = EmotionHead::new();
    let mut last_rendered: Option<Avatar> = None;
    let mut led_frame = LedFrame::default();

    // Pre-compute the blit rect once; it never changes.
    let canvas = Rectangle::new(EgPoint::zero(), Size::new(FB_WIDTH, FB_HEIGHT));

    let mut ticker = Ticker::every(Duration::from_millis(FRAME_PERIOD_MS));
    defmt::info!(
        "render task: {=u64} ms tick, EmotionTouch + RemoteCommand + PickupReaction + AmbientSleepy + EmotionCycle + EmotionStyle + Blink + Breath + IdleDrift + IdleSway + EmotionHead",
        FRAME_PERIOD_MS
    );

    loop {
        let now = clock.now();

        // Drain any tap edges the touch task or power-button task
        // published since last frame. Both sources publish to the
        // same signal so a button press is UX-indistinguishable from
        // a screen tap. `try_take` is non-blocking; a missing signal
        // is the common case and means `EmotionTouch::update` only
        // does the expired-hold cleanup work this tick.
        if touch::TAP_SIGNAL.try_take().is_some() {
            emotion_touch.tap();
        }
        // Drain IR-remote decoded commands, if any.
        if let Some(cmd) = ir::REMOTE_SIGNAL.try_take() {
            remote.queue(cmd.address, cmd.command);
        }
        // Drain the latest IMU reading. Published at ~100 Hz by the
        // imu task; at a 33 ms render tick we'll usually have a fresh
        // sample available, but if we don't, last-known values stay
        // on `avatar` so `PickupReaction` keeps a coherent view.
        if let Some(m) = imu::IMU_SIGNAL.try_take() {
            avatar.accel_g = m.accel_g;
            avatar.gyro_dps = m.gyro_dps;
        }
        // Drain the latest ambient reading. 2 Hz publish rate, so most
        // render ticks see nothing new — last-known lux stays on the
        // avatar so `AmbientSleepy` has coherent input.
        if let Some(lux) = ambient::AMBIENT_LUX_SIGNAL.try_take() {
            avatar.ambient_lux = Some(lux);
        }
        // Drain the latest magnetometer reading. 10 Hz publish rate;
        // no modifier consumes it yet (data-only landing), but the
        // value propagates so future compass / heading modifiers can
        // pick it up without touching the render task.
        if let Some(ut) = mag::MAG_SIGNAL.try_take() {
            avatar.mag_ut = Some(ut);
        }
        emotion_touch.update(&mut avatar, now);
        remote.update(&mut avatar, now);
        pickup.update(&mut avatar, now);
        ambient_sleepy.update(&mut avatar, now);
        cycle.update(&mut avatar, now);
        style.update(&mut avatar, now);
        blink.update(&mut avatar, now);
        breath.update(&mut avatar, now);
        drift.update(&mut avatar, now);
        sway.update(&mut avatar, now);
        emotion_head.update(&mut avatar, now);

        // Publish the final pose (sway + emotion bias) to the head task.
        // `Signal::signal` overwrites any un-consumed value, so a slower
        // head task never builds up a backlog — it just reads the most
        // recent pose.
        head::POSE_SIGNAL.signal(avatar.head_pose);

        // Render the LED ring from the same avatar state and publish to
        // the led task. The led task owns the PY32 transport; this
        // keeps I²C latency off the render path. Always publish —
        // `Signal::signal` overwrites unread values so the led task
        // can tick at its own cadence without building up a backlog.
        render_leds(&avatar, now, &mut led_frame);
        leds::LED_FRAME_SIGNAL.signal(led_frame);

        // Drain the latest observed pose from the head task. Updated at
        // ~1 Hz over there, so most render ticks see nothing new and
        // hold the previous value — which is exactly what we want.
        if let Some(actual) = head::HEAD_POSE_ACTUAL_SIGNAL.try_take() {
            avatar.head_pose_actual = actual;
        }

        // `frame_eq` intentionally skips `head_pose` — the LCD is mounted
        // rigidly to the head, so pan/tilt updates never change pixels.
        // `IdleSway` mutates `avatar.head_pose` every tick; using `==`
        // here would force a full-frame SPI blit on every sway step.
        if last_rendered
            .as_ref()
            .is_none_or(|prev| !prev.frame_eq(&avatar))
        {
            // Draw is Infallible on `Framebuffer`; the `let _ =` discards
            // the `Result<(), Infallible>` without triggering unwrap lints.
            let _ = avatar.draw(&mut fb);
            match display.fill_contiguous(&canvas, fb.as_slice().iter().copied()) {
                Ok(()) => last_rendered = Some(avatar),
                Err(e) => defmt::error!("render: blit failed: {}", defmt::Debug2Format(&e)),
            }
        }

        ticker.next().await;
    }
}

/// 50 Hz head-update task. Consumes [`head::POSE_SIGNAL`] and commands
/// the `SCServo` bus. Holds the last-seen pose between updates — servos
/// hold their position via internal torque, so a slow render task
/// never leaves the head wobbling.
///
/// Every `POSITION_POLL_EVERY` ticks (= 1 Hz at 50 Hz command cadence),
/// the task also reads back each servo's live position and publishes it
/// via [`head::HEAD_POSE_ACTUAL_SIGNAL`] for the render task to write
/// onto `avatar.head_pose_actual` + logs a `cmd vs actual` line.
///
/// UART write/read failures log at `warn` and continue: a transient
/// bus glitch shouldn't blank the face or reboot the binary.
#[embassy_executor::task]
async fn head_task(mut driver: HeadDriverImpl) {
    /// Poll position every N command ticks. `HEAD_PERIOD_MS` × N = 1 s.
    const POSITION_POLL_EVERY: u32 = 50;
    /// Per-read timeout for the `read_position` calls.
    const READ_TIMEOUT_MS: u64 = 10;

    let clock = HalClock;
    let mut ticker = Ticker::every(Duration::from_millis(HEAD_PERIOD_MS));
    let mut current = stackchan_core::Pose::NEUTRAL;
    let mut tick_count: u32 = 0;
    defmt::info!(
        "head task: {=u64} ms tick, consumes POSE_SIGNAL for SCServo IDs {=u8} (yaw) / {=u8} (pitch), reads actual @ {=u64} ms",
        HEAD_PERIOD_MS,
        head::YAW_SERVO_ID,
        head::PITCH_SERVO_ID,
        HEAD_PERIOD_MS.saturating_mul(u64::from(POSITION_POLL_EVERY)),
    );
    loop {
        if let Some(next) = head::POSE_SIGNAL.try_take() {
            current = next;
        }
        if let Err(e) = driver.set_pose(current, clock.now()).await {
            defmt::warn!("head: SCServo write failed: {}", defmt::Debug2Format(&e));
        }
        tick_count = tick_count.wrapping_add(1);
        if tick_count.is_multiple_of(POSITION_POLL_EVERY) {
            match embassy_time::with_timeout(
                Duration::from_millis(READ_TIMEOUT_MS),
                driver.read_pose(),
            )
            .await
            {
                Ok(Ok(actual)) => {
                    head::HEAD_POSE_ACTUAL_SIGNAL.signal(actual);
                    defmt::info!(
                        "head: cmd=({=f32}, {=f32}) actual=({=f32}, {=f32})",
                        current.pan_deg,
                        current.tilt_deg,
                        actual.pan_deg,
                        actual.tilt_deg,
                    );
                }
                Ok(Err(e)) => defmt::warn!(
                    "head: read_pose response error: {}",
                    defmt::Debug2Format(&e)
                ),
                Err(_) => {
                    defmt::warn!("head: read_pose timed out after {=u64} ms", READ_TIMEOUT_MS);
                }
            }
        }
        ticker.next().await;
    }
}

/// FT6336U polling task. Wraps the shared I²C bus in a [`Ft6336u`]
/// driver and delegates to [`touch::run_touch_loop`], which reads the
/// vendor ID once at startup and then publishes rising-edge taps on
/// [`touch::TAP_SIGNAL`]. The render task drains the signal and feeds
/// it into the [`EmotionTouch`] modifier.
#[embassy_executor::task]
async fn touch_task(shared_i2c: SharedI2c) -> ! {
    let touch = ft6336u::Ft6336u::new(shared_i2c);
    touch::run_touch_loop(touch).await
}

/// BMI270 IMU polling task. Wraps a second shared-bus handle onto the
/// same internal I²C0 (the `embassy-embedded-hal` `I2cDevice` mutex
/// serializes accesses automatically), runs the init sequence, and
/// publishes samples on [`imu::IMU_SIGNAL`].
#[embassy_executor::task]
async fn imu_task(shared_i2c: SharedI2c) -> ! {
    imu::run_imu_loop(shared_i2c).await
}

/// LTR-553 ambient-light polling task. Third consumer of the shared
/// I²C0 bus. Publishes lux estimates on [`ambient::AMBIENT_LUX_SIGNAL`].
#[embassy_executor::task]
async fn ambient_task(shared_i2c: SharedI2c) -> ! {
    ambient::run_ambient_loop(shared_i2c).await
}

/// AXP2101 power-button polling task. Fourth shared-I²C consumer.
/// Forwards short-press edges to [`touch::TAP_SIGNAL`] so the power
/// button behaves as a second tap source (cycle emotion + 30 s pin).
#[embassy_executor::task]
async fn button_task(shared_i2c: SharedI2c) -> ! {
    button::run_button_loop(shared_i2c).await
}

/// IR RX task on the RMT peripheral. Decodes NEC-protocol frames
/// from the IR receiver and publishes them on [`ir::REMOTE_SIGNAL`].
#[embassy_executor::task]
async fn ir_task(
    rmt: esp_hal::peripherals::RMT<'static>,
    pin: esp_hal::peripherals::GPIO21<'static>,
) -> ! {
    ir::run_ir_loop(rmt, pin).await
}

/// LED-ring output-sink task. Drains [`leds::LED_FRAME_SIGNAL`] at
/// 30 Hz and pushes each frame to the PY32 IO expander over the shared
/// I²C0 bus. Runs a brief fade-in before joining the signal pipeline.
#[embassy_executor::task]
async fn led_task(shared_i2c: SharedI2c) -> ! {
    leds::run_led_loop(shared_i2c).await
}

/// BMM150 magnetometer polling task. Fifth (now sixth with LEDs) shared-I²C
/// consumer. Publishes compensated µT tuples on [`mag::MAG_SIGNAL`].
#[embassy_executor::task]
async fn mag_task(shared_i2c: SharedI2c) -> ! {
    mag::run_mag_loop(shared_i2c).await
}

/// Audio task. Today a park loop; in PR 2B this becomes the I²S DMA
/// read loop that computes mic RMS per render window and publishes to
/// [`audio::AUDIO_RMS_SIGNAL`].
#[embassy_executor::task]
async fn audio_task() -> ! {
    audio::run_audio_loop().await
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // Two heap regions registered with the single `esp_alloc::HEAP`:
    //   1. Internal SRAM (reclaimed post-init) — fast, small, first-preference
    //      for the embassy task arena, defmt buffers, and short-lived allocs.
    //   2. External PSRAM — 8 MiB of slower memory the 150 KiB framebuffer
    //      lives in. Registered second so small allocs don't waste PSRAM.
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: HEAP_SIZE);
    esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    defmt::info!(
        "stackchan-firmware v{} — CoreS3 boot",
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

    // CoreS3 LCD (ILI9342C) on SPI2.
    //   SCK  = GPIO36
    //   MOSI = GPIO37
    //   CS   = GPIO3  (active low)
    //   DC   = GPIO35 (0 = command, 1 = data)
    //   RST  = AW9523 P1_1 (handled above — mipidsi sees `NoResetPin`)
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
    #[allow(clippy::items_after_statements)]
    static SPI_DI_BUF: StaticCell<[u8; 4096]> = StaticCell::new();
    let spi_di_buf = SPI_DI_BUF.init([0u8; 4096]);
    let di = SpiInterface::new(spi_device, dc, spi_di_buf);

    // `NoResetPin` is implied by omitting `.reset_pin(...)` — the hardware
    // reset is already done via AW9523 above. BGR color order matches the
    // CoreS3 panel wiring; without `invert_colors` the image appears as a
    // color-inverted negative on this specific module.
    let display: LcdDisplay = match Builder::new(ILI9342CRgb565, di)
        .display_size(320, 240)
        .color_order(ColorOrder::Bgr)
        .invert_colors(ColorInversion::Inverted)
        .init(&mut Delay)
    {
        Ok(d) => d,
        Err(e) => defmt::panic!("mipidsi init failed: {}", defmt::Debug2Format(&e)),
    };
    defmt::info!("ILI9342C ready — spawning render task");

    if let Err(e) = spawner.spawn(render_task(display)) {
        defmt::panic!("spawn render_task failed: {}", defmt::Debug2Format(&e));
    }
    if let Err(e) = spawner.spawn(head_task(board_io.head)) {
        defmt::panic!("spawn head_task failed: {}", defmt::Debug2Format(&e));
    }
    if let Err(e) = spawner.spawn(touch_task(board_io.i2c)) {
        defmt::panic!("spawn touch_task failed: {}", defmt::Debug2Format(&e));
    }
    // Three more shared-bus handles onto the same I²C0. The
    // `I2cDevice` wrapper serialises concurrent access so each task
    // can own a handle without contention bookkeeping.
    if let Err(e) = spawner.spawn(imu_task(I2cDevice::new(board_io.i2c_bus))) {
        defmt::panic!("spawn imu_task failed: {}", defmt::Debug2Format(&e));
    }
    if let Err(e) = spawner.spawn(ambient_task(I2cDevice::new(board_io.i2c_bus))) {
        defmt::panic!("spawn ambient_task failed: {}", defmt::Debug2Format(&e));
    }
    if let Err(e) = spawner.spawn(button_task(I2cDevice::new(board_io.i2c_bus))) {
        defmt::panic!("spawn button_task failed: {}", defmt::Debug2Format(&e));
    }
    if let Err(e) = spawner.spawn(ir_task(peripherals.RMT, peripherals.GPIO21)) {
        defmt::panic!("spawn ir_task failed: {}", defmt::Debug2Format(&e));
    }
    if let Err(e) = spawner.spawn(led_task(I2cDevice::new(board_io.i2c_bus))) {
        defmt::panic!("spawn led_task failed: {}", defmt::Debug2Format(&e));
    }
    if let Err(e) = spawner.spawn(mag_task(I2cDevice::new(board_io.i2c_bus))) {
        defmt::panic!("spawn mag_task failed: {}", defmt::Debug2Format(&e));
    }

    // Audio bring-up: configure AW88298 + ES7210 over shared I²C. Does
    // not start I²S streaming (that's PR 2B). Failures are warn-only —
    // audio degrades to "silent mic, muted speaker" rather than
    // halting the boot, since the rest of the avatar still works.
    let amp_bus = I2cDevice::new(board_io.i2c_bus);
    let adc_bus = I2cDevice::new(board_io.i2c_bus);
    match audio::bringup(amp_bus, adc_bus).await {
        Ok(()) => defmt::info!("audio: codecs up (I²S streaming pending PR 2B)"),
        Err(e) => defmt::warn!("audio: bring-up failed ({:?}) — audio disabled", e),
    }
    if let Err(e) = spawner.spawn(audio_task()) {
        defmt::panic!("spawn audio_task failed: {}", defmt::Debug2Format(&e));
    }

    // One-shot wall-clock read for the boot log. Single I²C round-trip;
    // failures are warn-only (logged inside `wallclock::read_and_format`).
    let mut rtc_buf = [0u8; 19];
    let rtc_bus = I2cDevice::new(board_io.i2c_bus);
    if let Some(stamp) = wallclock::read_and_format(rtc_bus, &mut rtc_buf).await {
        defmt::info!("boot @ {=str} (RTC)", stamp);
    }

    defmt::info!("boot complete — idle heartbeat");
    loop {
        Timer::after(Duration::from_secs(5)).await;
        defmt::debug!("heartbeat");
    }
}
