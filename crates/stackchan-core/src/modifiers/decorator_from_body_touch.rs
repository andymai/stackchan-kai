//! [`DecoratorFromBodyTouch`] — fires the [`Decorator::Heart`] overlay
//! when a positive emotion meets a back-of-head touch.
//!
//! Reads `entity.perception.body_touch` directly rather than depending
//! on the `Petting` skill's `Intent::BeingPet` so a brief tap (rather
//! than sustained petting) still earns the heart — the user reaction
//! we want is "anyone who pats Stack-chan gets a heart back," and
//! `Petting`'s sustain-counter is too slow for that interaction.

use crate::clock::Instant;
use crate::decorator::{Decorator, DecoratorState};
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::modifier::Modifier;

/// How long the Heart decorator is held after a triggering tap.
pub const HEART_HOLD_MS: u64 = 2_000;

/// Modifier that arms [`Decorator::Heart`] on body-touch + positive emotion.
#[derive(Debug, Clone, Copy)]
pub struct DecoratorFromBodyTouch {
    /// Hold duration in milliseconds. Configurable for tests.
    hold_ms: u64,
    /// Last `now` we observed `body_touch.any() == true`. Used so we
    /// only re-arm the heart on a rising edge, not on every frame the
    /// hand is still resting on the head.
    last_touched_at: Option<Instant>,
}

impl DecoratorFromBodyTouch {
    /// Construct with the default hold of [`HEART_HOLD_MS`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            hold_ms: HEART_HOLD_MS,
            last_touched_at: None,
        }
    }

    /// Construct with a custom hold duration. Test helper; firmware
    /// uses [`Self::new`].
    #[must_use]
    pub const fn with_hold_ms(hold_ms: u64) -> Self {
        Self {
            hold_ms,
            last_touched_at: None,
        }
    }
}

impl Default for DecoratorFromBodyTouch {
    fn default() -> Self {
        Self::new()
    }
}

/// Emotions that earn a heart on touch. Reactive variants
/// (`Sad`, `Surprised`, `Angry`, `Confused`, `Mad`, …) explicitly
/// exclude the heart so a hostile pat doesn't paint a contradicting
/// affection overlay.
const fn emotion_invites_heart(e: Emotion) -> bool {
    matches!(e, Emotion::Happy | Emotion::Loved | Emotion::Hi)
}

impl Modifier for DecoratorFromBodyTouch {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "DecoratorFromBodyTouch",
            description: "Arms face.decorator = Heart when body_touch.any() rises and the \
                          current emotion invites affection (Happy / Loved / Hi).",
            phase: Phase::Decoration,
            priority: 0,
            reads: &[Field::BodyTouch, Field::Emotion],
            writes: &[Field::Decorator],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let touched = entity.perception.body_touch.is_some_and(|t| t.any());

        // Detect rising edge: was untouched (or first frame), now is.
        // Without the rising-edge gate we'd refresh the expiry every
        // frame the hand is held, masking the natural fade-out.
        let rose = touched && self.last_touched_at.is_none();
        if touched {
            self.last_touched_at = Some(now);
        } else {
            self.last_touched_at = None;
        }

        if rose && emotion_invites_heart(entity.mind.affect.emotion) {
            entity.face.decorator = Some(DecoratorState::hold_for(
                Decorator::Heart,
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
    reason = "test-only: Option::unwrap on values just written by the \
              modifier-under-test"
)]
mod tests {
    use super::*;
    use crate::perception::BodyTouch;

    fn step(m: &mut DecoratorFromBodyTouch, entity: &mut Entity, ms: u64) {
        entity.tick.now = Instant::from_millis(ms);
        m.update(entity);
    }

    #[test]
    fn touch_on_happy_arms_heart() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Happy;
        entity.perception.body_touch = Some(BodyTouch {
            left: 0,
            centre: 2,
            right: 0,
        });
        let mut m = DecoratorFromBodyTouch::new();
        step(&mut m, &mut entity, 0);
        let state = entity.face.decorator.expect("heart should be armed");
        assert_eq!(state.kind, Decorator::Heart);
        assert_eq!(state.expires_at, Instant::from_millis(HEART_HOLD_MS));
    }

    #[test]
    fn touch_on_neutral_does_not_arm_heart() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Neutral;
        entity.perception.body_touch = Some(BodyTouch {
            left: 0,
            centre: 3,
            right: 0,
        });
        let mut m = DecoratorFromBodyTouch::new();
        step(&mut m, &mut entity, 0);
        assert!(entity.face.decorator.is_none());
    }

    #[test]
    fn sustained_touch_does_not_refresh_expiry() {
        // Without the rising-edge gate, a hand resting on the head
        // for 4 seconds would extend the heart indefinitely.
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Happy;
        entity.perception.body_touch = Some(BodyTouch {
            left: 0,
            centre: 2,
            right: 0,
        });
        let mut m = DecoratorFromBodyTouch::with_hold_ms(2_000);

        step(&mut m, &mut entity, 0);
        let first_expiry = entity.face.decorator.unwrap().expires_at;

        // Continued touch on subsequent frames must not re-arm.
        step(&mut m, &mut entity, 500);
        step(&mut m, &mut entity, 1_000);
        let still_first = entity.face.decorator.unwrap().expires_at;
        assert_eq!(first_expiry, still_first);
    }

    #[test]
    fn release_then_re_touch_re_arms() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Loved;
        let mut m = DecoratorFromBodyTouch::with_hold_ms(2_000);

        // Touch at t=0
        entity.perception.body_touch = Some(BodyTouch {
            left: 0,
            centre: 2,
            right: 0,
        });
        step(&mut m, &mut entity, 0);
        let first = entity.face.decorator.unwrap().expires_at;

        // Release at t=500
        entity.perception.body_touch = Some(BodyTouch::default());
        step(&mut m, &mut entity, 500);

        // Re-touch at t=1000 — fresh edge → fresh expiry.
        entity.perception.body_touch = Some(BodyTouch {
            left: 1,
            centre: 1,
            right: 0,
        });
        step(&mut m, &mut entity, 1_000);
        let second = entity.face.decorator.unwrap().expires_at;
        assert!(
            second > first,
            "re-armed expiry should be later than the original ({second:?} vs {first:?})"
        );
    }
}
