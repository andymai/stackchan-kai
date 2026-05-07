//! [`DecoratorFromEmotion`] — fires emotion-amplifying overlays on
//! the rising edge into the matching emotion. Maps:
//!
//! - [`crate::Emotion::Angry`] → [`Decorator::Angry`] (`#` vein-pop)
//! - [`crate::Emotion::Loved`] → [`Decorator::Shy`] (cheek hash marks)
//!
//! ## Why edge-triggered
//!
//! Same pattern as [`super::DecoratorFromShake`]: refresh on entry,
//! hold for [`DECORATOR_EMOTION_HOLD_MS`], then expire via
//! [`super::DecoratorExpiry`]. Sustained-emotion ticks past the
//! initial entry don't keep refreshing the hold — once the overlay
//! has played its beat, it drops out so the avatar's other
//! decorators (Sweat / Dizzy / Heart) can take over without competing.
//!
//! ## Phase + priority
//!
//! Runs in [`Phase::Decoration`] at priority `-5` — after
//! [`super::DecoratorExpiry`] (`-10`) clears any prior expired overlay,
//! and *before* every other trigger
//! ([`super::DecoratorFromBodyTouch`] at `0`,
//! [`super::DecoratorFromLoud`] at `10`,
//! [`super::DecoratorFromShake`] at `20`,
//! [`super::DecoratorFromListening`] at `25`). Within [`Phase::Decoration`],
//! later writes win, so an emotion-derived Angry overlay is overwritten
//! by any of the more-specific signal-driven triggers when their
//! condition is also true. A shake event that fires both
//! [`crate::mind::Intent::Shaken`] (→ Dizzy via
//! [`super::DecoratorFromShake`]) AND [`crate::Emotion::Angry`] (→
//! Angry here) ends up showing Dizzy, the more specific cue.

use crate::decorator::{Decorator, DecoratorState};
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::modifier::Modifier;

/// How long the emotion-derived decorator is held after the
/// emotion's rising edge.
///
/// `3_000` ms is short enough that the overlay reads as a beat tied
/// to the emotion entry rather than ambient state, and long enough
/// that the audience registers the cue before the emotion's own
/// style settle finishes.
pub const DECORATOR_EMOTION_HOLD_MS: u64 = 3_000;

/// Modifier that arms an emotion-amplifying decorator on the rising
/// edge into [`Emotion::Angry`] or [`Emotion::Loved`].
#[derive(Debug, Clone, Copy)]
pub struct DecoratorFromEmotion {
    /// Hold duration in ms.
    hold_ms: u64,
    /// Last frame's emotion. Edge-detector for the rising-edge into
    /// the supported variants.
    last_emotion: Option<Emotion>,
}

impl DecoratorFromEmotion {
    /// Construct with default hold ([`DECORATOR_EMOTION_HOLD_MS`]).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            hold_ms: DECORATOR_EMOTION_HOLD_MS,
            last_emotion: None,
        }
    }

    /// Construct with a custom hold duration. Test helper.
    #[must_use]
    pub const fn with_hold_ms(hold_ms: u64) -> Self {
        Self {
            hold_ms,
            last_emotion: None,
        }
    }

    /// Map a supported emotion to the matching decorator. Returns
    /// `None` for emotions that don't drive an overlay.
    const fn decorator_for(emotion: Emotion) -> Option<Decorator> {
        match emotion {
            Emotion::Angry => Some(Decorator::Angry),
            Emotion::Loved => Some(Decorator::Shy),
            _ => None,
        }
    }
}

impl Default for DecoratorFromEmotion {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for DecoratorFromEmotion {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "DecoratorFromEmotion",
            description: "Arms face.decorator from the rising edge into Emotion::Angry \
                          (→ Decorator::Angry) or Emotion::Loved (→ Decorator::Shy). \
                          Sustained emotion does not refresh the hold.",
            phase: Phase::Decoration,
            priority: -5,
            reads: &[Field::Emotion],
            writes: &[Field::Decorator],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let emotion = entity.mind.affect.emotion;
        let prev = self.last_emotion;
        self.last_emotion = Some(emotion);

        // Only fire on the rising edge into a mapped emotion.
        if prev == Some(emotion) {
            return;
        }
        if let Some(kind) = Self::decorator_for(emotion) {
            entity.face.decorator = Some(DecoratorState::hold_for(kind, now, self.hold_ms));
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test-only: Option::unwrap on values just written by the \
              modifier-under-test"
)]
mod tests {
    use super::*;
    use crate::clock::Instant;

    fn step(m: &mut DecoratorFromEmotion, entity: &mut Entity, emotion: Emotion, ms: u64) {
        entity.mind.affect.emotion = emotion;
        entity.tick.now = Instant::from_millis(ms);
        m.update(entity);
    }

    #[test]
    fn angry_edge_arms_angry_decorator() {
        let mut entity = Entity::default();
        let mut m = DecoratorFromEmotion::new();
        step(&mut m, &mut entity, Emotion::Neutral, 0);
        assert!(entity.face.decorator.is_none());
        step(&mut m, &mut entity, Emotion::Angry, 33);
        let state = entity.face.decorator.expect("angry decorator should arm");
        assert_eq!(state.kind, Decorator::Angry);
        assert_eq!(
            state.expires_at,
            Instant::from_millis(33 + DECORATOR_EMOTION_HOLD_MS)
        );
    }

    #[test]
    fn loved_edge_arms_shy_decorator() {
        let mut entity = Entity::default();
        let mut m = DecoratorFromEmotion::new();
        step(&mut m, &mut entity, Emotion::Neutral, 0);
        step(&mut m, &mut entity, Emotion::Loved, 100);
        let state = entity.face.decorator.expect("shy decorator should arm");
        assert_eq!(state.kind, Decorator::Shy);
    }

    #[test]
    fn sustained_emotion_does_not_refresh_expiry() {
        let mut entity = Entity::default();
        let mut m = DecoratorFromEmotion::with_hold_ms(2_000);
        step(&mut m, &mut entity, Emotion::Neutral, 0);
        step(&mut m, &mut entity, Emotion::Angry, 33);
        let first = entity.face.decorator.unwrap().expires_at;
        // Repeated Angry frames must not bump the hold.
        step(&mut m, &mut entity, Emotion::Angry, 66);
        step(&mut m, &mut entity, Emotion::Angry, 99);
        let still_first = entity.face.decorator.unwrap().expires_at;
        assert_eq!(first, still_first);
    }

    #[test]
    fn unmapped_emotion_does_nothing() {
        let mut entity = Entity::default();
        let mut m = DecoratorFromEmotion::new();
        for emotion in [
            Emotion::Neutral,
            Emotion::Happy,
            Emotion::Sad,
            Emotion::Sleepy,
            Emotion::Surprised,
            Emotion::Doubt,
            Emotion::Boring,
            Emotion::Hi,
            Emotion::Curious,
            Emotion::Confused,
            Emotion::Mad,
        ] {
            step(&mut m, &mut entity, emotion, 0);
        }
        assert!(entity.face.decorator.is_none());
    }

    #[test]
    fn re_entry_to_angry_after_other_emotion_re_arms() {
        let mut entity = Entity::default();
        let mut m = DecoratorFromEmotion::new();
        step(&mut m, &mut entity, Emotion::Neutral, 0);
        step(&mut m, &mut entity, Emotion::Angry, 33);
        let first_expiry = entity.face.decorator.unwrap().expires_at;
        // Transition to a different emotion, then back to Angry: the
        // edge fires again and the new hold starts.
        step(&mut m, &mut entity, Emotion::Sad, 100);
        step(&mut m, &mut entity, Emotion::Angry, 200);
        let second_expiry = entity.face.decorator.unwrap().expires_at;
        assert!(second_expiry > first_expiry);
    }

    #[test]
    fn first_tick_in_unmapped_emotion_does_not_arm() {
        // Boot directly into Neutral (the default): the modifier's
        // first tick must NOT fire even though prev_emotion is None
        // and emotion is Neutral. Only mapped emotions trigger.
        let mut entity = Entity::default();
        let mut m = DecoratorFromEmotion::new();
        step(&mut m, &mut entity, Emotion::Neutral, 0);
        assert!(entity.face.decorator.is_none());
    }

    #[test]
    fn first_tick_already_in_angry_arms_decorator() {
        // Edge case: boot directly into Angry. prev_emotion is None
        // so the early-return doesn't trip; decorator_for(Angry) is
        // Some, so the overlay arms on the first tick.
        let mut entity = Entity::default();
        let mut m = DecoratorFromEmotion::new();
        step(&mut m, &mut entity, Emotion::Angry, 0);
        let state = entity.face.decorator.expect("angry decorator should arm");
        assert_eq!(state.kind, Decorator::Angry);
    }
}
