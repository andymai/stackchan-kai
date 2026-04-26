//! Breath modifier: gently shifts all features up and down on a
//! sine-like cycle.
//!
//! A triangle-wave approximation keeps the core crate `no_std` and
//! `libm`-free. The default period is ~6 s with a 2-pixel peak-to-peak
//! amplitude, which reads as subtle breathing at 30 FPS.
//!
//! The amplitude is scaled per-tick by
//! `entity.face.style.breath_depth_scale` so emotion-driven modifiers
//! (Sleepy → deeper, Surprised → near-flat) can modulate breathing
//! without owning Breath's state.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::face::SCALE_DEFAULT;
use crate::mind::Attention;
use crate::modifier::Modifier;

/// Default full breath cycle (inhale + exhale), in milliseconds.
const DEFAULT_CYCLE_MS: u64 = 6_000;
/// Default peak-to-peak vertical amplitude, in pixels.
const DEFAULT_AMPLITUDE_PX: i32 = 2;

/// Numerator of the cycle-period scale applied while
/// `mind.attention` is non-`None`. `5/3` ≈ ×1.67 stretch maps to ~0.6×
/// breath rate during engagement (animation-12-principles intuition;
/// person-holding-still-while-attentive cue).
pub const ENGAGED_BREATH_CYCLE_NUM: u64 = 5;
/// Denominator paired with [`ENGAGED_BREATH_CYCLE_NUM`].
pub const ENGAGED_BREATH_CYCLE_DEN: u64 = 3;

/// A modifier that applies a slow rise-and-fall vertical offset to every
/// facial feature (both eyes + mouth), evoking breathing.
#[derive(Debug, Clone, Copy)]
pub struct Breath {
    /// Milliseconds per complete breath cycle.
    cycle_ms: u64,
    /// Peak-to-peak amplitude in pixels at the baseline (scale = 128) depth.
    amplitude_px: i32,
    /// Offset applied on the previous tick; used to diff-and-undo so the
    /// modifier composes cleanly with modifiers that set absolute positions.
    last_offset_px: i32,
}

impl Breath {
    /// Default breath: 6 s cycle, 2 px amplitude.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cycle_ms: DEFAULT_CYCLE_MS,
            amplitude_px: DEFAULT_AMPLITUDE_PX,
            last_offset_px: 0,
        }
    }

    /// Construct a `Breath` with custom cycle and baseline amplitude.
    #[must_use]
    pub const fn with_params(cycle_ms: u64, amplitude_px: i32) -> Self {
        Self {
            cycle_ms,
            amplitude_px,
            last_offset_px: 0,
        }
    }

    /// Amplitude after applying `breath_depth_scale`. `SCALE_DEFAULT` (128)
    /// passes the baseline through unchanged.
    fn scaled_amplitude(&self, scale: u8) -> i32 {
        // Compute in i64 so a full-scale (255) amplitude doesn't overflow
        // before the final /128. Baseline amplitudes are a handful of px,
        // so truncation back to i32 is lossless in practice.
        let numerator = i64::from(self.amplitude_px).saturating_mul(i64::from(scale));
        #[allow(clippy::cast_possible_truncation)]
        let scaled = (numerator / i64::from(SCALE_DEFAULT)) as i32;
        scaled
    }

    /// Compute the current offset for time `now` as an integer-pixel triangle
    /// wave in `[-amplitude/2, +amplitude/2]` at the given scale, using
    /// `cycle_ms` for the period.
    fn offset_at(&self, now: Instant, scale: u8, cycle_ms: u64) -> i32 {
        let amplitude = self.scaled_amplitude(scale);
        if cycle_ms == 0 || amplitude == 0 {
            return 0;
        }
        let phase = now.as_millis() % cycle_ms;
        let half = cycle_ms / 2;
        // Ascend in the first half, descend in the second half.
        let ascending = phase < half;
        let within_half = if ascending { phase } else { phase - half };
        let half_i64 = i64::try_from(half).unwrap_or(i64::MAX);
        let within_i64 = i64::try_from(within_half).unwrap_or(i64::MAX);
        // Map within_half in 0..half to 0..amplitude linearly, then shift
        // down by half-amplitude so the wave oscillates around 0.
        let scaled = i64::from(amplitude) * within_i64 / half_i64.max(1);
        #[allow(clippy::cast_possible_truncation)]
        let progress = scaled as i32;
        if ascending {
            progress - amplitude / 2
        } else {
            amplitude / 2 - progress
        }
    }
}

impl Default for Breath {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for Breath {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "Breath",
            description: "Slow rise-and-fall vertical offset on every facial feature, evoking \
                          breathing. Scaled by face.style.breath_depth_scale.",
            phase: Phase::Expression,
            priority: 0,
            reads: &[
                Field::BreathDepthScale,
                Field::Attention,
                Field::LeftEyeCenter,
                Field::RightEyeCenter,
                Field::MouthCenter,
            ],
            writes: &[
                Field::LeftEyeCenter,
                Field::RightEyeCenter,
                Field::MouthCenter,
            ],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        // Slow the breath cycle while attention is engaged.
        let cycle_ms = if matches!(entity.mind.attention, Attention::None) {
            self.cycle_ms
        } else {
            self.cycle_ms.saturating_mul(ENGAGED_BREATH_CYCLE_NUM) / ENGAGED_BREATH_CYCLE_DEN
        };
        let target = self.offset_at(
            entity.tick.now,
            entity.face.style.breath_depth_scale,
            cycle_ms,
        );
        let delta = target - self.last_offset_px;
        if delta == 0 {
            return;
        }
        entity.face.left_eye.center.y += delta;
        entity.face.right_eye.center.y += delta;
        entity.face.mouth.center.y += delta;
        self.last_offset_px = target;
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::Entity;

    #[test]
    fn zero_amplitude_is_noop() {
        let mut entity = Entity::default();
        let baseline_y = entity.face.left_eye.center.y;
        let mut breath = Breath::with_params(1000, 0);
        for ms in 0..2000 {
            entity.tick.now = Instant::from_millis(ms);
            breath.update(&mut entity);
        }
        assert_eq!(entity.face.left_eye.center.y, baseline_y);
    }

    #[test]
    fn offset_stays_within_amplitude_at_default_scale() {
        let breath = Breath::with_params(1000, 4);
        // Sample across an entire cycle; offset must never exceed amplitude/2.
        for ms in 0..1000 {
            let o = breath.offset_at(Instant::from_millis(ms), SCALE_DEFAULT, 1000);
            assert!((-2..=2).contains(&o), "offset {o} at ms {ms} out of range");
        }
    }

    #[test]
    fn composes_across_ticks_without_drift() {
        let mut entity = Entity::default();
        let baseline_y = entity.face.left_eye.center.y;
        let mut breath = Breath::with_params(1000, 4);

        for ms in 0..=2000 {
            entity.tick.now = Instant::from_millis(ms);
            breath.update(&mut entity);
        }
        let final_y = entity.face.left_eye.center.y;
        let drift = (final_y - baseline_y).abs();
        assert!(drift <= 2, "drift {drift}px exceeds half-amplitude");
    }

    #[test]
    fn depth_scale_amplifies_or_attenuates() {
        let breath = Breath::with_params(1000, 4);
        let max_at = |scale: u8| {
            (0..1000)
                .map(|ms| {
                    breath
                        .offset_at(Instant::from_millis(ms), scale, 1000)
                        .abs()
                })
                .max()
                .unwrap_or(0)
        };
        let shallow = max_at(64);
        let baseline = max_at(SCALE_DEFAULT);
        let deep = max_at(255);

        assert!(shallow < baseline, "shallow={shallow}, baseline={baseline}");
        assert!(deep > baseline, "deep={deep}, baseline={baseline}");
    }

    #[test]
    fn depth_scale_zero_freezes_breath() {
        let mut entity = Entity::default();
        entity.face.style.breath_depth_scale = 0;
        let baseline_y = entity.face.left_eye.center.y;
        let mut breath = Breath::with_params(1000, 4);
        for ms in 0..=2000 {
            entity.tick.now = Instant::from_millis(ms);
            breath.update(&mut entity);
        }
        assert_eq!(entity.face.left_eye.center.y, baseline_y);
    }
}
