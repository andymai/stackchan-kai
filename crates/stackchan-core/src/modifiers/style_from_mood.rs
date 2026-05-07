//! [`StyleFromMood`] — applies the operator-selected [`Mood`] as a
//! multiplier on top of the per-emotion style targets that
//! [`super::StyleFromEmotion`] already wrote.
//!
//! Runs at priority `-7` in [`Phase::Expression`] — after
//! `StyleFromEmotion` (`-10`) and `StyleFromIntent` (`-5`), but before
//! [`super::Blink`] / [`super::Breath`] (`0`) so the cadence modifiers
//! see the final mood-adjusted scale.
//!
//! ## Composition
//!
//! - `face.style.blink_rate_scale *= mood.blink_multiplier()`
//! - `face.style.breath_depth_scale *= mood.breath_multiplier()`
//!
//! Idle-drift amplitude is scaled by the modifier itself reading
//! `mood.drift_multiplier()` — the drift modifiers don't currently
//! carry a runtime scale, so a follow-up can plumb that through; for
//! now this modifier covers the two dominant cadence axes.
//!
//! [`Mood`]: crate::mood::Mood
//! [`Phase::Expression`]: crate::director::Phase::Expression

use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::modifier::Modifier;

/// Stateless modifier that scales the cadence-bearing style fields by
/// the active [`crate::Mood`] multiplier.
#[derive(Debug, Default, Clone, Copy)]
pub struct StyleFromMood;

impl StyleFromMood {
    /// Construct.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Multiply a `u8` scale field by an `f32` multiplier and clamp back
/// into `0..=255`. Saturating so a mood with an extreme multiplier
/// can't overflow a baseline that's already near the cap.
fn scale_u8(value: u8, multiplier: f32) -> u8 {
    if !multiplier.is_finite() || multiplier <= 0.0 {
        return 0;
    }
    let scaled = f32::from(value) * multiplier;
    if scaled <= 0.0 {
        0
    } else if scaled >= 255.0 {
        255
    } else {
        // `scaled` is in `[0, 255)` here; the cast is precision-safe.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rounded = scaled as u8;
        rounded
    }
}

impl Modifier for StyleFromMood {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "StyleFromMood",
            description: "Scales face.style.blink_rate_scale and breath_depth_scale by the \
                          active mood's multipliers. Runs after StyleFromEmotion / \
                          StyleFromIntent so it sees the final per-emotion + per-intent \
                          baseline.",
            phase: Phase::Expression,
            priority: -7,
            reads: &[Field::Mood, Field::BlinkRateScale, Field::BreathDepthScale],
            writes: &[Field::BlinkRateScale, Field::BreathDepthScale],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let mood = entity.mind.mood;
        entity.face.style.blink_rate_scale =
            scale_u8(entity.face.style.blink_rate_scale, mood.blink_multiplier());
        entity.face.style.breath_depth_scale = scale_u8(
            entity.face.style.breath_depth_scale,
            mood.breath_multiplier(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::face::SCALE_DEFAULT;
    use crate::mood::Mood;

    #[test]
    fn neutral_mood_passes_scales_through_unchanged() {
        let mut entity = Entity::default();
        entity.mind.mood = Mood::Neutral;
        entity.face.style.blink_rate_scale = SCALE_DEFAULT;
        entity.face.style.breath_depth_scale = SCALE_DEFAULT;
        let mut m = StyleFromMood::new();
        m.update(&mut entity);
        assert_eq!(entity.face.style.blink_rate_scale, SCALE_DEFAULT);
        assert_eq!(entity.face.style.breath_depth_scale, SCALE_DEFAULT);
    }

    #[test]
    fn playful_speeds_blink_above_baseline() {
        let mut entity = Entity::default();
        entity.mind.mood = Mood::Playful;
        entity.face.style.blink_rate_scale = SCALE_DEFAULT;
        let mut m = StyleFromMood::new();
        m.update(&mut entity);
        assert!(entity.face.style.blink_rate_scale > SCALE_DEFAULT);
    }

    #[test]
    fn calm_slows_blink_below_baseline() {
        let mut entity = Entity::default();
        entity.mind.mood = Mood::Calm;
        entity.face.style.blink_rate_scale = SCALE_DEFAULT;
        let mut m = StyleFromMood::new();
        m.update(&mut entity);
        assert!(entity.face.style.blink_rate_scale < SCALE_DEFAULT);
    }

    #[test]
    fn calm_deepens_breath_above_baseline() {
        let mut entity = Entity::default();
        entity.mind.mood = Mood::Calm;
        entity.face.style.breath_depth_scale = SCALE_DEFAULT;
        let mut m = StyleFromMood::new();
        m.update(&mut entity);
        assert!(entity.face.style.breath_depth_scale > SCALE_DEFAULT);
    }

    #[test]
    fn sleepy_drops_blink_far_below_baseline() {
        // Sleepy mood × Sleepy emotion is the slowest-blink combo —
        // pin that the multiplier actually compounds with the base.
        let mut entity = Entity::default();
        entity.mind.mood = Mood::Sleepy;
        // Pretend StyleFromEmotion already set Sleepy emotion's
        // blink_rate_scale = 48.
        entity.face.style.blink_rate_scale = 48;
        let mut m = StyleFromMood::new();
        m.update(&mut entity);
        // 48 × 0.4 = 19.2 → 19
        assert_eq!(entity.face.style.blink_rate_scale, 19);
    }

    #[test]
    fn scale_zero_input_stays_zero() {
        // `Surprised` emotion sets blink_rate_scale = 0 (eyes held
        // wide). Mood multiplier must respect that — otherwise a
        // Playful Surprised would unexpectedly start blinking.
        let mut entity = Entity::default();
        entity.mind.mood = Mood::Playful;
        entity.face.style.blink_rate_scale = 0;
        let mut m = StyleFromMood::new();
        m.update(&mut entity);
        assert_eq!(entity.face.style.blink_rate_scale, 0);
    }

    #[test]
    fn scale_u8_saturates_at_max() {
        // A baseline near the cap × a > 1.0 multiplier mustn't wrap.
        assert_eq!(scale_u8(200, 2.0), 255);
        assert_eq!(scale_u8(255, 1.5), 255);
    }

    #[test]
    fn scale_u8_handles_nonfinite_multiplier_as_zero() {
        // Defensive: a future caller passing NaN / ±inf / negative
        // must not poison the style fields. We pick the conservative
        // "drop everything" branch — the avatar reads as still rather
        // than chaotic on garbage input.
        assert_eq!(scale_u8(128, f32::NAN), 0);
        assert_eq!(scale_u8(128, f32::INFINITY), 0);
        assert_eq!(scale_u8(128, f32::NEG_INFINITY), 0);
        assert_eq!(scale_u8(128, -1.0), 0);
    }
}
