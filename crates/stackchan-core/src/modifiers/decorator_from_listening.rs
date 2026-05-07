//! [`DecoratorFromListening`] — arms the [`Decorator::Ear`] overlay
//! while [`Attention::Listening`] is active.
//!
//! Triggers any time the avatar enters a listening state — operator
//! `POST /listen`, sustained-audio detection by the
//! [`crate::skills::Listening`] skill, or a future wake-word event.
//! Refreshes the decorator's expiry every frame so the ear icon
//! stays visible as long as Listening attention persists; lets the
//! standard 500 ms tail (via [`super::DecoratorExpiry`]) fade it
//! out cleanly when Listening releases.

use crate::decorator::{Decorator, DecoratorState};
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::mind::Attention;
use crate::modifier::Modifier;

/// Tail duration after Listening releases.
///
/// Each frame Listening is active, the decorator's `expires_at` gets
/// refreshed to `now + DECORATOR_TAIL_MS`. When Listening drops, the
/// decorator stays visible for this window before
/// [`super::DecoratorExpiry`] clears it — softens the visual
/// transition on every release.
pub const DECORATOR_TAIL_MS: u64 = 500;

/// Stateless modifier that arms [`Decorator::Ear`] while Listening.
///
/// Same trigger surface as the other decorator modifiers
/// (rising-edge-style, but here we re-arm every frame to keep the
/// ear visible across the whole window — simpler than tracking
/// last-frame state, since the decorator's `expires_at` is the
/// single source of truth for visibility).
#[derive(Debug, Default, Clone, Copy)]
pub struct DecoratorFromListening;

impl DecoratorFromListening {
    /// Construct.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Modifier for DecoratorFromListening {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "DecoratorFromListening",
            description: "Arms face.decorator = Ear while mind.attention is Listening; \
                          refreshes expires_at each frame and lets DecoratorExpiry's tail \
                          handle the fade-out when Listening releases.",
            phase: Phase::Decoration,
            // Priority 5 — runs after the other decorator triggers
            // (Heart=0, Sweat=10, Dizzy=20). Ear takes precedence
            // over Sweat / Dizzy for an active listen window because
            // the operator-driven listening signal is intentional;
            // those autonomous decorators can re-arm on the next
            // trigger.
            priority: 5,
            reads: &[Field::Attention],
            writes: &[Field::Decorator],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        if matches!(entity.mind.attention, Attention::Listening { .. }) {
            entity.face.decorator = Some(DecoratorState::hold_for(
                Decorator::Ear,
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
    fn arms_ear_while_listening() {
        let mut entity = entity_with_attention(
            Attention::Listening {
                since: Instant::from_millis(0),
            },
            33,
        );
        let mut m = DecoratorFromListening::new();
        m.update(&mut entity);
        let state = entity.face.decorator.expect("Ear should be armed");
        assert_eq!(state.kind, Decorator::Ear);
        assert_eq!(
            state.expires_at,
            Instant::from_millis(33 + DECORATOR_TAIL_MS)
        );
    }

    #[test]
    fn does_not_arm_when_not_listening() {
        let mut entity = entity_with_attention(Attention::None, 0);
        let mut m = DecoratorFromListening::new();
        m.update(&mut entity);
        assert!(entity.face.decorator.is_none());
    }

    #[test]
    fn does_not_arm_for_tracking_attention() {
        // Tracking is a different attention mode; only Listening
        // gets the Ear overlay.
        use crate::head::Pose;
        let mut entity = entity_with_attention(
            Attention::Tracking {
                target: Pose {
                    pan_deg: 0.0,
                    tilt_deg: 0.0,
                },
                since: Instant::from_millis(0),
            },
            33,
        );
        let mut m = DecoratorFromListening::new();
        m.update(&mut entity);
        assert!(entity.face.decorator.is_none());
    }

    #[test]
    fn refreshes_expiry_each_frame() {
        // Per-frame re-arm pattern: each tick Listening is active,
        // `expires_at` should advance to `now + tail`.
        let mut entity = entity_with_attention(
            Attention::Listening {
                since: Instant::from_millis(0),
            },
            33,
        );
        let mut m = DecoratorFromListening::new();
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
