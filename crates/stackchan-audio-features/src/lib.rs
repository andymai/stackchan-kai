//! Audio feature extraction for on-device keyword spotting.
//!
//! Implements the audio frontend half of the `microWakeWord`
//! pipeline ([kahrendt/microWakeWord]): 16 kHz mono `i16` PCM in,
//! quantized log-mel feature frames out, one frame every 10 ms.
//! Pure Rust + `no_std`. Host-testable in isolation against
//! synthetic fixtures.
//!
//! No inference. The downstream `MixConv` classifier (`TFLite-micro`
//! or a future pure-Rust equivalent) consumes [`MelFrame`]s but
//! lives in a follow-up. Shipping the frontend on its own means
//! the next session's choice of inference path isn't blocked on
//! DSP work, and it lets the parameters be exercised against
//! ground-truth fixtures before any model is plugged in.
//!
//! # Pipeline
//!
//! ```text
//! i16 PCM (16 kHz mono)
//!   │  push_samples(): rolling 480-sample window, 160-sample hop
//!   ▼
//! Hann window (480 taps, symmetric)
//!   │
//!   ▼
//! Zero-pad to 512, real FFT
//!   │
//!   ▼
//! Magnitude spectrum (257 bins)
//!   │
//!   ▼
//! Mel filterbank (40 triangles, 125–7500 Hz)
//!   │
//!   ▼
//! Natural log, int8 quantization
//!   │
//!   ▼
//! MelFrame { features: [i8; 40] }
//! ```
//!
//! # Streaming contract
//!
//! [`MelFrontend::push_samples`] takes a slice of `i16` samples
//! and returns at most one [`MelFrame`] per call. Callers feed
//! audio in any chunk size; the frontend buffers until a window's
//! worth of samples (480) is available, emits a frame, then slides
//! the window by the hop length (160 samples = 10 ms). Drop the
//! return value to discard the frame.
//!
//! For a 20 ms `AudioFrame` (320 samples) input, the frontend
//! emits two frames per call after the first window fills (the
//! first input fills 320/480, the second pushes to 640 = 1 hop
//! past window-full, and so on).
//!
//! # Bit-exactness disclaimer
//!
//! The reference `microWakeWord` frontend uses TensorFlow's
//! `AudioFrontend` op, which includes spectral subtraction for
//! noise reduction and PCAN automatic gain control. This crate
//! omits both — the simpler log-mel pipeline is bit-exact against
//! reference numpy implementations for clean inputs, but real
//! microphone audio will diverge from the reference network's
//! expected feature distribution. The follow-up PR that lands
//! inference should either port the spectral subtraction + PCAN
//! kernels or train a model on the simpler features this crate
//! produces.
//!
//! [kahrendt/microWakeWord]: https://github.com/kahrendt/microWakeWord

#![no_std]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "DSP code casts freely between i16 PCM, f32 spectra, and i8 quantized output by design; \
              the casts are bounded by the upstream sample widths and explicitly clamped on the i8 \
              quantization edge."
)]

extern crate alloc;

pub mod frontend;
pub mod mel;
pub mod quantize;
pub mod window;

pub use frontend::{HOP_SAMPLES, MEL_BIN_COUNT, MelFrame, MelFrontend, WINDOW_SAMPLES};
