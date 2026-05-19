//! Hann window.
//!
//! `microWakeWord` and most TFLite-micro keyword-spotting frontends
//! use a 30 ms periodic Hann window. 30 ms at 16 kHz is 480
//! samples; the window coefficients are precomputed at construction
//! time and applied per-frame via `apply_to` rather than recomputed
//! in the hot path.

use libm::cosf;

use crate::frontend::WINDOW_SAMPLES;

/// Precomputed Hann coefficients, scaled to `f32` in `[0, 1]`.
///
/// `w[n] = 0.5 * (1 - cos(2π·n / N))` for `n ∈ 0..N`, the
/// **periodic** definition `microWakeWord` uses (matches
/// `numpy.hanning` with `sym=False` / `scipy.signal.windows.hann`
/// with `sym=False`). The symmetric form (`sym=True`, `N-1` in the
/// denominator) drops one coefficient to zero and shifts the
/// spectrum subtly differently; using the wrong variant is one of
/// the classic ways a hand-rolled frontend drifts off a reference
/// implementation by a few dB per bin.
pub struct HannWindow {
    /// Per-sample coefficients in `[0, 1]`. Length is fixed at
    /// [`WINDOW_SAMPLES`] so the array can be heap-free.
    coefficients: [f32; WINDOW_SAMPLES],
}

impl HannWindow {
    /// Construct a 480-sample periodic Hann window.
    #[must_use]
    pub fn new() -> Self {
        let mut coefficients = [0.0_f32; WINDOW_SAMPLES];
        let n_f = WINDOW_SAMPLES as f32;
        for (i, c) in coefficients.iter_mut().enumerate() {
            // Periodic Hann: 2π·i / N, NOT 2π·i / (N-1).
            let theta = 2.0 * core::f32::consts::PI * (i as f32) / n_f;
            *c = 0.5 * (1.0 - cosf(theta));
        }
        Self { coefficients }
    }

    /// Apply the window in place: `samples[i] *= coefficient[i]`.
    ///
    /// Length-checked via `iter_mut().zip(self.coefficients)` so a
    /// shorter slice silently uses fewer coefficients — but the
    /// caller invariant in [`crate::frontend::MelFrontend`] always
    /// passes exactly [`WINDOW_SAMPLES`] floats.
    pub fn apply_to(&self, samples: &mut [f32]) {
        for (s, &c) in samples.iter_mut().zip(self.coefficients.iter()) {
            *s *= c;
        }
    }

    /// Access the raw coefficients for inspection / tests.
    #[must_use]
    pub const fn coefficients(&self) -> &[f32; WINDOW_SAMPLES] {
        &self.coefficients
    }
}

impl Default for HannWindow {
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
    fn periodic_hann_starts_and_ends_correctly() {
        let w = HannWindow::new();
        let c = w.coefficients();
        // Periodic Hann: w[0] = 0 exactly; w[N/2] = 1 exactly.
        assert!(c[0].abs() < 1e-6);
        assert!((c[WINDOW_SAMPLES / 2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn periodic_hann_is_symmetric_around_n_over_2() {
        let w = HannWindow::new();
        let c = w.coefficients();
        // For periodic Hann, w[k] == w[N-k] for k ∈ 1..N/2.
        // (Periodic, not symmetric — the asymmetry is only at k=0
        // vs k=N, which are distinct under periodic indexing.)
        for k in 1..WINDOW_SAMPLES / 2 {
            assert!(
                (c[k] - c[WINDOW_SAMPLES - k]).abs() < 1e-6,
                "asymmetry at k={k}: {} vs {}",
                c[k],
                c[WINDOW_SAMPLES - k]
            );
        }
    }

    #[test]
    fn apply_to_zeros_passes_through_zeros() {
        let w = HannWindow::new();
        let mut buf = [0.0_f32; WINDOW_SAMPLES];
        w.apply_to(&mut buf);
        assert!(buf.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn apply_to_scales_constant_input() {
        let w = HannWindow::new();
        let mut buf = [1.0_f32; WINDOW_SAMPLES];
        w.apply_to(&mut buf);
        // Sum of periodic Hann coefficients over a full window is
        // N/2 (one full cosine cycle averages 0; the 0.5 constant
        // term contributes N/2). Verify the windowed sum matches.
        let sum: f32 = buf.iter().sum();
        let expected = (WINDOW_SAMPLES as f32) / 2.0;
        assert!(
            (sum - expected).abs() < 0.01,
            "windowed sum {sum} != expected {expected}"
        );
    }

    #[test]
    fn default_matches_new_constructor() {
        // Both paths run the same runtime cosine computation; any
        // divergence would mean a refactor changed one path without
        // updating the other. Use bit-exact equality (not a fuzzy
        // compare) since the constructors are deterministic.
        let from_default = <HannWindow as Default>::default();
        let from_new = HannWindow::new();
        let a = from_default.coefficients();
        let b = from_new.coefficients();
        for k in 0..WINDOW_SAMPLES {
            // Compare bit patterns — sidesteps clippy::float_cmp on
            // the strict f32 equality the assertion actually wants.
            assert_eq!(
                a[k].to_bits(),
                b[k].to_bits(),
                "coefficient {k} diverged: default={} new={}",
                a[k],
                b[k],
            );
        }
    }
}
