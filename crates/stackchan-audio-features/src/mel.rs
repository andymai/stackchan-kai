//! Mel filterbank.
//!
//! Triangular filters spanning the auditory mel scale, applied to
//! a magnitude spectrum produced by an FFT. The mel scale models
//! human pitch perception's roughly-logarithmic response above
//! ~1 kHz; speech recognition + keyword spotting front-ends use it
//! because the resulting features track perceived speech content
//! more closely than raw linear-frequency bins.
//!
//! Defaults match `microWakeWord`'s published frontend:
//! 40 channels, lower edge 125 Hz, upper edge 7500 Hz, 512-point
//! FFT at 16 kHz sample rate (giving 257 magnitude bins).
//!
//! # Reference implementation
//!
//! Slaney's auditory-toolbox mel scale is the de-facto standard
//! in audio ML — `librosa.filters.mel`,
//! `tensorflow.signal.linear_to_mel_weight_matrix`, and the
//! TFLite-micro `AudioFrontend` op all use it. The formulas
//! below match those reference implementations:
//!
//! ```text
//! hz_to_mel(f) = 2595 · log10(1 + f / 700)
//! mel_to_hz(m) = 700 · (10^(m / 2595) - 1)
//! ```

use alloc::boxed::Box;
use alloc::vec::Vec;

use libm::{log10f, powf};

use crate::frontend::{FFT_BIN_COUNT, MEL_BIN_COUNT, MEL_LOWER_HZ, MEL_UPPER_HZ, SAMPLE_RATE_HZ};

/// Convert frequency (Hz) to mel scale.
#[must_use]
pub fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * log10f(1.0 + hz / 700.0)
}

/// Convert mel scale to frequency (Hz).
#[must_use]
pub fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (powf(10.0, mel / 2595.0) - 1.0)
}

/// Precomputed mel filterbank.
///
/// Each row of `weights` is a triangular filter spanning a band of
/// FFT magnitude bins. Multiplying a spectrum by this matrix
/// collapses the 257 linear-frequency bins into [`MEL_BIN_COUNT`]
/// (40) perceptual bands.
///
/// Triangles overlap by design: each filter's peak sits at one mel
/// centre, with the left edge meeting the previous filter's centre
/// and the right edge meeting the next. The result is a partition
/// of unity on the inner mel range, with the lower/upper extremes
/// tapering to zero.
pub struct MelFilterbank {
    /// Filter weights, indexed `[mel_bin][fft_bin]`. Most entries
    /// are zero — each row's non-zero span is bounded by the
    /// adjacent mel centres — but a flat 2-D layout keeps the
    /// multiply hot loop branch-free.
    ///
    /// Heap-allocated (40 × 257 × 4 = ~41 KiB) because that's too
    /// large for an embassy task's default stack on the
    /// firmware target. PSRAM via `esp-alloc` is the host on
    /// device.
    weights: Box<[[f32; FFT_BIN_COUNT]]>,
}

impl MelFilterbank {
    /// Build the filterbank for the crate's fixed
    /// (`SAMPLE_RATE_HZ`, `WINDOW_SAMPLES`, `MEL_BIN_COUNT`,
    /// `MEL_LOWER_HZ`, `MEL_UPPER_HZ`) configuration.
    ///
    /// Built once per `MelFrontend` construction and held in the
    /// frontend; the inner-loop dot product is the only operation
    /// that runs per audio frame.
    #[must_use]
    pub fn new() -> Self {
        // Mel centres: MEL_BIN_COUNT + 2 evenly-spaced points on
        // the mel scale, then converted back to Hz. The +2 is for
        // the lower and upper edges of the band, which serve as
        // the left edge of the first triangle and the right edge
        // of the last.
        let mel_lower = hz_to_mel(MEL_LOWER_HZ);
        let mel_upper = hz_to_mel(MEL_UPPER_HZ);
        let mel_step = (mel_upper - mel_lower) / ((MEL_BIN_COUNT + 1) as f32);

        let mut centres_hz = [0.0_f32; MEL_BIN_COUNT + 2];
        for (i, c) in centres_hz.iter_mut().enumerate() {
            *c = mel_to_hz(mel_lower + (i as f32) * mel_step);
        }

        // FFT bin centres in Hz: `bin · sample_rate / fft_size`.
        // FFT_BIN_COUNT is N/2 + 1 = 257 for N = 512.
        let fft_size = 2 * (FFT_BIN_COUNT - 1);
        let bin_hz =
            |bin: usize| -> f32 { (bin as f32) * (SAMPLE_RATE_HZ as f32) / (fft_size as f32) };

        // Heap-allocate row by row so the 41 KiB matrix never
        // lives on the stack in full. Each push is one
        // 1028-byte row.
        let mut weights_vec: Vec<[f32; FFT_BIN_COUNT]> = Vec::with_capacity(MEL_BIN_COUNT);
        for m in 0..MEL_BIN_COUNT {
            let left = centres_hz[m];
            let centre = centres_hz[m + 1];
            let right = centres_hz[m + 2];
            let mut row = [0.0_f32; FFT_BIN_COUNT];
            for (bin, w) in row.iter_mut().enumerate() {
                let f = bin_hz(bin);
                *w = if f <= left || f >= right {
                    0.0
                } else if f <= centre {
                    (f - left) / (centre - left)
                } else {
                    (right - f) / (right - centre)
                };
            }
            weights_vec.push(row);
        }
        Self {
            weights: weights_vec.into_boxed_slice(),
        }
    }

    /// Apply the filterbank to a magnitude spectrum.
    ///
    /// `magnitude.len()` must equal [`FFT_BIN_COUNT`]; shorter
    /// inputs would produce undefined energy in the upper filters.
    /// The output `[f32; MEL_BIN_COUNT]` holds the summed weighted
    /// energy per band.
    #[must_use]
    pub fn apply(&self, magnitude: &[f32; FFT_BIN_COUNT]) -> [f32; MEL_BIN_COUNT] {
        let mut out = [0.0_f32; MEL_BIN_COUNT];
        for (m, out_bin) in out.iter_mut().enumerate() {
            let mut acc = 0.0_f32;
            for (w, &mag) in self.weights[m].iter().zip(magnitude.iter()) {
                acc += w * mag;
            }
            *out_bin = acc;
        }
        out
    }

    /// Access the raw matrix for inspection / tests.
    #[must_use]
    pub const fn weights(&self) -> &[[f32; FFT_BIN_COUNT]] {
        &self.weights
    }
}

impl Default for MelFilterbank {
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
    use super::*;

    #[test]
    fn hz_to_mel_roundtrips() {
        for &hz in &[125.0_f32, 1000.0, 2500.0, 5000.0, 7500.0] {
            let back = mel_to_hz(hz_to_mel(hz));
            assert!(
                (back - hz).abs() < 0.5,
                "round-trip failed for {hz}: got {back}"
            );
        }
    }

    #[test]
    fn lowest_filter_starts_at_mel_lower_edge() {
        let fb = MelFilterbank::new();
        let fft_size = 2 * (FFT_BIN_COUNT - 1);
        let bin_hz =
            |bin: usize| -> f32 { (bin as f32) * (SAMPLE_RATE_HZ as f32) / (fft_size as f32) };
        // First filter's leftmost non-zero bin sits at or above
        // MEL_LOWER_HZ — the triangle's left edge.
        let first_nonzero = fb.weights[0]
            .iter()
            .position(|&w| w > 0.0)
            .expect("first filter must have at least one non-zero bin");
        assert!(
            bin_hz(first_nonzero) >= MEL_LOWER_HZ - 1.0,
            "first non-zero bin {} ({}Hz) below MEL_LOWER_HZ {}",
            first_nonzero,
            bin_hz(first_nonzero),
            MEL_LOWER_HZ,
        );
    }

    #[test]
    fn highest_filter_ends_at_mel_upper_edge() {
        let fb = MelFilterbank::new();
        let fft_size = 2 * (FFT_BIN_COUNT - 1);
        let bin_hz =
            |bin: usize| -> f32 { (bin as f32) * (SAMPLE_RATE_HZ as f32) / (fft_size as f32) };
        let last = MEL_BIN_COUNT - 1;
        let last_nonzero = fb.weights[last]
            .iter()
            .rposition(|&w| w > 0.0)
            .expect("last filter must have at least one non-zero bin");
        // The rightmost non-zero bin sits at or below MEL_UPPER_HZ
        // — the triangle's right edge.
        assert!(
            bin_hz(last_nonzero) <= MEL_UPPER_HZ + 50.0,
            "last non-zero bin {} ({}Hz) above MEL_UPPER_HZ {}",
            last_nonzero,
            bin_hz(last_nonzero),
            MEL_UPPER_HZ,
        );
    }

    #[test]
    fn filter_peaks_sum_to_partition_of_unity_on_inner_band() {
        // Sum of all triangles at any one FFT bin in the inner
        // band (above the first centre, below the last centre)
        // approximates 1.0 — that's what "partition of unity"
        // means for adjacent triangular filters with shared edges.
        let fb = MelFilterbank::new();
        let fft_size = 2 * (FFT_BIN_COUNT - 1);
        let lower_centre_bin = (250.0 * fft_size as f32 / SAMPLE_RATE_HZ as f32) as usize;
        let upper_centre_bin = (6500.0 * fft_size as f32 / SAMPLE_RATE_HZ as f32) as usize;
        for bin in lower_centre_bin..upper_centre_bin {
            let total: f32 = fb.weights.iter().map(|row| row[bin]).sum();
            assert!(
                (total - 1.0).abs() < 0.01,
                "inner-band sum {total} != 1.0 at bin {bin}"
            );
        }
    }

    #[test]
    fn applies_to_zero_magnitude_produces_zero_energy() {
        let fb = MelFilterbank::new();
        let zero = [0.0_f32; FFT_BIN_COUNT];
        let out = fb.apply(&zero);
        assert!(out.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn applies_to_unit_spike_concentrates_energy_in_one_band() {
        // A magnitude spectrum with a single non-zero bin should
        // land its energy in at most two adjacent mel filters
        // (the bin sits inside two overlapping triangles).
        let fb = MelFilterbank::new();
        let mut mag = [0.0_f32; FFT_BIN_COUNT];
        let probe_bin = 32; // ~1 kHz at 16 kHz / 512.
        mag[probe_bin] = 1.0;
        let out = fb.apply(&mag);
        let nonzero = out.iter().filter(|&&x| x > 0.0).count();
        assert!(
            (1..=2).contains(&nonzero),
            "expected 1-2 non-zero mel bins, got {nonzero}: {out:?}"
        );
    }

    #[test]
    fn default_matches_new() {
        // Default::default() delegates to new(); this also asserts
        // new() is deterministic across two calls.
        let from_default = <MelFilterbank as Default>::default();
        let from_new = MelFilterbank::new();
        assert_eq!(from_default.weights().len(), from_new.weights().len());
        for (row, (d, n)) in from_default
            .weights()
            .iter()
            .zip(from_new.weights().iter())
            .enumerate()
        {
            for (col, (a, b)) in d.iter().zip(n.iter()).enumerate() {
                assert_eq!(
                    a.to_bits(),
                    b.to_bits(),
                    "weight diverged at row={row} col={col}: default={a} new={b}",
                );
            }
        }
    }

    #[test]
    fn weights_accessor_exposes_matrix_with_full_band_coverage() {
        let fb = MelFilterbank::new();
        let w = fb.weights();
        assert_eq!(w.len(), MEL_BIN_COUNT);
        // Every filter has at least one non-zero bin — otherwise the
        // band-edge construction has silently degenerated.
        for (m, row) in w.iter().enumerate() {
            assert!(
                row.iter().any(|&x| x > 0.0),
                "mel filter {m} has no non-zero weights",
            );
        }
    }
}
