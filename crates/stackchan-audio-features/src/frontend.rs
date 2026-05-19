//! Streaming mel-spectrogram frontend.
//!
//! Wraps the Hann window, real-input FFT, mel filterbank, and
//! log-quantize stages into a stateful pipeline that accepts
//! `i16` PCM in any chunk size and emits one [`MelFrame`] per
//! window-hop boundary.
//!
//! Window length is 480 samples (30 ms @ 16 kHz); hop length is
//! 160 samples (10 ms). Each FFT runs on a Hann-windowed 480-sample
//! frame, zero-padded to 512 to feed `microfft::real::rfft_512`.

use libm::sqrtf;
use microfft::real::rfft_512;

use crate::mel::MelFilterbank;
use crate::quantize::{QuantParams, log_then_quantize};
use crate::window::HannWindow;

/// Mono PCM sample rate the frontend's filterbank assumes.
pub const SAMPLE_RATE_HZ: u32 = 16_000;

/// Number of samples in one analysis window: 30 ms @ 16 kHz.
pub const WINDOW_SAMPLES: usize = 480;

/// Hop between successive windows: 10 ms @ 16 kHz.
pub const HOP_SAMPLES: usize = 160;

/// FFT length. Smallest power of two ≥ [`WINDOW_SAMPLES`]; the
/// extra `32` samples are zero-padding. Fixed at compile time so
/// `microfft::real::rfft_512` can monomorphise.
pub const FFT_SAMPLES: usize = 512;

/// Number of magnitude bins produced by an `N`-point real FFT:
/// `N / 2 + 1` (one-sided spectrum including DC and Nyquist).
pub const FFT_BIN_COUNT: usize = FFT_SAMPLES / 2 + 1;

/// Number of mel-filterbank output channels per frame.
pub const MEL_BIN_COUNT: usize = 40;

/// Lower edge of the mel filterbank, in Hz. Speech energy below
/// this point is mostly room noise; `microWakeWord`'s published
/// frontend uses 125 Hz.
pub const MEL_LOWER_HZ: f32 = 125.0;

/// Upper edge of the mel filterbank, in Hz. `microWakeWord` uses
/// 7500 Hz — slightly below the 8 kHz Nyquist so the final
/// triangle isn't truncated.
pub const MEL_UPPER_HZ: f32 = 7500.0;

/// One quantized log-mel feature frame, output every
/// [`HOP_SAMPLES`] samples (~10 ms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MelFrame {
    /// `int8` mel features, one per filterbank channel. Layout
    /// matches the input tensor TFLite-micro keyword-spotting
    /// models expect (`shape = [1, 40, 1]` for streaming
    /// inference).
    pub features: [i8; MEL_BIN_COUNT],
}

/// Streaming mel-feature extractor.
///
/// Owns the rolling input buffer, precomputed Hann window, and
/// precomputed mel filterbank. One frontend instance handles
/// continuous audio for the lifetime of an inference session.
pub struct MelFrontend {
    /// Ring-buffered sample window. In steady state, [`head`]
    /// points to the oldest sample (also the next overwrite
    /// position); the buffer always holds the last
    /// [`WINDOW_SAMPLES`] samples in modular order.
    ///
    /// The ring layout avoids an O([`WINDOW_SAMPLES`]) memmove on
    /// every sample. At 16 kHz that's the difference between
    /// ~7.7 M float copies/sec (linear-shift) and one indexed
    /// write/sec (ring). Two contiguous `copy_from_slice` calls
    /// reorder the ring into time-major layout once per emitted
    /// frame (every 10 ms), which costs ~1.9 KiB of memcpy
    /// vs. ~30 MB/s of buffer churn under the old layout.
    ///
    /// [`head`]: Self::head
    buf: [f32; WINDOW_SAMPLES],
    /// Count of samples written during the *initial* fill
    /// (0..[`WINDOW_SAMPLES`]). Once equal to [`WINDOW_SAMPLES`]
    /// the buffer is primed and subsequent writes use [`head`]
    /// in ring mode; this counter stays pinned at
    /// [`WINDOW_SAMPLES`] until [`reset`](Self::reset) is called.
    filled: usize,
    /// Ring index of the oldest sample, which is also the slot
    /// the next steady-state write overwrites. Meaningless
    /// before priming completes; left at zero during initial
    /// fill so the first emitted frame's two-copy reorder
    /// degenerates to a single contiguous copy.
    head: usize,
    /// Hopped-sample countdown — number of new samples that need
    /// to land before the next frame is ready, after the initial
    /// window has filled. Starts at 0 (first frame fires the moment
    /// the buffer first contains 480 samples).
    hop_remaining: usize,
    /// Precomputed Hann taps.
    hann: HannWindow,
    /// Precomputed filterbank weights.
    mel: MelFilterbank,
    /// Per-tensor quantization for the downstream model's input.
    quant: QuantParams,
}

impl MelFrontend {
    /// Construct with the default `microWakeWord` quantization
    /// parameters. Plug in model-specific values via
    /// [`MelFrontend::with_quant`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_quant(QuantParams::DEFAULT)
    }

    /// Construct with explicit quantization parameters drawn from
    /// the target model's `quantization_parameters` metadata.
    #[must_use]
    pub fn with_quant(quant: QuantParams) -> Self {
        Self {
            buf: [0.0; WINDOW_SAMPLES],
            filled: 0,
            head: 0,
            hop_remaining: 0,
            hann: HannWindow::new(),
            mel: MelFilterbank::new(),
            quant,
        }
    }

    /// Push `i16` PCM samples and consume each fully-formed frame
    /// via `on_frame`.
    ///
    /// The callback shape (vs. returning frames) keeps the
    /// allocation strategy in the caller's hands — a wake-word
    /// task can feed each frame directly into TFLite-micro's
    /// streaming input tensor without ever building a `Vec`.
    pub fn push_samples<F: FnMut(MelFrame)>(&mut self, samples: &[i16], mut on_frame: F) {
        for &sample in samples {
            self.feed_one(f32::from(sample), &mut on_frame);
        }
    }

    /// Internal: handle one sample, emit a frame if a window
    /// boundary just crossed.
    fn feed_one<F: FnMut(MelFrame)>(&mut self, sample: f32, on_frame: &mut F) {
        if self.filled < WINDOW_SAMPLES {
            self.buf[self.filled] = sample;
            self.filled += 1;
            if self.filled == WINDOW_SAMPLES {
                // First emit; head stays at 0 because the next
                // steady-state write overwrites the oldest sample
                // (index 0, the very first sample we wrote).
                on_frame(self.emit_frame());
                self.hop_remaining = HOP_SAMPLES;
            }
            return;
        }
        // Steady-state: O(1) ring write. The slot we overwrite is
        // the oldest sample by construction, and advancing `head`
        // makes the *next*-oldest the new oldest.
        self.buf[self.head] = sample;
        self.head = (self.head + 1) % WINDOW_SAMPLES;
        self.hop_remaining -= 1;
        if self.hop_remaining == 0 {
            on_frame(self.emit_frame());
            self.hop_remaining = HOP_SAMPLES;
        }
    }

    /// Reset to the unprimed state. Call when the audio stream
    /// has a discontinuity (DMA error, silence gap) so the next
    /// frame waits for a fresh full window of post-gap samples
    /// rather than smearing pre- and post-gap audio together.
    pub fn reset(&mut self) {
        self.filled = 0;
        self.head = 0;
        self.hop_remaining = 0;
        self.buf.fill(0.0);
    }

    /// Render the current `buf` contents into a quantized frame.
    fn emit_frame(&self) -> MelFrame {
        // Window into a scratch buffer, then zero-pad up to
        // FFT_SAMPLES. `microfft::real::rfft_512` operates
        // in place, so we keep its input padded with zeros from
        // index WINDOW_SAMPLES..FFT_SAMPLES.
        let mut fft_buf = [0.0_f32; FFT_SAMPLES];
        // Reorder the ring from oldest-first to time-major
        // layout via two contiguous `copy_from_slice`s. When
        // `head == 0` the second slice is empty and the first
        // copies the whole window — same as the old linear-shift
        // layout's single copy. Otherwise we pay one extra
        // memcpy per emitted frame (every 10 ms) instead of one
        // shift per sample (every 62 µs).
        let head = self.head;
        let tail_len = WINDOW_SAMPLES - head;
        fft_buf[..tail_len].copy_from_slice(&self.buf[head..]);
        fft_buf[tail_len..WINDOW_SAMPLES].copy_from_slice(&self.buf[..head]);
        self.hann.apply_to(&mut fft_buf[..WINDOW_SAMPLES]);

        // In-place real FFT. Returns N/2 complex bins; the
        // Nyquist bin's real part overwrites the DC imaginary
        // slot per `microfft`'s API convention.
        let spectrum = rfft_512(&mut fft_buf);

        let mut magnitude = [0.0_f32; FFT_BIN_COUNT];
        // microfft packs N/2 complex bins (indices 0..N/2). The
        // Nyquist bin's real part is stored in the imaginary slot
        // of `spectrum[0]`. We expand here to the canonical
        // N/2 + 1 layout the mel filterbank expects.
        magnitude[0] = spectrum[0].re.abs();
        for i in 1..FFT_SAMPLES / 2 {
            let c = spectrum[i];
            magnitude[i] = sqrtf(c.re * c.re + c.im * c.im);
        }
        magnitude[FFT_BIN_COUNT - 1] = spectrum[0].im.abs();

        let mel_energy = self.mel.apply(&magnitude);
        MelFrame {
            features: log_then_quantize(&mel_energy, self.quant),
        }
    }
}

impl Default for MelFrontend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests assert structural invariants; .expect / .unwrap are the standard test idiom"
)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    /// Collect all frames produced by feeding `samples` into a
    /// fresh frontend with default quantization.
    fn run(samples: &[i16]) -> Vec<MelFrame> {
        let mut fe = MelFrontend::new();
        let mut out = Vec::new();
        fe.push_samples(samples, |f| out.push(f));
        out
    }

    #[test]
    fn no_frame_before_window_fills() {
        let frames = run(&[0_i16; WINDOW_SAMPLES - 1]);
        assert!(frames.is_empty());
    }

    #[test]
    fn first_frame_emitted_exactly_at_window_fill() {
        let frames = run(&[0_i16; WINDOW_SAMPLES]);
        assert_eq!(frames.len(), 1, "expected one frame at window fill");
    }

    #[test]
    fn additional_frames_fire_every_hop_after_first() {
        // 480 samples → 1 frame; +160 → 2; +160 → 3; +160 → 4.
        let frames = run(&[0_i16; WINDOW_SAMPLES + 3 * HOP_SAMPLES]);
        assert_eq!(frames.len(), 4);
    }

    #[test]
    fn silent_input_produces_log_floor_frames() {
        // log(epsilon) quantized lands at -53 with default params
        // — every feature in the frame should equal that floor.
        let frames = run(&[0_i16; WINDOW_SAMPLES]);
        let f = frames[0];
        assert!(
            f.features.iter().all(|&x| x == -53),
            "expected uniform floor, got {:?}",
            &f.features[..6]
        );
    }

    #[test]
    fn pure_tone_concentrates_energy_in_expected_mel_bins() {
        // 1 kHz pure tone at moderate amplitude. The mel filter
        // covering ~1 kHz should hold significantly more energy
        // than filters far from the tone.
        let sr = SAMPLE_RATE_HZ as f32;
        let freq = 1000.0_f32;
        let amplitude: f32 = 10_000.0;
        let samples: Vec<i16> = (0..WINDOW_SAMPLES + 4 * HOP_SAMPLES)
            .map(|n| {
                let t = n as f32 / sr;
                let v = amplitude * libm::sinf(2.0 * core::f32::consts::PI * freq * t);
                v as i16
            })
            .collect();
        let frames = run(&samples);
        let f = frames.last().expect("expected at least one frame");

        // 1 kHz lands at mel ≈ 1000 mel; with the band running
        // hz_to_mel(125) ≈ 188.5 mel through hz_to_mel(7500) ≈
        // 2718.7 mel and 40 filters between, 1 kHz sits at
        // centre ≈ (1000 - 188.5) / ((2718.7 - 188.5) / 41) ≈
        // filter index 12-13. Allow a couple of bins of slack.
        let peak_idx = f
            .features
            .iter()
            .enumerate()
            .max_by_key(|&(_, &v)| v)
            .map(|(i, _)| i)
            .unwrap();
        assert!(
            (10..=15).contains(&peak_idx),
            "tone peak landed at feature[{peak_idx}], expected within 10..=15. \
             features={:?}",
            f.features,
        );
        // Peak should be at least ~15 quant steps above the
        // floor at the band edges (the silent log floor under
        // default params lands around -25 to -40 depending on
        // microphone noise; far-from-tone bins here include the
        // tone's spectral skirt and are noisier).
        let peak_value = f.features[peak_idx];
        let low_edge_max = f.features[..3].iter().copied().max().unwrap_or(i8::MIN);
        let high_edge_max = f.features[36..].iter().copied().max().unwrap_or(i8::MIN);
        let edges_max = low_edge_max.max(high_edge_max);
        assert!(
            peak_value > edges_max + 10,
            "tone energy did not stand out from band edges: \
             peak[{peak_idx}]={peak_value}, edges_max={edges_max}, \
             features={:?}",
            f.features,
        );
    }

    #[test]
    fn reset_clears_initial_fill_state() {
        let mut fe = MelFrontend::new();
        // Half-fill, then reset.
        fe.push_samples(&[0_i16; WINDOW_SAMPLES / 2], |_| {});
        fe.reset();
        // After reset, half a window of new samples should NOT
        // produce a frame — the half from before is discarded.
        let mut frames = 0;
        fe.push_samples(&[0_i16; WINDOW_SAMPLES / 2], |_| frames += 1);
        assert_eq!(frames, 0);
    }

    #[test]
    fn default_constructs_same_as_new() {
        // Both paths build the mel filterbank from the same const
        // configuration; two MelFrontends consuming identical input
        // must produce identical frames.
        let mut from_default = <MelFrontend as Default>::default();
        let mut from_new = MelFrontend::new();
        let silence = [0_i16; WINDOW_SAMPLES];
        let mut frames_d: Vec<MelFrame> = Vec::new();
        let mut frames_n: Vec<MelFrame> = Vec::new();
        from_default.push_samples(&silence, |f| frames_d.push(f));
        from_new.push_samples(&silence, |f| frames_n.push(f));
        assert_eq!(frames_d.len(), 1);
        assert_eq!(frames_n.len(), 1);
        assert_eq!(frames_d[0].features, frames_n[0].features);
    }
}
