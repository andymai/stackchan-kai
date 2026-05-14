# stackchan-audio-features

`no_std` + `alloc` audio frontend for on-device keyword spotting.

Implements the streaming feature pipeline `microWakeWord` and most
`TFLite-micro` keyword-spotting models expect on their input
tensor:

1. **Hann window** ([`window::HannWindow`]) — 480-sample
   *periodic* taps (matches `numpy.hanning(N, sym=False)` /
   `scipy.signal.windows.hann(N, sym=False)`; using the symmetric
   form drifts a few dB per bin).
2. **512-point real FFT** — via [`microfft`], zero-padding the
   480 windowed samples to 512.
3. **40-channel mel filterbank** ([`mel::MelFilterbank`]) —
   triangular filters spanning 125 Hz to 7 500 Hz on the Slaney
   mel scale used by `librosa`, `tensorflow.signal`, and TFLite's
   `AudioFrontend` op.
4. **Log + int8 quantize** ([`quantize::log_then_quantize`]) —
   natural log with a 1e-6 floor, then asymmetric per-tensor
   quantization with the model's `scale` and `zero_point`.

`MelFrontend::push_samples` is the operator-facing entry point:
feed any chunk size of `i16` PCM at 16 kHz mono, receive one
`MelFrame` ([`frontend::MelFrame`]) every 10 ms via a callback.

## What this crate is not

- **No inference.** The MixConv classifier that consumes these
  features lives in a follow-up. The frontend ships on its own
  so the next session's choice between TFLite-micro-via-FFI,
  a custom MixConv interpreter, or a different architecture
  isn't blocked on DSP work.
- **No spectral subtraction or PCAN gain control.** The
  `microWakeWord` reference frontend includes both for
  noise-robust live audio; this crate's outputs are bit-exact
  against reference numpy implementations of the simpler
  log-mel pipeline. A follow-up will either port those kernels
  or train against the simpler features.

## Verification

23 host tests cover: Hann symmetry + scaling, mel filter edges
+ partition-of-unity, quantization rounding + saturation +
floor, streaming window/hop cadence, and a 1 kHz pure-tone
fixture that asserts the peak energy lands in the expected mel
band (around index 12 of 40 for the 125–7 500 Hz band).

[`window::HannWindow`]: src/window.rs
[`mel::MelFilterbank`]: src/mel.rs
[`quantize::log_then_quantize`]: src/quantize.rs
[`frontend::MelFrame`]: src/frontend.rs
[`microfft`]: https://crates.io/crates/microfft
