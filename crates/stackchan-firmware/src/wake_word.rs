//! On-device wake-word detector.
//!
//! Opt-in via `behavior.wake_word_enabled` **and** the presence of
//! `/sd/WAKE_WORD.tflite`. When either is missing, the task parks
//! immediately — no audio subscriber slot, no tensor arena
//! allocated, no interpreter constructed.
//!
//! ## Pipeline
//!
//! ```text
//! AUDIO_FRAME_PUBSUB  ─►  MelFrontend  ─►  Interpreter  ─►  REMOTE_COMMAND_SIGNAL
//!   (320 i16 samples       (480 / 160      (1 × 40 i8        (StartListen, drained
//!    every 20 ms)           window/hop)     per timestep)     by the render loop)
//! ```
//!
//! Each 20 ms `AudioFrame` (320 samples @ 16 kHz) yields one or two
//! `MelFrame`s through the streaming mel frontend (480-sample window,
//! 160-sample hop = 10 ms hop). Each mel frame's 40 int8 features
//! become the input tensor for a single `Interpreter::invoke` call;
//! the microWakeWord streaming model keeps its own history through
//! `VarHandle` / `ReadVariable` / `AssignVariable` resource-variable
//! ops, so the host side feeds one timestep at a time.
//!
//! When the output score crosses the operator-supplied threshold
//! (`behavior.wake_word_threshold`, default `100`), the task
//! signals [`crate::net::http::REMOTE_COMMAND_SIGNAL`] with
//! [`RemoteCommand::StartListen`] for [`POST_WAKE_CAPTURE_MS`]. The
//! render loop drains the signal and applies the same effects as
//! an operator-initiated `POST /listen`: trigger the sidecar PCM
//! capture *and* hand the variant to the modifier graph so the
//! avatar visibly reacts — [`stackchan_core::Attention::Listening`] hold,
//! ear decorator overlay, acknowledgement chirp. Both the local
//! and operator paths converge on this one signal.

// microWakeWord, ESPHome, TFLite Micro, MixConv, MicroInterpreter
// appear throughout the prose; per-occurrence backticking buries
// the explanation. Same pattern as `crate::ble` and the
// `esp-tflite-micro-sys` crate's module-level allow.
#![allow(clippy::doc_markdown)]

use alloc::vec;

use embassy_sync::pubsub::WaitResult;
use embassy_time::Instant;
use esp_tflite_micro_sys::{Interpreter, Resolver, TfLiteStatus};
use stackchan_audio_features::{MEL_BIN_COUNT, MelFrontend};
use stackchan_core::RemoteCommand;

use crate::audio::AUDIO_FRAME_PUBSUB;
use crate::net::http::REMOTE_COMMAND_SIGNAL;

/// Listen-window duration emitted on each wake fire. Matches the
/// operator-initiated `POST /listen` default so the two paths
/// converge on the same sidecar request shape and the same
/// `Attention::Listening` hold.
const POST_WAKE_CAPTURE_MS: u32 = 4_000;

/// Minimum gap between consecutive `REMOTE_COMMAND_SIGNAL.signal()`
/// calls. Without this, a real wake utterance — which spans dozens
/// of mel frames above threshold — would call `signal()` on every
/// frame. Since `Signal` is one-shot consume-and-clear with
/// last-write-wins payload, the downstream consumers (sidecar
/// capture, `Attention::Listening` modifier) would re-fire as soon
/// as the first listen window finished, cascading into a second /
/// third capture from queued signals. The cooldown is set to
/// `POST_WAKE_CAPTURE_MS + 1 000 ms` so a new fire is only possible
/// after the previous window has fully elapsed, ruling out the
/// cascade pattern regardless of which downstream is slower to
/// drain.
const POST_WAKE_COOLDOWN_MS: u64 = POST_WAKE_CAPTURE_MS as u64 + 1_000;

/// Index of the wake-word score in the model output tensor. For
/// single-scalar microWakeWord outputs the tensor is a 1-byte
/// `[1]` shape and index 0 holds the score; multi-class outputs
/// would index the positive class explicitly.
const WAKE_CLASS_INDEX: usize = 0;

/// Upper bound on the number of `MelFrame`s a single 20 ms audio
/// frame can emit. With 480-sample windows and 160-sample hops at
/// 16 kHz, the streaming frontend buffers state across frames; one
/// 320-sample audio frame produces at most two mel frames after
/// the initial window fills. Round up to 4 for safety margin.
const MAX_MEL_FRAMES_PER_AUDIO_FRAME: usize = 4;

/// Wake-word task entry point.
///
/// `enabled` mirrors `behavior.wake_word_enabled`; `model_bytes`
/// is the on-SD-card TFLite model read once at boot and leaked to
/// a `'static` slice. An empty `model_bytes` parks the task — the
/// missing-file path through `Storage::read_wake_word_model`
/// returns `Vec::new()` rather than an error, so this is the
/// expected default state for units without a model installed.
///
/// `threshold` mirrors `behavior.wake_word_threshold` — the int8
/// score above which the model output is treated as a positive
/// detection. `arena_bytes` mirrors
/// `behavior.wake_word_arena_kib * 1024` — the size of the TFLM
/// tensor arena, allocated once from PSRAM at task start.
#[embassy_executor::task]
pub async fn wake_word_task(
    enabled: bool,
    threshold: i8,
    arena_bytes: usize,
    model_bytes: &'static [u8],
) -> ! {
    if !enabled || model_bytes.is_empty() {
        defmt::info!(
            "wake-word: disabled (enabled={=bool}, model={=usize} bytes); idle",
            enabled,
            model_bytes.len(),
        );
        park_forever().await;
    }

    let Ok(mut subscriber) = AUDIO_FRAME_PUBSUB.subscriber() else {
        defmt::error!("wake-word: subscriber slot exhausted; idle");
        park_forever().await;
    };

    // PSRAM-backed tensor arena. Leaking the Vec yields a
    // `'static mut [u8]` that the interpreter can borrow for the
    // task's (i.e. program's) lifetime.
    let arena: &'static mut [u8] = vec![0_u8; arena_bytes].leak();

    let Some(mut resolver) = Resolver::new() else {
        defmt::error!("wake-word: resolver alloc failed; idle");
        park_forever().await;
    };
    if let Err(e) = register_micro_wake_word_ops(&mut resolver) {
        defmt::error!(
            "wake-word: op registration failed (status={=i32}); idle",
            e.0
        );
        park_forever().await;
    }

    let Some(mut interp) = Interpreter::new(model_bytes, &resolver, arena) else {
        defmt::error!("wake-word: Interpreter::new failed (bad model or schema mismatch); idle",);
        park_forever().await;
    };

    if let Err(e) = interp.allocate_tensors() {
        defmt::error!(
            "wake-word: allocate_tensors failed (status={=i32}); idle",
            e.0,
        );
        park_forever().await;
    }

    defmt::info!(
        "wake-word: interpreter ready ({=usize} inputs, {=usize} outputs, arena={=usize} B, threshold={=i8})",
        interp.inputs_len(),
        interp.outputs_len(),
        arena_bytes,
        threshold,
    );

    let mut frontend = MelFrontend::new();
    let mut mel_buf: heapless::Vec<[i8; MEL_BIN_COUNT], MAX_MEL_FRAMES_PER_AUDIO_FRAME> =
        heapless::Vec::new();
    // Earliest wall-clock instant at which the next wake fire is
    // allowed. `None` (the initial state) means "any score is
    // eligible to fire"; updated to `now + cooldown` on each fire.
    let mut next_fire_after: Option<Instant> = None;

    loop {
        match subscriber.next_message().await {
            WaitResult::Message(frame) => {
                // Collect mel frames into a small stack buffer
                // before running inference. Doing the invoke inside
                // the `push_samples` closure works but threads two
                // `&mut` borrows (frontend + interp) through the
                // same expression; collecting first is clearer and
                // keeps inference cost out of the frontend hot path.
                mel_buf.clear();
                frontend.push_samples(&frame.samples, |mel_frame| {
                    if mel_buf.push(mel_frame.features).is_err() {
                        defmt::warn!("wake-word: mel_buf overflow; frame dropped");
                    }
                });
                for features in &mel_buf {
                    run_inference(&mut interp, features, threshold, &mut next_fire_after);
                }
            }
            WaitResult::Lagged(n) => {
                defmt::warn!("wake-word: lagged {=u64} frames", n);
            }
        }
    }
}

/// Feed one mel frame through the interpreter and signal
/// [`REMOTE_COMMAND_SIGNAL`] with [`RemoteCommand::StartListen`] on
/// detection. Logs and continues on transient errors so a single
/// bad invoke doesn't kill the task.
///
/// `next_fire_after` carries the cooldown deadline across calls:
/// a fresh fire is suppressed (no `signal()`) until the deadline
/// has passed, which prevents a single utterance — spanning dozens
/// of consecutive high-score mel frames — from queueing repeated
/// listen windows behind the first one.
fn run_inference(
    interp: &mut Interpreter<'_>,
    features: &[i8; MEL_BIN_COUNT],
    threshold: i8,
    next_fire_after: &mut Option<Instant>,
) {
    let Some(input) = interp.input_bytes_mut(0) else {
        defmt::warn!("wake-word: input(0) missing after allocate_tensors");
        return;
    };
    if input.len() < features.len() {
        defmt::warn!(
            "wake-word: input tensor too small ({=usize} < {=usize})",
            input.len(),
            features.len(),
        );
        return;
    }
    // Bit-pattern-preserving copy: `i8::cast_unsigned()` is the
    // explicit no-op bitcast TFLM's int8 input tensor expects on a
    // `u8*` byte buffer. The per-element loop keeps the firmware's
    // `deny(unsafe_code)` clean — vectorisation is the optimiser's
    // problem and at 40 bytes / mel frame doesn't matter.
    for (dst, &feat) in input[..features.len()].iter_mut().zip(features.iter()) {
        *dst = feat.cast_unsigned();
    }

    if let Err(e) = interp.invoke() {
        defmt::warn!("wake-word: invoke failed (status={=i32})", e.0);
        return;
    }
    let Some(output) = interp.output_bytes(0) else {
        return;
    };
    let Some(&score_u8) = output.get(WAKE_CLASS_INDEX) else {
        return;
    };
    let score = score_u8.cast_signed();
    if score < threshold {
        return;
    }
    let now = Instant::now();
    if let Some(deadline) = *next_fire_after
        && now < deadline
    {
        // High-score frame inside the cooldown window — almost
        // certainly the same utterance that just fired. Suppress
        // silently; logging here would spam at the mel-frame rate.
        return;
    }
    defmt::info!("wake-word: fired (score={=i8})", score);
    REMOTE_COMMAND_SIGNAL.signal(RemoteCommand::StartListen {
        duration_ms: POST_WAKE_CAPTURE_MS,
    });
    *next_fire_after = Some(now + embassy_time::Duration::from_millis(POST_WAKE_COOLDOWN_MS));
}

/// Register the 20-op microWakeWord operator set on `resolver`.
///
/// Order matters only for the resolver's internal table indexing,
/// not for inference semantics — TFLM consults the table via op
/// type rather than insertion order. Listing them here in the same
/// order ESPHome's `streaming_model.cpp` registers them keeps the
/// table layout byte-comparable across builds.
fn register_micro_wake_word_ops(resolver: &mut Resolver) -> Result<(), TfLiteStatus> {
    resolver.add_call_once()?;
    resolver.add_var_handle()?;
    resolver.add_reshape()?;
    resolver.add_read_variable()?;
    resolver.add_strided_slice()?;
    resolver.add_concatenation()?;
    resolver.add_assign_variable()?;
    resolver.add_conv_2d()?;
    resolver.add_mul()?;
    resolver.add_add()?;
    resolver.add_mean()?;
    resolver.add_fully_connected()?;
    resolver.add_logistic()?;
    resolver.add_quantize()?;
    resolver.add_depthwise_conv_2d()?;
    resolver.add_average_pool_2d()?;
    resolver.add_max_pool_2d()?;
    resolver.add_pad()?;
    resolver.add_pack()?;
    resolver.add_split_v()?;
    Ok(())
}

/// Spin forever, parking the task when it can't proceed
/// (no model, no subscriber slot, allocation failure, …).
async fn park_forever() -> ! {
    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(3600)).await;
    }
}
