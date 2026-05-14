//! Log scaling + int8 quantization.
//!
//! The mel filterbank produces unbounded `f32` energies; `TFLite`
//! keyword-spotting models expect signed-8-bit features at the
//! input tensor. Two steps connect them:
//!
//! 1. Natural log scaling, with a small `epsilon` floor so the
//!    log of a silent band doesn't go to `-∞`.
//! 2. Asymmetric `int8` quantization with a model-specific
//!    `scale` and `zero_point`.
//!
//! Quantization parameters are model-specific by design —
//! `TFLite`'s `quantization_parameters` for the input tensor is the
//! source of truth. The defaults exposed by [`QuantParams::DEFAULT`]
//! match `microWakeWord`'s published "hey jarvis" model
//! (`scale = 0.5`, `zero_point = -25`); future models will ship
//! their own parameters.

use libm::logf;

use crate::frontend::MEL_BIN_COUNT;

/// Natural-log floor: any input below this is treated as if it
/// equalled this value, so the log doesn't produce `-∞` (or, on
/// `xtensa-esp32s3-none-elf` without HW FP, a quiet NaN).
const LOG_EPSILON: f32 = 1.0e-6;

/// Per-tensor asymmetric quantization parameters.
///
/// Maps a real value `x` to int8 via
/// `clamp(round(x / scale) + zero_point, -128, 127)`. Mirrors
/// `TFLite`'s reference quantization for `int8` activation tensors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuantParams {
    /// Real-world scale per quant step. Must be strictly positive.
    pub scale: f32,
    /// Quantized value that corresponds to real-world zero.
    pub zero_point: i8,
}

impl QuantParams {
    /// Match `microWakeWord`'s reference quantization on its
    /// shipped models. Plug in model-specific values via
    /// [`QuantParams::new`] when running against a custom net.
    pub const DEFAULT: Self = Self {
        scale: 0.5,
        zero_point: -25,
    };

    /// Construct with explicit `scale` and `zero_point`. No
    /// validation on `scale` — passing `0` will produce infinity
    /// at the divide; passing negative inverts the mapping.
    /// Callers read these from the model's
    /// `quantization_parameters` and pass them straight through.
    #[must_use]
    pub const fn new(scale: f32, zero_point: i8) -> Self {
        Self { scale, zero_point }
    }
}

impl Default for QuantParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Apply natural-log scaling and asymmetric int8 quantization to
/// one mel-energy frame.
///
/// `mel_energy[i]` is replaced by
/// `quantize(log(max(mel_energy[i], LOG_EPSILON)), params)`.
#[must_use]
pub fn log_then_quantize(
    mel_energy: &[f32; MEL_BIN_COUNT],
    params: QuantParams,
) -> [i8; MEL_BIN_COUNT] {
    let mut out = [0_i8; MEL_BIN_COUNT];
    for (i, &e) in mel_energy.iter().enumerate() {
        let floored = if e > LOG_EPSILON { e } else { LOG_EPSILON };
        out[i] = quantize_scalar(logf(floored), params);
    }
    out
}

/// Scalar `f32 → i8` quantization helper, factored out so tests
/// can pin down its rounding behaviour. Saturating on overflow.
#[must_use]
pub fn quantize_scalar(x: f32, params: QuantParams) -> i8 {
    // round-half-away-from-zero matches TFLite's reference kernel;
    // `f32::round` does what we want on host. The cast saturates
    // via clamp because raw `as i8` truncates wrap-around for
    // out-of-range values.
    let scaled = x / params.scale;
    let rounded = if scaled >= 0.0 {
        libm::floorf(scaled + 0.5)
    } else {
        libm::ceilf(scaled - 0.5)
    };
    let shifted = rounded + f32::from(params.zero_point);
    let clamped = if shifted < f32::from(i8::MIN) {
        f32::from(i8::MIN)
    } else if shifted > f32::from(i8::MAX) {
        f32::from(i8::MAX)
    } else {
        shifted
    };
    clamped as i8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_zero_maps_to_zero_point() {
        assert_eq!(
            quantize_scalar(0.0, QuantParams::new(0.5, -25)),
            -25,
            "real 0 should map to zero_point"
        );
    }

    #[test]
    fn quantize_one_scale_step_above_zero() {
        // x = 0.5, scale = 0.5 → 0.5/0.5 = 1 step → zero_point + 1.
        assert_eq!(quantize_scalar(0.5, QuantParams::new(0.5, -25)), -24);
    }

    #[test]
    fn quantize_saturates_at_int8_min_and_max() {
        // Large negative input -> i8::MIN, large positive -> i8::MAX.
        let p = QuantParams::new(0.5, 0);
        assert_eq!(quantize_scalar(-1000.0, p), i8::MIN);
        assert_eq!(quantize_scalar(1000.0, p), i8::MAX);
    }

    #[test]
    fn quantize_rounds_half_away_from_zero() {
        // round(0.5) → 1, round(-0.5) → -1.
        let p = QuantParams::new(1.0, 0);
        assert_eq!(quantize_scalar(0.5, p), 1);
        assert_eq!(quantize_scalar(-0.5, p), -1);
        // round(0.4) → 0, round(-0.4) → 0.
        assert_eq!(quantize_scalar(0.4, p), 0);
        assert_eq!(quantize_scalar(-0.4, p), 0);
    }

    #[test]
    fn log_then_quantize_zero_input_uses_epsilon_floor() {
        // log(1e-6) = ln(1e-6) ≈ -13.815. With scale=0.5,
        // zero_point=-25: -13.815/0.5 = -27.63 → round to -28,
        // plus zero_point -25 → -53.
        let zero = [0.0_f32; MEL_BIN_COUNT];
        let out = log_then_quantize(&zero, QuantParams::DEFAULT);
        assert!(
            out.iter().all(|&x| x == -53),
            "all-zero input did not produce expected floor value: got {:?}",
            &out[..4]
        );
    }

    #[test]
    fn log_then_quantize_unit_input_produces_log_of_one() {
        // log(1) = 0; with zero_point=-25 the output is -25.
        let unit = [1.0_f32; MEL_BIN_COUNT];
        let out = log_then_quantize(&unit, QuantParams::DEFAULT);
        assert!(out.iter().all(|&x| x == -25));
    }

    #[test]
    fn default_quant_params_match_documented_values() {
        let p = QuantParams::DEFAULT;
        assert!((p.scale - 0.5).abs() < f32::EPSILON);
        assert_eq!(p.zero_point, -25);
    }
}
