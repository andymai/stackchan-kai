//! StackChan firmware for the M5Stack CoreS3.
//!
//! Boot sequence: esp-hal init → internal SRAM + PSRAM heaps registered
//! with `esp_alloc` → esp-rtos embassy → AXP2101 LDOs → AW9523 drives the
//! board-level enables and pulses LCD reset → SPI2 + ILI9342C via mipidsi.
//! Main then spawns a ~30 FPS
//! embassy task that runs the full modifier stack
//! (`EmotionCycle` → `StyleFromEmotion` → `Blink` → `Breath` → `IdleDrift`)
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
    ambient, audio, board, body_touch, button, camera, clock, framebuffer, head, imu, ir, leds,
    net, power, storage, touch, tracking_trace, wallclock, watchdog,
};

use board::{HeadDriverImpl, SharedI2c};
use clock::HalClock;
use core::cell::RefCell;
use core::num::NonZeroU32;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Ticker, Timer};
use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::{Point as EgPoint, Size},
    pixelcolor::Rgb565,
    pixelcolor::raw::RawU16,
    primitives::Rectangle,
};
// `RefCellDevice` shares the SPI2 bus between the LCD and the SD card
// client. Single-core embassy + cooperative scheduling means the
// `RefCell` borrow held during a blocking SPI transaction can't be
// re-entered — task switches only happen at `.await` points, and
// SPI transactions are entirely synchronous.
use embedded_hal_bus::spi::RefCellDevice;
use esp_hal::{
    Blocking,
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig},
    rng::Rng,
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
    Clock, Director, Entity, Face, HeadDriver, LedFrame,
    modifiers::{
        AttentionFromTracking, Blink, Breath, DormancyFromActivity, EmotionCycle,
        EmotionFromAmbient, EmotionFromBattery, EmotionFromIntent, EmotionFromRemote,
        EmotionFromTouch, EmotionFromVoice, GazeFromAttention, HeadFromAttention, HeadFromEmotion,
        HeadFromIntent, IdleDrift, IdleHeadDrift, IntentFromBodyTouch, IntentFromLoud,
        MicrosaccadeFromAttention, MouthFromAudio, RemoteCommandModifier, StyleFromEmotion,
        StyleFromIntent,
    },
    render_leds,
    skills::{Handling, Listening, Petting},
    voice::{ChirpKind, PhraseId, Utterance},
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
/// if a single frame's `Face::draw` runs long (e.g. during a blink
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
        RefCellDevice<'static, Spi<'static, Blocking>, Output<'static>, Delay>,
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
/// `EmotionFromTouch` → `EmotionCycle` → `StyleFromEmotion` → `Blink` →
/// `Breath` → `IdleDrift` → `IdleHeadDrift` → `HeadFromEmotion` →
/// `HeadFromAttention`. `EmotionFromTouch` runs first so a tap queued from the
/// touch task becomes the active emotion before `EmotionCycle` checks
/// the `manual_until` gate. `IdleHeadDrift` writes the base
/// `entity.motor.head_pose` (slow wander); `HeadFromEmotion` adds an
/// emotion-keyed bias on top; `HeadFromAttention` adds an upward listening
/// tilt when the `Listening` skill (registered separately) sets
/// `mind.attention = Listening`.
/// The final pose is published to the 50 Hz head task via
/// [`head::POSE_SIGNAL`]. `frame_eq` short-circuits blits when no
/// pixel-affecting modifier changed anything — pose updates alone never
/// trigger a redundant LCD blit because `head_pose` is excluded from
/// `frame_eq`.
#[embassy_executor::task]
#[allow(
    clippy::too_many_lines,
    reason = "tick body composes 12+ modifiers + sensor drains + render — splitting fragments the per-frame ordering invariants"
)]
async fn render_task(mut display: LcdDisplay, drift_seed: NonZeroU32, head_drift_seed: NonZeroU32) {
    let clock = HalClock;
    let mut fb = Framebuffer::new();
    defmt::debug!(
        "framebuffer allocated in PSRAM: {=u32}x{=u32} Rgb565",
        FB_WIDTH,
        FB_HEIGHT
    );
    let mut entity = Entity::default();
    let mut emotion_from_touch = EmotionFromTouch::new();
    // Empty remote mapping by default. Populate with your specific
    // NEC `(address, command, emotion)` tuples after running
    // `examples/ir_bench.rs` to discover your remote's codes.
    let mut remote = EmotionFromRemote::new();
    let mut emotion_from_intent = EmotionFromIntent::new();
    let mut emotion_from_ambient = EmotionFromAmbient::new();
    let mut emotion_from_battery = EmotionFromBattery::new();
    let mut emotion_from_voice = EmotionFromVoice::new();
    let mut intent_from_loud = IntentFromLoud::new();
    let mut attention_from_tracking = AttentionFromTracking::new();
    let mut remote_command = RemoteCommandModifier::new();
    let mut dormancy_from_activity = DormancyFromActivity::new();
    let mut cycle = EmotionCycle::new();
    let mut style = StyleFromEmotion::new();
    let mut style_from_intent = StyleFromIntent::new();
    let mut gaze_from_attention = GazeFromAttention::new();
    let mut microsaccade = MicrosaccadeFromAttention::new();
    let mut blink = Blink::new();
    let mut breath = Breath::new();
    // Both idle modifiers seed from independent ESP32-S3 hardware RNG
    // samples (`esp_hal::rng::Rng`) taken once at boot in `main()`
    // before this task spawns. Each boot produces a distinct schedule,
    // and the two schedules aren't correlated within a unit — so the
    // eye drift and head glance don't fall into a shared rhythm.
    let mut drift = IdleDrift::with_seed(drift_seed);
    let mut head_drift = IdleHeadDrift::with_seed(head_drift_seed);
    let mut head_from_emotion = HeadFromEmotion::new();
    let mut head_from_attention = HeadFromAttention::new();
    let mut head_from_intent = HeadFromIntent::new();
    let mut mouth_from_audio = MouthFromAudio::new();
    let mut listening = Listening::new();
    let mut petting = Petting::new();
    let mut handling = Handling::new();
    let mut intent_from_body_touch = IntentFromBodyTouch::new();
    let mut last_rendered: Option<Face> = None;
    // Last instant TX was observed playing. Drives the post-playback
    // tail of the audio_rms gate so the mic doesn't pick up the
    // speaker's residual response immediately after a chirp ends.
    let mut last_tx_active_at: Option<stackchan_core::Instant> = None;
    // Camera-tracking observability. Compiles to a ZST + no-op methods
    // unless the `tracking-trace` cargo feature is on; see
    // [`tracking_trace`] for what the feature-on path emits.
    let mut tracking_trace = tracking_trace::TraceState::new();

    // Build the Director and register the canonical modifier stack
    // plus skills (via add_skill).
    // Phase ordering (Affect → Expression → Motion → Audio) is enforced
    // by `ModifierMeta::phase`; intra-phase ordering is registration
    // order modulated by per-modifier `priority`. See AGENTS.md and
    // `crates/stackchan-core/src/director.rs` for the rationale.
    let mut director = Director::new();
    director
        .add_modifier(&mut emotion_from_touch)
        .expect("registry full");
    director
        .add_modifier(&mut intent_from_body_touch)
        .expect("registry full");
    director.add_modifier(&mut remote).expect("registry full");
    director
        .add_modifier(&mut emotion_from_intent)
        .expect("registry full");
    director
        .add_modifier(&mut emotion_from_voice)
        .expect("registry full");
    director
        .add_modifier(&mut intent_from_loud)
        .expect("registry full");
    director
        .add_modifier(&mut emotion_from_ambient)
        .expect("registry full");
    director
        .add_modifier(&mut emotion_from_battery)
        .expect("registry full");
    director
        .add_modifier(&mut attention_from_tracking)
        .expect("registry full");
    director
        .add_modifier(&mut remote_command)
        .expect("registry full");
    director
        .add_modifier(&mut dormancy_from_activity)
        .expect("registry full");
    director.add_modifier(&mut cycle).expect("registry full");
    director.add_modifier(&mut style).expect("registry full");
    director
        .add_modifier(&mut style_from_intent)
        .expect("registry full");
    director
        .add_modifier(&mut gaze_from_attention)
        .expect("registry full");
    director
        .add_modifier(&mut microsaccade)
        .expect("registry full");
    director.add_modifier(&mut blink).expect("registry full");
    director.add_modifier(&mut breath).expect("registry full");
    director.add_modifier(&mut drift).expect("registry full");
    director
        .add_modifier(&mut head_drift)
        .expect("registry full");
    director
        .add_modifier(&mut head_from_emotion)
        .expect("registry full");
    director
        .add_modifier(&mut head_from_attention)
        .expect("registry full");
    director
        .add_modifier(&mut head_from_intent)
        .expect("registry full");
    director
        .add_modifier(&mut mouth_from_audio)
        .expect("registry full");
    director
        .add_skill(&mut listening)
        .expect("skill registry full");
    director
        .add_skill(&mut petting)
        .expect("skill registry full");
    director
        .add_skill(&mut handling)
        .expect("skill registry full");
    let mut led_frame = LedFrame::default();
    // Camera-mode state. The button task's long-press handler is the
    // sole producer of `CAMERA_MODE_SIGNAL`; we mirror it locally so
    // every render tick can branch quickly without re-checking the
    // signal. Starts `false` (avatar mode).
    let mut camera_mode: bool = false;
    // Most-recently completed camera frame as a `&'static [u8]`. The
    // camera task publishes one of these on `CAMERA_FRAME_SIGNAL`
    // after copying each ping-pong buffer into a scratch slot; we
    // hold the latest so we keep blitting it until a fresher frame
    // arrives. `None` until the first capture completes.
    let mut latest_camera_frame: Option<&'static [u8]> = None;

    // Pre-compute the blit rect once; it never changes.
    let canvas = Rectangle::new(EgPoint::zero(), Size::new(FB_WIDTH, FB_HEIGHT));

    let mut ticker = Ticker::every(Duration::from_millis(FRAME_PERIOD_MS));
    defmt::info!(
        "render task: {=u64} ms tick, EmotionFromTouch + IntentFromBodyTouch + EmotionFromRemote + EmotionFromIntent + EmotionFromVoice + IntentFromLoud + EmotionFromAmbient + EmotionFromBattery + AttentionFromTracking + DormancyFromActivity + EmotionCycle + StyleFromEmotion + StyleFromIntent + GazeFromAttention + MicrosaccadeFromAttention + Blink + Breath + IdleDrift + IdleHeadDrift + HeadFromEmotion + HeadFromAttention + HeadFromIntent + MouthFromAudio + Listening[skill] + Petting[skill] + Handling[skill]",
        FRAME_PERIOD_MS
    );

    loop {
        let now = clock.now();

        // Drain camera-mode toggle edges from the button task. Each
        // signal is a fresh "user wants this mode" assertion; we mirror
        // it locally + fire the appropriate audio chirp on transitions.
        // CAMERA_MODE_SIGNAL is now a display-only toggle: the camera
        // task always streams (for tracking); this flag only decides
        // whether the LCD shows the camera preview or the avatar.
        if let Some(active) = camera::CAMERA_MODE_SIGNAL.try_take()
            && active != camera_mode
        {
            camera_mode = active;
            let phrase = if active {
                PhraseId::CameraModeEnteredChirp
            } else {
                PhraseId::CameraModeExitedChirp
            };
            if let Err(e) = audio::try_dispatch_utterance(&Utterance::phrase(phrase)) {
                defmt::warn!("audio: camera-mode chirp dropped ({:?})", e);
            }
        }
        // Drain any newly completed camera frame slice. Latest-wins:
        // we hold the most recent slice ref until the camera task
        // publishes a fresher one.
        if let Some(frame) = camera::CAMERA_FRAME_SIGNAL.try_take() {
            latest_camera_frame = Some(frame);
        }

        // Drain any tap edges the touch task or power-button task
        // published since last frame. Both sources publish to the
        // same signal so a button press is UX-indistinguishable from
        // a screen tap. `try_take` is non-blocking; a missing signal
        // is the common case and means `EmotionFromTouch::update` only
        // does the expired-hold cleanup work this tick.
        if touch::TAP_SIGNAL.try_take().is_some() {
            if camera_mode {
                // In camera mode a tap means "capture this frame":
                // ask the camera task to log stats + a thumbnail
                // strip over RTT next frame, instead of cycling
                // emotion (which the user can't see anyway).
                camera::CAMERA_CAPTURE_REQUEST.signal(());
                defmt::debug!("render: tap in camera mode → capture request");
            } else {
                entity.input.tap_pending = true;
            }
        }
        // Drain IR-remote decoded commands, if any.
        if let Some(cmd) = ir::REMOTE_SIGNAL.try_take() {
            entity.input.remote_pending = Some((cmd.address, cmd.command));
        }
        // Drain HTTP control-plane commands, if any. RemoteCommandModifier
        // (Phase::Cognition) consumes the slot, asserts emotion or
        // attention, and re-asserts each tick until the hold expires.
        if let Some(cmd) = net::http::REMOTE_COMMAND_SIGNAL.try_take() {
            entity.input.remote_command = Some(cmd);
        }
        // Drain the latest IMU reading.
        if let Some(m) = imu::IMU_SIGNAL.try_take() {
            entity.perception.accel_g = m.accel_g;
            entity.perception.gyro_dps = m.gyro_dps;
        }
        // Drain the latest ambient reading.
        if let Some(lux) = ambient::AMBIENT_LUX_SIGNAL.try_take() {
            entity.perception.ambient_lux = Some(lux);
        }
        // Drain the latest power status from the AXP2101 task.
        // The `EmotionFromBattery` modifier owns the arming-edge logic
        // for the alert chirp now — it sets
        // `entity.voice.chirp_request = Some(ChirpKind::LowBatteryAlert)`
        // and the post-`run()` dispatch below enqueues it.
        if let Some(status) = power::POWER_STATUS_SIGNAL.try_take() {
            entity.perception.battery_percent = Some(status.battery_percent);
            entity.perception.usb_power_present = Some(status.usb_power);
        }
        // Drain the latest mic-RMS sample, gated by TX activity so the
        // speaker doesn't re-trigger sound-reactive modifiers on its
        // own chirps. While TX is playing — or for `TX_GATE_TAIL_MS`
        // afterwards to cover the speaker's mechanical decay —
        // `audio_rms` is held at `None` and any pending RMS signal is
        // drained so a stale window doesn't leak through post-gate.
        let tx_playing = audio::AUDIO_TX_PLAYING.load(core::sync::atomic::Ordering::Relaxed);
        if tx_playing {
            last_tx_active_at = Some(now);
        }
        let gate_active = tx_playing
            || last_tx_active_at
                .is_some_and(|t| now.saturating_duration_since(t) < audio::TX_GATE_TAIL_MS);
        if gate_active {
            entity.perception.audio_rms = None;
            let _ = audio::AUDIO_RMS_SIGNAL.try_take();
        } else if let Some(rms) = audio::AUDIO_RMS_SIGNAL.try_take() {
            entity.perception.audio_rms = Some(rms.0);
        }
        // TX-side lip-sync drain. While the audio task is playing
        // speech, MouthFromAudio reads `tx_lip_sync` in preference to
        // the (gated) mic. When playback ends, we observe the
        // AUDIO_TX_PLAYING transition and clear the field so the next
        // mic-driven path takes over.
        if tx_playing {
            if let Some(hint) = audio::TX_LIP_SYNC_SIGNAL.try_take() {
                entity.perception.tx_lip_sync = Some(hint);
            }
        } else {
            // Drop any in-flight signal value plus the perception
            // entry so MouthFromAudio falls back to the mic path.
            let _ = audio::TX_LIP_SYNC_SIGNAL.try_take();
            entity.perception.tx_lip_sync = None;
        }
        // Drain the latest body-touch (back-of-head Si12T) reading.
        if let Some(touch) = body_touch::BODY_TOUCH_SIGNAL.try_take() {
            entity.perception.body_touch = Some(touch);
        }
        // Drain the latest camera tracker observation. Cognition
        // modifiers read entity.perception.tracking to decide whether
        // to flip mind.attention to Tracking{target}.
        if let Some(observation) = camera::CAMERA_TRACKING_SIGNAL.try_take() {
            tracking_trace.note_observation(now);
            entity.perception.tracking = Some(observation);
        }

        // Run the entire modifier graph in one call. The Director sorts
        // by (phase, priority, registration order) and ticks each
        // modifier — see `crate::director` for the full pipeline.
        director.run(&mut entity, now);
        tracking_trace.observe(&entity, now);

        // Drain speech intents raised this tick. Two parallel paths
        // during the chirp → utterance migration:
        //
        // - `entity.voice.chirp_request` (legacy): translated to a
        //   non-verbal `PhraseId` and dispatched through the same
        //   speech path. Modifiers that still publish ChirpKind keep
        //   working without code changes; once they migrate to
        //   `utterance_request`, this arm goes away with `ChirpKind`.
        // - `entity.voice.utterance_request` (current): structured
        //   speech intent. Routed straight through.
        if let Some(kind) = entity.voice.chirp_request.take() {
            // ChirpKind is `non_exhaustive`; the catch-all `_` arm
            // is required syntactically and there's no stable way
            // to force every known variant to appear above it.
            // Adding a `ChirpKind` variant in core compiles silently
            // here — but `ChirpKind` is being retired alongside the
            // `voice.chirp_request` field, so the surface is shrinking
            // (no new variants expected).
            let phrase = match kind {
                ChirpKind::Pickup => Some(PhraseId::PickupChirp),
                ChirpKind::Wake => Some(PhraseId::WakeChirp),
                ChirpKind::Startle => Some(PhraseId::StartleChirp),
                ChirpKind::LowBatteryAlert => Some(PhraseId::LowBatteryChirp),
                ChirpKind::CameraModeEntered => Some(PhraseId::CameraModeEnteredChirp),
                ChirpKind::CameraModeExited => Some(PhraseId::CameraModeExitedChirp),
                _ => None,
            };
            if let Some(phrase) = phrase {
                let utterance = Utterance::phrase(phrase);
                if let Err(e) = audio::try_dispatch_utterance(&utterance) {
                    defmt::warn!(
                        "audio: chirp {:?} dropped ({:?})",
                        defmt::Debug2Format(&kind),
                        e,
                    );
                }
            }
        }
        if let Some(utterance) = entity.voice.utterance_request.take()
            && let Err(e) = audio::try_dispatch_utterance(&utterance)
        {
            defmt::warn!(
                "audio: utterance {:?} dropped ({:?})",
                defmt::Debug2Format(&utterance),
                e,
            );
        }

        // Publish the final pose to the head task.
        head::POSE_SIGNAL.signal(entity.motor.head_pose);

        // Update the read-only avatar snapshot the HTTP `/state`
        // handler reads. Per-frame is cheap (one mutex, one struct
        // copy) and avoids a second consumer racing the existing
        // latest-wins `Signal`s.
        net::snapshot::update_avatar(
            entity.mind.affect.emotion,
            entity.motor.head_pose,
            Some(entity.motor.head_pose_actual),
        );
        // Push the latest snapshot to any connected SSE subscribers.
        // Throttled internally so a 30 Hz render doesn't firehose
        // GET /state/stream clients.
        net::snapshot::publish_if_changed();

        // Render the LED ring from the same entity state.
        render_leds(&entity, now, &mut led_frame);
        leds::LED_FRAME_SIGNAL.signal(led_frame);

        // Drain the latest observed pose from the head task.
        if let Some(actual) = head::HEAD_POSE_ACTUAL_SIGNAL.try_take() {
            entity.motor.head_pose_actual = actual;
        }

        // Pick a blit target based on mode:
        //  - camera mode: grab the latest completed camera frame and
        //    blit those QVGA RGB565 bytes directly. The camera frame
        //    is 320×240 (= the LCD resolution) so it's a 1:1 blit;
        //    we just unpack the byte stream into `Rgb565` pixels for
        //    `fill_contiguous`. We zero `last_rendered` so the next
        //    avatar-mode render does a full redraw rather than
        //    incorrectly thinking the avatar is already on screen.
        //  - avatar mode: the existing `frame_eq`-gated avatar blit.
        if camera_mode {
            if let Some(frame) = latest_camera_frame {
                let pixels = frame.chunks_exact(2).map(|chunk| {
                    Rgb565::from(RawU16::new(
                        (u16::from(chunk[0]) << 8) | u16::from(chunk[1]),
                    ))
                });
                match display.fill_contiguous(&canvas, pixels) {
                    Ok(()) => {}
                    Err(e) => {
                        defmt::error!("render: camera blit failed: {}", defmt::Debug2Format(&e));
                    }
                }
            }
            last_rendered = None;
        } else if last_rendered
            .as_ref()
            .is_none_or(|prev| *prev != entity.face)
        {
            // Compare faces directly: `Face` has `PartialEq` derived on
            // every visual field. Non-visual state (pose, sensors,
            // mind, events, tick) lives elsewhere on `Entity` and so
            // can't trigger spurious blits — `IdleHeadDrift` mutates
            // `motor.head_pose`, which is invisible to this dirty
            // check by construction.
            //
            // Draw is Infallible on `Framebuffer`; the `let _ =`
            // discards the `Result<(), Infallible>` without
            // triggering unwrap lints.
            let _ = entity.face.draw(&mut fb);
            match display.fill_contiguous(&canvas, fb.as_slice().iter().copied()) {
                Ok(()) => last_rendered = Some(entity.face),
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
/// onto `entity.motor.head_pose_actual` + logs a `cmd vs actual` line.
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
        watchdog::HEAD.beat();
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
/// it into the [`EmotionFromTouch`] modifier.
#[embassy_executor::task]
async fn touch_task(shared_i2c: SharedI2c) -> ! {
    let touch = ft6336u::Ft6336u::new(shared_i2c);
    touch::run_touch_loop(touch).await
}

/// `Si12T` body-touch polling task. Drives the back-of-head 3-zone pads
/// at 50 ms cadence and publishes per-zone state on
/// [`body_touch::BODY_TOUCH_SIGNAL`]. The render task drains it into
/// `entity.perception.body_touch`; modifiers / skills do their own
/// edge detection from there.
#[embassy_executor::task]
async fn body_touch_task(shared_i2c: SharedI2c) -> ! {
    body_touch::run_body_touch_loop(shared_i2c).await
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

/// AXP2101 battery-monitor task. Polls the gauge register at 1 Hz
/// and publishes the `SoC` percent on [`power::BATTERY_PERCENT_SIGNAL`]
/// for the render task to drain into `entity.perception.battery_percent`.
#[embassy_executor::task]
async fn power_task(shared_i2c: SharedI2c) -> ! {
    power::run_power_loop(shared_i2c).await
}

/// Audio task. Owns the I²S0 peripheral + DMA. Runs full bring-up
/// (I²S MCLK first so ES7210 can answer I²C, both codecs over I²C,
/// AW88298 volume + un-mute, then RX + TX DMA), then runs the RX RMS
/// loop (publishing on [`audio::AUDIO_RMS_SIGNAL`]) and TX feeder
/// (boot greeting + silence) concurrently inside this single task.
#[embassy_executor::task]
async fn audio_task(peripherals: audio::AudioPeripherals) -> ! {
    audio::run_audio_task(peripherals).await
}

/// Camera task. Owns the `LCD_CAM` peripheral + `DMA_CH1` + 11 DVP pins,
/// shares the I²C0 bus for SCCB control. Runs the GC0308 register
/// init at boot, then streams continuously: alternates ping-pong DMA
/// captures and copies each completed frame into a PSRAM scratch slot,
/// publishing the slice via [`camera::CAMERA_FRAME_SIGNAL`] for the
/// render task to blit AND running the block-grid tracker on every
/// frame, with results published on [`camera::CAMERA_TRACKING_SIGNAL`].
#[embassy_executor::task]
async fn camera_task(peripherals: camera::CameraPeripherals) -> ! {
    camera::run_camera_task(peripherals).await
}

/// Watchdog supervisor. Polls per-channel heartbeat counters (audio,
/// IMU, ambient, power, head) every 5 s and warns when any falls
/// silent. See `src/watchdog.rs` for the cadence-aware logic.
#[embassy_executor::task]
async fn watchdog_task() -> ! {
    watchdog::run_watchdog_loop().await
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

    // SPI2 is shared territory on CoreS3: LCD + SD card both sit on
    // SCK=GPIO36 + MOSI=GPIO37, and SD MISO is wired to GPIO35 — the
    // same physical pin as LCD DC. Park the bus in a `'static RefCell`
    // so each peripheral can hold its own `RefCellDevice` against the
    // same underlying `Spi` without contention.
    #[allow(clippy::items_after_statements)]
    static SPI2_BUS: StaticCell<RefCell<Spi<'static, Blocking>>> = StaticCell::new();
    let spi_bus = SPI2_BUS.init(RefCell::new(spi_bus));

    let spi_device = match RefCellDevice::new(spi_bus, cs, Delay) {
        Ok(dev) => dev,
        Err(e) => defmt::panic!("RefCellDevice init failed: {}", defmt::Debug2Format(&e)),
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

    // SD-card boot config (offline-first). The SD shares SPI2 with
    // the LCD and routes its MISO line to GPIO35 — the same pin the
    // LCD uses as DC — so `SdSpiDevice` flips GPIO35's OE bit per CS
    // edge (see `sd_spi.rs`). A missing card or unparseable file is
    // not fatal: the firmware logs a warn and uses `Config::default`.
    let sd_cs = Output::new(peripherals.GPIO4, Level::High, OutputConfig::default());
    let net_config = match storage::Storage::mount(spi_bus, sd_cs) {
        Ok(mut sd) => {
            let cfg = match sd.read_config() {
                Ok(c) => {
                    storage::log_config_summary(&c);
                    c
                }
                Err(e) => {
                    defmt::warn!("SD: read /sd/STACKCHAN.RON failed ({}); using defaults", e);
                    stackchan_net::Config::default()
                }
            };
            // Park the mounted storage handle in the shared mutex so
            // PUT /settings can write back without re-mounting, and
            // mirror the read config into the snapshot so GET /settings
            // sees what's actually persisted.
            storage::install_storage(sd).await;
            *storage::CONFIG_SNAPSHOT.lock().await = Some(cfg.clone());
            cfg
        }
        Err(e) => {
            defmt::warn!(
                "SD: mount failed ({}); booting offline-first with defaults",
                e
            );
            // No SD ⇒ leave CONFIG_SNAPSHOT as None so GET /settings
            // returns 503 (matches PUT /settings, which also 503s).
            stackchan_net::Config::default()
        }
    };
    // Bring up the radio + Wi-Fi station + embassy-net TCP/IP stack.
    // `esp_rtos::start` ran at boot (line above), which `esp_radio::init`
    // requires before claiming the WIFI peripheral. Both the
    // `Controller` and `WifiController` live in static cells so the
    // spawned tasks can borrow them for `'static`.
    #[allow(clippy::items_after_statements)]
    static RADIO: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
    let radio = match esp_radio::init() {
        Ok(c) => RADIO.init(c),
        Err(e) => defmt::panic!("esp-radio init failed: {}", defmt::Debug2Format(&e)),
    };
    let (wifi_controller, interfaces) =
        match esp_radio::wifi::new(radio, peripherals.WIFI, esp_radio::wifi::Config::default()) {
            Ok(r) => r,
            Err(e) => defmt::panic!("wifi::new failed: {}", defmt::Debug2Format(&e)),
        };

    let net_rng = Rng::new();
    let net_seed: u64 = u64::from(net_rng.random()) | (u64::from(net_rng.random()) << 32);
    let stack_resources = net::stack::STACK_RESOURCES
        .init(embassy_net::StackResources::<{ net::stack::STACK_SOCKETS }>::new());
    let (net_stack, net_runner) = embassy_net::new(
        interfaces.sta,
        embassy_net::Config::dhcpv4(embassy_net::DhcpConfig::default()),
        stack_resources,
        net_seed,
    );
    if let Err(e) = spawner.spawn(net::stack::net_runner_task(net_runner)) {
        defmt::panic!("spawn(net_runner_task) failed: {}", defmt::Debug2Format(&e));
    }
    if let Err(e) = spawner.spawn(net::wifi::wifi_task(
        wifi_controller,
        net_config.wifi.ssid.clone(),
        net_config.wifi.psk.clone(),
    )) {
        defmt::panic!("spawn(wifi_task) failed: {}", defmt::Debug2Format(&e));
    }
    let sntp_rtc_bus = I2cDevice::new(board_io.i2c_bus);
    if let Err(e) = spawner.spawn(net::sntp::sntp_task(
        net_stack,
        sntp_rtc_bus,
        net_config.time.sntp_servers.clone(),
    )) {
        defmt::panic!("spawn(sntp_task) failed: {}", defmt::Debug2Format(&e));
    }
    for _ in 0..net::http::HTTP_WORKER_COUNT {
        if let Err(e) = spawner.spawn(net::http::http_worker(net_stack)) {
            defmt::panic!("spawn(http_worker) failed: {}", defmt::Debug2Format(&e));
        }
    }
    if let Err(e) = spawner.spawn(net::mdns::mdns_task(
        net_stack,
        net_config.mdns.hostname.clone(),
    )) {
        defmt::panic!("spawn(mdns_task) failed: {}", defmt::Debug2Format(&e));
    }

    // Sample the chip's hardware RNG twice — once for IdleDrift
    // (eyes), once for IdleHeadDrift (head). Using independent seeds
    // keeps the two idle schedules from drifting in lockstep across a
    // multi-unit deployment AND from being correlated within a single
    // unit. xorshift32 forbids zero, so re-roll on the (1 in 2^32)
    // chance we hit it — the loop almost always exits first try.
    let rng = Rng::new();
    let sample_seed = || loop {
        if let Some(s) = NonZeroU32::new(rng.random()) {
            return s;
        }
    };
    let drift_seed = sample_seed();
    let head_drift_seed = sample_seed();
    defmt::info!(
        "idle seeds: eye={=u32:#010x} head={=u32:#010x}",
        drift_seed.get(),
        head_drift_seed.get()
    );

    if let Err(e) = spawner.spawn(render_task(display, drift_seed, head_drift_seed)) {
        defmt::panic!("spawn render_task failed: {}", defmt::Debug2Format(&e));
    }
    if let Err(e) = spawner.spawn(head_task(board_io.head)) {
        defmt::panic!("spawn head_task failed: {}", defmt::Debug2Format(&e));
    }
    if let Err(e) = spawner.spawn(touch_task(board_io.i2c)) {
        defmt::panic!("spawn touch_task failed: {}", defmt::Debug2Format(&e));
    }
    if let Err(e) = spawner.spawn(body_touch_task(I2cDevice::new(board_io.i2c_bus))) {
        defmt::panic!("spawn body_touch_task failed: {}", defmt::Debug2Format(&e));
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
    if let Err(e) = spawner.spawn(power_task(I2cDevice::new(board_io.i2c_bus))) {
        defmt::panic!("spawn power_task failed: {}", defmt::Debug2Format(&e));
    }
    if let Err(e) = spawner.spawn(watchdog_task()) {
        defmt::panic!("spawn watchdog_task failed: {}", defmt::Debug2Format(&e));
    }

    // Audio subsystem. The task owns I²S0 + DMA + audio pins and
    // runs the full bring-up sequence itself (I²S MCLK first, then
    // AW88298, then ES7210). Failures inside the task log-and-park —
    // audio degrades to "silent mic, muted speaker" rather than
    // halting the avatar.
    let audio_periph = audio::AudioPeripherals {
        i2s: peripherals.I2S0,
        dma: peripherals.DMA_CH0,
        mclk: peripherals.GPIO0,
        bclk: peripherals.GPIO34,
        ws: peripherals.GPIO33,
        din: peripherals.GPIO14,
        dout: peripherals.GPIO13,
        amp_bus: I2cDevice::new(board_io.i2c_bus),
        adc_bus: I2cDevice::new(board_io.i2c_bus),
    };
    if let Err(e) = spawner.spawn(audio_task(audio_periph)) {
        defmt::panic!("spawn audio_task failed: {}", defmt::Debug2Format(&e));
    }

    // Camera subsystem. The task owns LCD_CAM + DMA_CH1 + the 11 DVP
    // pins and shares the existing I²C0 bus for SCCB control. It
    // runs SCCB init + LCD_CAM construction once at boot, then
    // streams continuously: each frame is published on
    // `CAMERA_FRAME_SIGNAL` for preview AND fed through the block-
    // grid tracker, with the result landing on
    // `CAMERA_TRACKING_SIGNAL` for the engine. Failures inside the
    // task log-and-park — camera silently degrades to "no preview /
    // no tracking" while the rest of the avatar keeps running.
    let camera_periph = camera::CameraPeripherals {
        lcd_cam: peripherals.LCD_CAM,
        dma: peripherals.DMA_CH1,
        i2c: I2cDevice::new(board_io.i2c_bus),
        pclk: peripherals.GPIO45,
        href: peripherals.GPIO38,
        vsync: peripherals.GPIO46,
        d0: peripherals.GPIO39,
        d1: peripherals.GPIO40,
        d2: peripherals.GPIO41,
        d3: peripherals.GPIO42,
        d4: peripherals.GPIO15,
        d5: peripherals.GPIO16,
        d6: peripherals.GPIO48,
        d7: peripherals.GPIO47,
    };
    if let Err(e) = spawner.spawn(camera_task(camera_periph)) {
        defmt::panic!("spawn camera_task failed: {}", defmt::Debug2Format(&e));
    }

    // One-shot wall-clock read for the boot-log timestamp. RTC
    // failures are warn-only (logged inside `wallclock::read_datetime`);
    // we just skip the timestamped log line when no reading is
    // available. The boot greeting clips remain available for explicit
    // playback via `audio-bench`.
    let rtc_bus = I2cDevice::new(board_io.i2c_bus);
    if let Some(dt) = wallclock::read_datetime(rtc_bus).await {
        let mut rtc_buf = [0u8; 19];
        defmt::info!(
            "boot @ {=str} (RTC)",
            bm8563::format_datetime(dt, &mut rtc_buf),
        );
    }

    defmt::info!("boot complete — idle heartbeat");
    loop {
        Timer::after(Duration::from_secs(5)).await;
        defmt::debug!("heartbeat");
    }
}
