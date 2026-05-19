//! [`DecoratorFromLoud`] — fires the [`Decorator::Sweat`] overlay when
//! a sudden loud sound coincides with a non-positive emotion.
//!
//! The trigger reuses the same RMS that [`crate::modifiers::IntentFromLoud`]
//! reads; the difference is that this modifier paints a decorator
//! rather than writing intent. Both can fire on the same frame
//! without conflict.

use crate::decorator::{Decorator, DecoratorState};
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::modifier::Modifier;

/// How long the Sweat decorator is held after a triggering loud spike.
pub const SWEAT_HOLD_MS: u64 = 3_000;

/// Default rising-edge RMS threshold that fires the sweat overlay.
/// Matches `IntentFromLoud`'s startle threshold so the two reactions
/// are visually correlated.
pub const SWEAT_RMS_THRESHOLD: f32 = 0.4;

/// Modifier that arms [`Decorator::Sweat`] on a loud rising edge while
/// the current emotion is non-positive (Sad / Surprised / Angry / Mad
/// / Confused).
#[derive(Debug, Clone, Copy)]
pub struct DecoratorFromLoud {
    /// Hold duration in ms.
    hold_ms: u64,
    /// Threshold RMS — sweat arms only on the rising edge across this.
    threshold: f32,
    /// `true` once we've seen a sample above [`Self::threshold`]; the
    /// next dip back below resets it. This keeps a sustained loud
    /// noise from re-arming on every frame.
    armed_above: bool,
}

impl DecoratorFromLoud {
    /// Construct with default hold + threshold.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            hold_ms: SWEAT_HOLD_MS,
            threshold: SWEAT_RMS_THRESHOLD,
            armed_above: false,
        }
    }

    /// Construct with a custom hold + threshold. Test helper.
    #[must_use]
    pub const fn with_params(hold_ms: u64, threshold: f32) -> Self {
        Self {
            hold_ms,
            threshold,
            armed_above: false,
        }
    }
}

impl Default for DecoratorFromLoud {
    fn default() -> Self {
        Self::new()
    }
}

/// Emotions that justify a sweat overlay. Positive emotions (Happy /
/// Loved / Hi / Curious) should *not* paint sweat even on a loud spike
/// — those reactions belong to `IntentFromLoud`'s startle path, which
/// would have flipped the emotion away from positive territory if the
/// spike was genuinely jarring.
const fn emotion_admits_sweat(e: Emotion) -> bool {
    matches!(
        e,
        Emotion::Sad | Emotion::Surprised | Emotion::Angry | Emotion::Mad | Emotion::Confused
    )
}

impl Modifier for DecoratorFromLoud {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "DecoratorFromLoud",
            description: "Arms face.decorator = Sweat on the rising edge of audio_rms across \
                          the threshold while emotion is non-positive (Sad / Surprised / \
                          Angry / Mad / Confused).",
            phase: Phase::Decoration,
            priority: 10,
            reads: &[Field::AudioRms, Field::Emotion],
            writes: &[Field::Decorator],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let rms = entity.perception.audio_rms.unwrap_or(0.0);
        let above = rms.is_finite() && rms > self.threshold;

        // Rising edge across the threshold.
        let rose = above && !self.armed_above;
        self.armed_above = above;

        if rose && emotion_admits_sweat(entity.mind.affect.emotion) {
            entity.face.decorator = Some(DecoratorState::hold_for(
                Decorator::Sweat,
                now,
                self.hold_ms,
            ));
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test-only: Option::unwrap and Option::expect on values just \
              assigned by the modifier-under-test"
)]
mod tests {
    use super::*;
    use crate::clock::Instant;

    fn step(m: &mut DecoratorFromLoud, entity: &mut Entity, rms: f32, ms: u64) {
        entity.perception.audio_rms = Some(rms);
        entity.tick.now = Instant::from_millis(ms);
        m.update(entity);
    }

    #[test]
    fn loud_on_sad_arms_sweat() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Sad;
        let mut m = DecoratorFromLoud::with_params(3_000, 0.4);
        step(&mut m, &mut entity, 0.5, 0);
        let state = entity.face.decorator.expect("sweat should arm");
        assert_eq!(state.kind, Decorator::Sweat);
        assert_eq!(state.expires_at, Instant::from_millis(3_000));
    }

    #[test]
    fn loud_on_happy_does_not_arm_sweat() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Happy;
        let mut m = DecoratorFromLoud::new();
        step(&mut m, &mut entity, 0.6, 0);
        assert!(entity.face.decorator.is_none());
    }

    #[test]
    fn sustained_loud_does_not_re_arm() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Surprised;
        let mut m = DecoratorFromLoud::with_params(3_000, 0.4);
        step(&mut m, &mut entity, 0.5, 0);
        let first = entity.face.decorator.unwrap().expires_at;

        // Continued loud audio — must not re-arm.
        step(&mut m, &mut entity, 0.7, 100);
        step(&mut m, &mut entity, 0.6, 200);
        let still_first = entity.face.decorator.unwrap().expires_at;
        assert_eq!(first, still_first);
    }

    #[test]
    fn quiet_then_loud_re_arms_sweat() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Angry;
        let mut m = DecoratorFromLoud::with_params(3_000, 0.4);
        step(&mut m, &mut entity, 0.5, 0);
        let first = entity.face.decorator.unwrap().expires_at;
        step(&mut m, &mut entity, 0.05, 500); // quiet — resets the edge
        step(&mut m, &mut entity, 0.7, 1_000); // loud again
        let second = entity.face.decorator.unwrap().expires_at;
        assert!(
            second > first,
            "fresh edge should produce later expiry ({second:?} vs {first:?})"
        );
    }

    #[test]
    fn nonfinite_rms_is_silent() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Sad;
        let mut m = DecoratorFromLoud::new();
        step(&mut m, &mut entity, f32::NAN, 0);
        assert!(entity.face.decorator.is_none());
        step(&mut m, &mut entity, f32::INFINITY, 100);
        assert!(entity.face.decorator.is_none());
    }

    #[test]
    fn every_emotion_variant_categorized_for_sweat() {
        // Pin the sweat allowlist exhaustively. Iterates
        // `Emotion::ALL` (the canonical variant slice, length-pinned
        // by `all_length_matches_variant_count` in `emotion.rs`) so
        // adding a new variant flows through both the iteration and
        // the exhaustive `match` below without a parallel-list to
        // maintain. The `match` provides compile-time exhaustiveness
        // (no wildcard) so a new variant forces a deliberate
        // categorize-or-deny decision instead of silently landing in
        // `emotion_admits_sweat`'s no-admit `matches!` bucket.
        for &variant in Emotion::ALL {
            let expected_admit = match variant {
                Emotion::Sad
                | Emotion::Surprised
                | Emotion::Angry
                | Emotion::Mad
                | Emotion::Confused => true,
                Emotion::Neutral
                | Emotion::Happy
                | Emotion::Sleepy
                | Emotion::Doubt
                | Emotion::Boring
                | Emotion::Hi
                | Emotion::Loved
                | Emotion::Curious => false,
            };
            assert_eq!(
                emotion_admits_sweat(variant),
                expected_admit,
                "Emotion::{variant:?} sweat-admit classification drifted"
            );
        }
    }

    #[test]
    fn loud_overwrites_existing_decorator_on_admit() {
        // Sweat is unconditional on admit — a Heart decorator from a
        // recent petting moment loses to a startling loud spike. Pins
        // last-write-wins on `face.decorator`; the priority order
        // between decorator modifiers is encoded in their `priority`
        // values, not in any conditional inside `update`.
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Sad;
        entity.face.decorator = Some(DecoratorState::hold_for(
            Decorator::Heart,
            Instant::from_millis(0),
            5_000,
        ));
        let mut m = DecoratorFromLoud::new();
        step(&mut m, &mut entity, 0.6, 33);
        let state = entity
            .face
            .decorator
            .expect("decorator should be present after admit");
        assert_eq!(
            state.kind,
            Decorator::Sweat,
            "Sweat must overwrite the prior Heart on admit"
        );
    }

    #[test]
    fn rising_edge_during_disallowed_emotion_locks_out_subsequent_admit() {
        // A loud spike that begins while Happy doesn't arm Sweat — the
        // rising edge happens and `armed_above` flips. If the emotion
        // then shifts to Sad while the loud sample is still above
        // threshold, no fresh edge is detected (the existing
        // implementation correlates the edge to a single audio event,
        // not to a per-frame emotion check). Pinning this so a future
        // "let emotion changes count as a fresh edge" PR has to opt
        // in deliberately rather than discovering the surprise later.
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Happy;
        let mut m = DecoratorFromLoud::with_params(3_000, 0.4);
        step(&mut m, &mut entity, 0.6, 0);
        assert!(
            entity.face.decorator.is_none(),
            "Happy gate suppresses arm on the rising edge"
        );

        entity.mind.affect.emotion = Emotion::Sad;
        step(&mut m, &mut entity, 0.6, 33);
        assert!(
            entity.face.decorator.is_none(),
            "mid-event emotion shift does not retroactively arm Sweat"
        );

        // Quiet resets the edge; the next rise (still Sad) arms.
        step(&mut m, &mut entity, 0.05, 200);
        step(&mut m, &mut entity, 0.6, 250);
        let state = entity
            .face
            .decorator
            .expect("post-quiet rising edge while Sad should arm");
        assert_eq!(state.kind, Decorator::Sweat);
    }

    #[test]
    fn default_matches_new() {
        let from_default = <DecoratorFromLoud as Default>::default();
        let from_new = DecoratorFromLoud::new();
        assert_eq!(from_default.armed_above, from_new.armed_above);
        assert_eq!(from_default.hold_ms, from_new.hold_ms);
        assert!((from_default.threshold - from_new.threshold).abs() < f32::EPSILON);
    }

    #[test]
    fn meta_declares_decoration_phase_and_decorator_write() {
        let m = DecoratorFromLoud::new();
        let meta = m.meta();
        assert_eq!(meta.name, "DecoratorFromLoud");
        assert_eq!(meta.phase, Phase::Decoration);
        assert_eq!(meta.priority, 10);
        assert_eq!(meta.writes, &[Field::Decorator]);
        assert_eq!(meta.reads, &[Field::AudioRms, Field::Emotion]);
    }
}
