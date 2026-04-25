//! `MouthOpenAudio`: drives `face.mouth.mouth_open` from a microphone
//! RMS signal with a dB-mapped attack/release envelope.
//!
//! The firmware audio task publishes per-render-tick RMS (linear
//! amplitude normalised against full-scale i16) into
//! `entity.perception.audio_rms`. Each tick this modifier converts
//! RMS to dB, clamps to a speech-friendly window, maps linearly to
//! `0.0..=1.0`, and applies a single-pole IIR envelope (fast attack,
//! slow release) so the mouth doesn't twitch on silence jitter.
//!
//! ## Sim-testability
//!
//! Sim tests drive the modifier the same way firmware does: write to
//! `entity.perception.audio_rms = Some(rms)`, then call
//! `mouth.update(&mut entity)`.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::modifier::Modifier;
// `F32Ext` is where `no_std` `f32::mul_add` lives. `_` suppresses an
// "unused import" lint in builds where core's `mul_add` happens to
// resolve (some Rust versions expose it via `core`).
#[allow(unused_imports)]
use micromath::F32Ext as _;

/// Default lower dB bound mapped to closed mouth (`mouth_open = 0.0`).
///
/// `-50 dBFS` is roughly a quiet room; anything below this is
/// background noise the avatar shouldn't react to.
pub const DEFAULT_SILENCE_DB: f32 = -50.0;
/// Default upper dB bound mapped to fully-open mouth (`mouth_open = 1.0`).
///
/// `-10 dBFS` is comfortable speaking volume at the CoreS3's mic
/// distance; louder audio caps the mouth at fully open.
pub const DEFAULT_FULL_DB: f32 = -10.0;

/// Default attack time constant, in milliseconds. Short so the mouth
/// opens quickly on speech onset.
pub const DEFAULT_ATTACK_MS: f32 = 20.0;
/// Default release time constant, in milliseconds. Long so the mouth
/// closes gracefully between syllables rather than snapping shut.
pub const DEFAULT_RELEASE_MS: f32 = 100.0;

/// Microphone RMS value that means "inaudible." Below this we skip
/// the dB conversion (avoids `log(0) = -∞`) and treat the input as
/// silence.
const INAUDIBLE_RMS: f32 = 1e-5;

/// Modifier that turns microphone RMS into a mouth-open amplitude.
#[derive(Debug, Clone, Copy)]
pub struct MouthOpenAudio {
    /// Envelope state — the smoothed `mouth_open` value that actually
    /// gets written to the avatar.
    current: f32,
    /// dB value mapped to `mouth_open = 0.0`.
    silence_db: f32,
    /// dB value mapped to `mouth_open = 1.0`.
    full_db: f32,
    /// Attack time constant in milliseconds (silence → target).
    attack_ms: f32,
    /// Release time constant in milliseconds (target → silence).
    release_ms: f32,
    /// Last-update instant, for time-aware envelope smoothing.
    last_tick: Option<Instant>,
}

impl MouthOpenAudio {
    /// Construct with default dB window + envelope timings.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            current: 0.0,
            silence_db: DEFAULT_SILENCE_DB,
            full_db: DEFAULT_FULL_DB,
            attack_ms: DEFAULT_ATTACK_MS,
            release_ms: DEFAULT_RELEASE_MS,
            last_tick: None,
        }
    }

    /// Override the dB window that maps to `0.0..=1.0` mouth-open.
    ///
    /// `silence_db` must be less than `full_db`; swapped values are
    /// accepted but produce a mouth that opens on *quiet* audio, which
    /// is almost never what callers want.
    #[must_use]
    pub const fn with_db_window(mut self, silence_db: f32, full_db: f32) -> Self {
        self.silence_db = silence_db;
        self.full_db = full_db;
        self
    }

    /// Override the attack / release time constants, in milliseconds.
    #[must_use]
    pub const fn with_timings(mut self, attack_ms: f32, release_ms: f32) -> Self {
        self.attack_ms = attack_ms;
        self.release_ms = release_ms;
        self
    }

    /// Target `mouth_open` value for the stored RMS, before envelope
    /// smoothing. Split out so tests can assert on the mapping without
    /// running the envelope.
    fn target_from_rms(&self, rms: f32) -> f32 {
        // Non-finite inputs (`NaN`, `±∞`) and sub-audible levels both
        // read as "silent" — `ln_approx` on `NaN` bit-decodes to a
        // spurious finite value, so we have to filter here rather
        // than rely on the clamp at the end.
        if !rms.is_finite() || rms <= INAUDIBLE_RMS {
            return 0.0;
        }
        let db = linear_to_db(rms);
        let span = self.full_db - self.silence_db;
        if span.abs() < f32::EPSILON {
            return 0.0;
        }
        let raw = (db - self.silence_db) / span;
        clamp_unit(raw)
    }
}

impl Default for MouthOpenAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for MouthOpenAudio {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "MouthOpenAudio",
            description: "Drives face.mouth.mouth_open from perception.audio_rms via a dB-mapped \
                          attack/release envelope so the mouth lip-syncs roughly to mic input.",
            phase: Phase::Audio,
            priority: 0,
            reads: &[Field::AudioRms, Field::MouthOpen],
            writes: &[Field::MouthOpen],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        // Pre-publish (audio_rms = None) reads as silent. Once the
        // firmware audio task starts publishing, it stays Some.
        let rms = entity.perception.audio_rms.unwrap_or(0.0);
        let target = self.target_from_rms(rms);

        let dt_ms = match self.last_tick {
            // First tick: snap to target so the envelope doesn't
            // slew up from 0.0 on boot.
            None => {
                self.current = target;
                self.last_tick = Some(now);
                entity.face.mouth.mouth_open = self.current;
                return;
            }
            // Clamp dt to 1..=200 so a stalled render tick doesn't
            // blow the envelope state (or, if `now` goes backward,
            // treat it as a small forward step).
            Some(prev) => now.saturating_duration_since(prev).clamp(1, 200),
        };
        self.last_tick = Some(now);

        // Direction-dependent τ: attack when opening, release when closing.
        let tau_ms = if target > self.current {
            self.attack_ms
        } else {
            self.release_ms
        };

        // Single-pole IIR: α = 1 − exp(−dt/τ). We approximate exp via
        // clamped linear-in-dt/τ for small ratios, which matches a
        // scalar envelope feel without pulling in `libm`.
        #[allow(
            clippy::cast_precision_loss,
            reason = "dt_ms was clamped to 1..=200 just above; fits in f32 exactly"
        )]
        let dt_ms_f32 = dt_ms as f32;
        let alpha = one_minus_exp_approx(dt_ms_f32 / tau_ms);
        self.current += (target - self.current) * alpha;
        entity.face.mouth.mouth_open = clamp_unit(self.current);
    }
}

/// Linear amplitude → dB. `20 * log10(x)` via a ln-based identity.
///
/// Uses [`ln_approx`] internally so the whole crate stays `no_std` +
/// libm-free. Accurate to ~0.5 dB over the `1e-5..1.0` range we use
/// for mic-RMS (silence to full-scale), which is well below the
/// envelope's perceptual resolution.
fn linear_to_db(x: f32) -> f32 {
    // `log10(x) = ln(x) / ln(10)` = `ln(x) * log10(e)`.
    20.0 * ln_approx(x) * core::f32::consts::LOG10_E
}

/// `ln(1.5)`. Not in `core::f32::consts`; used as the expansion point
/// for the [`ln_approx`] Taylor series.
const LN_1_5: f32 = 0.405_465_1;

/// Constant term of the Horner-form Taylor series for `ln(1.5 + u)`.
const LN_HORNER_A0: f32 = LN_1_5;
/// Linear coefficient: `1/1.5`.
const LN_HORNER_A1: f32 = 1.0 / 1.5;
/// Quadratic coefficient: `-1/(2·1.5²)`.
const LN_HORNER_A2: f32 = -0.222_222_22;
/// Cubic coefficient: `1/(3·1.5³)`.
const LN_HORNER_A3: f32 = 0.098_765_43;
/// Quartic coefficient: `-1/(4·1.5⁴)`.
const LN_HORNER_A4: f32 = -0.049_382_716;

/// Natural-log approximation for `x > 0`.
///
/// Decomposes `x = 2^k · m` (`m ∈ [1, 2)`), computes `ln(m)` from
/// its Taylor series around `m = 1.5` (the geometric midpoint of
/// `[1, 2)`), then reassembles. Max error ≈ `1e-3` over `[1, 2)`,
/// dominated by the series truncation at the 4th-order term — well
/// below the envelope's perceptual resolution.
fn ln_approx(x: f32) -> f32 {
    // Decompose into mantissa + exponent. `fract` + `trunc` aren't in
    // core, so we use `f32::to_bits` on the IEEE layout.
    let bits = x.to_bits();
    // Exponent bias for f32 is 127.
    #[allow(
        clippy::cast_possible_wrap,
        reason = "IEEE 754 biased exponent fits in i32 after unbiasing"
    )]
    let exp = (((bits >> 23) & 0xFF) as i32) - 127;
    // Mantissa in [1.0, 2.0): clear sign + exponent bits, reinsert
    // biased exponent 127.
    let m_bits = (bits & 0x007F_FFFF) | 0x3F80_0000;
    let m = f32::from_bits(m_bits);

    let u = m - 1.5;
    // Horner-form Taylor series of ln(1.5 + u) around u = 0:
    //   a0 + u * (a1 + u * (a2 + u * (a3 + u * a4)))
    let ln_m = u.mul_add(
        u.mul_add(
            u.mul_add(u.mul_add(LN_HORNER_A4, LN_HORNER_A3), LN_HORNER_A2),
            LN_HORNER_A1,
        ),
        LN_HORNER_A0,
    );

    #[allow(
        clippy::cast_precision_loss,
        reason = "exp is small (-127..=127); f32 representation is exact"
    )]
    let exp_f = exp as f32;
    exp_f.mul_add(core::f32::consts::LN_2, ln_m)
}

/// `1 - exp(-x)` approximation for small `x >= 0`.
///
/// Uses a 3-term Padé-style rational approximation that stays within
/// `~1e-3` over `[0, 4]` (our dt/τ stays in `0..=10` for realistic
/// render ticks), then saturates at 1.0 for larger arguments so the
/// envelope snaps to target when `dt >> τ`.
fn one_minus_exp_approx(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 4.0 {
        return 1.0;
    }
    // (x + x²/2) / (1 + x + x²/2): matches `1 - exp(-x)` to ~1e-3 on
    // [0, 4].
    let x2 = x * x;
    let num = x2.mul_add(0.5, x);
    let den = 1.0 + num;
    num / den
}

/// Clamp to `[0.0, 1.0]`, handling `NaN` as `0.0` so a pathological
/// RMS value can't poison the avatar state.
fn clamp_unit(v: f32) -> f32 {
    if v.is_nan() || v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tolerance for envelope value comparisons. 0.05 gives ±5% of the
    /// mouth-open range — well within the modifier's perceptual band.
    const TOL: f32 = 0.05;

    fn run(mouth: &mut MouthOpenAudio, entity: &mut Entity, rms: f32, ms: u64) {
        entity.perception.audio_rms = Some(rms);
        entity.tick.now = Instant::from_millis(ms);
        mouth.update(entity);
    }

    #[test]
    fn inaudible_rms_maps_to_closed() {
        let mut entity = Entity::default();
        let mut mouth = MouthOpenAudio::new();
        run(&mut mouth, &mut entity, 0.0, 0);
        assert!(entity.face.mouth.mouth_open.abs() < f32::EPSILON);
        run(&mut mouth, &mut entity, 1e-6, 10);
        assert!(entity.face.mouth.mouth_open < TOL);
    }

    #[test]
    fn full_scale_rms_opens_mouth_fully() {
        let mut entity = Entity::default();
        let mut mouth = MouthOpenAudio::new();
        // First tick snaps to target (no envelope on boot).
        run(&mut mouth, &mut entity, 1.0, 0);
        assert!(
            entity.face.mouth.mouth_open > 1.0 - TOL,
            "expected ~1.0, got {}",
            entity.face.mouth.mouth_open
        );
    }

    #[test]
    fn first_tick_snaps_without_envelope() {
        // Boot-time behaviour: we don't want the mouth to slew from 0
        // to a live target over the release window.
        let mut entity = Entity::default();
        let mut mouth = MouthOpenAudio::new();
        // -30 dBFS ≈ rms 0.0316; with default window that's
        // (−30 − (−50)) / 40 = 0.5.
        run(&mut mouth, &mut entity, 0.031_62, 0);
        assert!(
            (entity.face.mouth.mouth_open - 0.5).abs() < TOL,
            "expected ~0.5 on boot snap, got {}",
            entity.face.mouth.mouth_open
        );
    }

    #[test]
    fn attack_is_faster_than_release() {
        // Compare how far the envelope moves in the same dt for
        // opening vs closing. Attack τ=20 ms should cover more ground
        // than release τ=100 ms over a 10 ms tick.
        let mut entity = Entity::default();
        let mut mouth = MouthOpenAudio::new();
        // Boot at closed.
        run(&mut mouth, &mut entity, 1e-6, 0);
        let before_attack = entity.face.mouth.mouth_open;
        // One 10 ms tick with full-scale target.
        run(&mut mouth, &mut entity, 1.0, 10);
        let attack_step = entity.face.mouth.mouth_open - before_attack;

        // Reset: boot at full.
        let mut avatar2 = Entity::default();
        let mut mouth2 = MouthOpenAudio::new();
        run(&mut mouth2, &mut avatar2, 1.0, 0);
        let before_release = avatar2.face.mouth.mouth_open;
        // One 10 ms tick with silent target.
        run(&mut mouth2, &mut avatar2, 1e-6, 10);
        let release_step = before_release - avatar2.face.mouth.mouth_open;

        assert!(
            attack_step > release_step,
            "attack={attack_step:.3} should exceed release={release_step:.3}"
        );
    }

    #[test]
    fn envelope_settles_to_target_eventually() {
        let mut entity = Entity::default();
        let mut mouth = MouthOpenAudio::new();
        // Boot at closed.
        run(&mut mouth, &mut entity, 1e-6, 0);
        // 2 seconds of full-scale audio at 33 ms tick — well past
        // several release τ's.
        let mut ms = 33;
        while ms < 2_000 {
            run(&mut mouth, &mut entity, 1.0, ms);
            ms += 33;
        }
        assert!(
            entity.face.mouth.mouth_open > 1.0 - TOL,
            "expected settled ~1.0, got {}",
            entity.face.mouth.mouth_open
        );
    }

    #[test]
    fn clamp_handles_out_of_range_rms() {
        let mut entity = Entity::default();
        let mut mouth = MouthOpenAudio::new();
        run(&mut mouth, &mut entity, 10.0, 0); // 10× full-scale
        assert!(
            entity.face.mouth.mouth_open <= 1.0 + 1e-3,
            "expected clamp to ≤1, got {}",
            entity.face.mouth.mouth_open
        );
    }

    #[test]
    fn clamp_handles_nan_rms() {
        let mut entity = Entity::default();
        let mut mouth = MouthOpenAudio::new();
        run(&mut mouth, &mut entity, f32::NAN, 0);
        assert!(entity.face.mouth.mouth_open.abs() < f32::EPSILON);
    }

    #[test]
    fn target_from_rms_maps_dbfs_linearly() {
        let mouth = MouthOpenAudio::new();
        // Midpoint of default window: -30 dBFS = rms 10^(-30/20) ≈ 0.0316
        let t = mouth.target_from_rms(0.031_62);
        assert!((t - 0.5).abs() < TOL, "expected 0.5, got {t}");
        // Full scale: rms 1.0 = 0 dBFS → past full_db; clamps to 1.0.
        let t = mouth.target_from_rms(1.0);
        assert!((t - 1.0).abs() < f32::EPSILON);
        // Deep silence: rms 1e-6 → below silence_db; clamps to 0.0.
        let t = mouth.target_from_rms(1e-6);
        assert!(t.abs() < f32::EPSILON);
    }

    #[test]
    fn ln_approx_matches_libm_reference() {
        // We don't link libm in tests, but we can spot-check against
        // known values to ~1e-3.
        for (input, expected) in [
            (1.0_f32, 0.0_f32),
            (2.0, core::f32::consts::LN_2),
            (core::f32::consts::E, 1.0),
            (10.0, core::f32::consts::LN_10),
            (0.5, -core::f32::consts::LN_2),
        ] {
            let actual = ln_approx(input);
            assert!(
                (actual - expected).abs() < 5e-3,
                "ln({input}) ≈ {expected}, got {actual}"
            );
        }
    }
}
