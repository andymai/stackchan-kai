//! [`DecoratorFromThinking`] — arms the [`Decorator::Thinking`] overlay
//! while [`Attention::Thinking`] is active.
//!
//! Triggers across the window where the avatar is waiting on a sidecar
//! reply: the listen capture closed, audio was uploaded, and the
//! network round-trip is in flight. Refreshes the decorator's expiry
//! every frame so the thought bubble stays visible as long as Thinking
//! attention persists; lets the standard 500 ms tail (via
//! [`super::DecoratorExpiry`]) fade it out cleanly when the reply
//! lands or the hold expires.

use crate::decorator::{Decorator, DecoratorState};
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::mind::Attention;
use crate::modifier::Modifier;

/// Tail duration after Thinking releases. Mirrors
/// [`super::DecoratorFromListening`]'s tail so consecutive Listening →
/// Thinking → reply transitions share the same release cadence.
pub const DECORATOR_TAIL_MS: u64 = 500;

/// Stateless modifier that arms [`Decorator::Thinking`] while Thinking.
///
/// Mirrors [`super::DecoratorFromListening`]'s shape; the two are
/// mutually exclusive at the source ([`Attention`] holds one variant
/// at a time) so their priority order only matters relative to the
/// autonomous decorators (Heart, Sweat, Dizzy).
#[derive(Debug, Default, Clone, Copy)]
pub struct DecoratorFromThinking;

impl DecoratorFromThinking {
    /// Construct.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Modifier for DecoratorFromThinking {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "DecoratorFromThinking",
            description: "Arms face.decorator = Thinking while mind.attention is Thinking; \
                          refreshes expires_at each frame and lets DecoratorExpiry's tail \
                          handle the fade-out when Thinking releases.",
            phase: Phase::Decoration,
            // Priority 26 — runs after DecoratorFromListening (25)
            // and the autonomous triggers (Heart=0, Sweat=10,
            // Dizzy=20). Director sorts ascending and
            // `face.decorator` is a single Option, so last-write-wins:
            // an active thinking window beats Listening's Ear (mutually
            // exclusive at the attention source anyway) and both
            // autonomous decorators.
            priority: 26,
            reads: &[Field::Attention],
            writes: &[Field::Decorator],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        if matches!(entity.mind.attention, Attention::Thinking { .. }) {
            entity.face.decorator = Some(DecoratorState::hold_for(
                Decorator::Thinking,
                entity.tick.now,
                DECORATOR_TAIL_MS,
            ));
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::missing_docs_in_private_items,
    reason = "test-only: decorator is set by the modifier-under-test inside the same scope"
)]
mod tests {
    use super::*;
    use crate::clock::Instant;

    fn entity_with_attention(attention: Attention, now_ms: u64) -> Entity {
        let mut e = Entity::default();
        e.mind.attention = attention;
        e.tick.now = Instant::from_millis(now_ms);
        e
    }

    #[test]
    fn arms_thinking_while_thinking() {
        let mut entity = entity_with_attention(
            Attention::Thinking {
                since: Instant::from_millis(0),
            },
            33,
        );
        let mut m = DecoratorFromThinking::new();
        m.update(&mut entity);
        let state = entity.face.decorator.expect("Thinking should be armed");
        assert_eq!(state.kind, Decorator::Thinking);
        assert_eq!(
            state.expires_at,
            Instant::from_millis(33 + DECORATOR_TAIL_MS)
        );
    }

    #[test]
    fn does_not_arm_when_not_thinking() {
        let mut entity = entity_with_attention(Attention::None, 0);
        let mut m = DecoratorFromThinking::new();
        m.update(&mut entity);
        assert!(entity.face.decorator.is_none());
    }

    #[test]
    fn does_not_arm_for_listening_attention() {
        // Listening drives DecoratorFromListening; only Thinking gets
        // the thought-bubble overlay.
        let mut entity = entity_with_attention(
            Attention::Listening {
                since: Instant::from_millis(0),
            },
            33,
        );
        let mut m = DecoratorFromThinking::new();
        m.update(&mut entity);
        assert!(entity.face.decorator.is_none());
    }

    #[test]
    fn refreshes_expiry_each_frame() {
        let mut entity = entity_with_attention(
            Attention::Thinking {
                since: Instant::from_millis(0),
            },
            33,
        );
        let mut m = DecoratorFromThinking::new();
        m.update(&mut entity);
        let first = entity.face.decorator.expect("first arm").expires_at;

        entity.tick.now = Instant::from_millis(166);
        m.update(&mut entity);
        let second = entity.face.decorator.expect("re-arm").expires_at;
        assert!(
            second > first,
            "expiry must advance with `now` ({second:?} vs {first:?})"
        );
    }
}
