---
crate: stackchan-audio-features
role: Streaming log-mel feature frontend for on-device keyword spotting
bus: none
transport: "i16 PCM in → MelFrame out"
no_std: true (with alloc unused by the pipeline)
unsafe: forbidden
status: experimental (v0.x)
---

# stackchan-audio-features

`no_std` audio frontend for on-device keyword spotting. Implements
the streaming feature pipeline that `microWakeWord` (and most
TFLite-micro keyword-spotting models) expect on their input tensor:
16 kHz mono `i16` PCM in, quantized log-mel feature frames out, one
frame every 10 ms. Pure Rust, host-testable in isolation against
synthetic fixtures.

No inference. The downstream `MixConv` classifier (TFLite-micro via
[`esp-tflite-micro-sys`](../esp-tflite-micro-sys), or a future
pure-Rust equivalent) consumes [`MelFrame`]s but lives in the
firmware crate. Shipping the frontend on its own means the
inference path can swap without the DSP work moving.

## Pipeline

```mermaid
flowchart LR
    PCM[i16 PCM 16 kHz mono] --> Buf[push_samples — buffer to 480 / hop 160]
    Buf --> Win[Hann window, 480 periodic taps]
    Win --> Pad[Zero-pad to 512]
    Pad --> FFT[Real FFT — microfft, size-512]
    FFT --> Mag[Magnitude spectrum — 257 bins]
    Mag --> Mel[Mel filterbank — 40 triangles, 125–7500 Hz]
    Mel --> Log[Natural log, 1e-6 floor]
    Log --> Q[int8 asymmetric quantize]
    Q --> Frame[MelFrame &lbrace; features: i8 array &rbrace;]
```

Constants live in `frontend.rs` and are the single source of truth
for cadence: `SAMPLE_RATE_HZ`, `WINDOW_SAMPLES`, `HOP_SAMPLES`,
`FFT_SAMPLES`, `FFT_BIN_COUNT`, `MEL_BIN_COUNT`, `MEL_LOWER_HZ`,
`MEL_UPPER_HZ`. Change one of them and the rest of the crate follows.

## What's here

- [`MelFrontend`] — operator-facing streaming entry point. `new()`
  for the model's default int8 quantization; [`MelFrontend::with_quant`]
  for a model-specific `(scale, zero_point)`. [`MelFrontend::push_samples`]
  accepts any chunk size and drives a callback zero or more times per
  call — once per completed hop boundary. [`MelFrontend::reset`]
  drops the rolling buffer state (e.g. after a wake-word fire).
- [`MelFrame`] — one frame of int8 mel features. `Copy`-able plain
  data.
- [`HannWindow`] — 480-sample **periodic** taps (matches
  `numpy.hanning(N, sym=False)` / `scipy.signal.windows.hann(N, sym=False)`;
  using the symmetric form drifts a few dB per bin).
- [`MelFilterbank`] — triangular filters spanning 125 Hz to 7 500 Hz
  on the Slaney mel scale used by `librosa`, `tensorflow.signal`,
  and TFLite's `AudioFrontend` op.
- [`log_then_quantize`] / [`quantize_scalar`] / [`QuantParams`] —
  natural log with a 1e-6 floor, then asymmetric per-tensor
  quantization with the model's `scale` and `zero_point`.
- [`hz_to_mel`] / [`mel_to_hz`] — Slaney mel-scale conversion
  helpers, used to lay out the filterbank.

## What this crate is not

- **No inference.** The classifier lives in
  [`stackchan-firmware`'s wake-word task](../stackchan-firmware/src/wake_word.rs)
  and runs the TFLite-micro interpreter from
  [`esp-tflite-micro-sys`](../esp-tflite-micro-sys).
- **No spectral subtraction or PCAN gain control.** The
  `microWakeWord` reference frontend includes both for noise-robust
  live audio; this crate's outputs are bit-exact against reference
  numpy implementations of the simpler log-mel pipeline. Follow-up
  either ports those kernels or trains against the simpler features.

## Verification

Per-module `#[cfg(test)]` blocks cover: Hann symmetry + scaling, mel
filter edges + partition-of-unity, quantization rounding +
saturation + floor handling, streaming window/hop cadence, and a
1 kHz pure-tone fixture that asserts peak energy lands in the
expected mel band. The tests run on host in `just check` — no
firmware target, no model required.

## Gotchas

1. **Periodic Hann, not symmetric.** The 480 taps come from
   `(sym=False)`. Using the symmetric form gives a few-dB drift per
   bin; the discrepancy is silent and only shows up against
   reference fixtures.
2. **`MelFrontend::push_samples` is push-not-pull.** Callers feed
   arbitrary chunk sizes; the frontend buffers internally and fires
   the callback zero or more times per call. For a 20 ms
   `AudioFrame` (320 samples), one call usually produces two
   frames. Pass `|_| {}` as the callback to discard.
3. **Int8 quantization is asymmetric.** [`QuantParams::DEFAULT`]
   carries `microWakeWord`'s shipped parameters; a different model
   needs `with_quant`. Wrong `(scale, zero_point)` produces silent
   feature drift, not a runtime error.
4. **`libm` for log/sqrt/powf.** `core::f32` doesn't expose those
   on `xtensa-esp32s3-none-elf` (no hardware-FPU intrinsic). The
   `libm` dep matches `microWakeWord`'s reference choice and keeps
   outputs bit-exact against it.

## Integration

- Single consumer today: [`stackchan-firmware`'s wake-word
  task](../stackchan-firmware/src/wake_word.rs). The task subscribes
  to `AUDIO_FRAME_PUBSUB` (20 ms PCM frames published by the audio
  task), feeds each frame to [`MelFrontend::push_samples`], and
  shifts each emitted [`MelFrame`] into the TFLite-micro input
  tensor's circular buffer.
- Depends only on [`microfft`](https://crates.io/crates/microfft)
  (size-512 real FFT) and [`libm`](https://crates.io/crates/libm).
- **Stability:** Experimental in v0.x. The constants in
  `frontend.rs` are pinned to the `microWakeWord` reference
  pipeline; a different model architecture may require parameter
  changes.

[`MelFrontend`]: src/frontend.rs
[`MelFrontend::push_samples`]: src/frontend.rs
[`MelFrontend::with_quant`]: src/frontend.rs
[`MelFrontend::reset`]: src/frontend.rs
[`MelFrame`]: src/frontend.rs
[`HannWindow`]: src/window.rs
[`MelFilterbank`]: src/mel.rs
[`log_then_quantize`]: src/quantize.rs
[`quantize_scalar`]: src/quantize.rs
[`QuantParams`]: src/quantize.rs
[`QuantParams::DEFAULT`]: src/quantize.rs
[`hz_to_mel`]: src/mel.rs
[`mel_to_hz`]: src/mel.rs
