//! [`BubbleExpiry`] — runs first in [`Phase::Decoration`] and clears
//! `face.bubble` once its `expires_at` deadline passes.
//!
//! Mirrors [`super::DecoratorExpiry`] for the speech-bubble layer:
//! splitting expiry into its own modifier keeps trigger paths
//! stateless on the time axis. Each trigger only writes a fresh
//! [`crate::bubble::BubbleState`] when its condition fires.
//!
//! [`Phase::Decoration`]: crate::director::Phase::Decoration

use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::modifier::Modifier;

/// Stateless modifier that clears expired bubbles.
///
/// Runs at priority `-10` in [`Phase::Decoration`] alongside
/// [`super::DecoratorExpiry`] so future trigger modifiers see a
/// clean slate before re-arming.
///
/// [`Phase::Decoration`]: crate::director::Phase::Decoration
#[derive(Debug, Default, Clone, Copy)]
pub struct BubbleExpiry;

impl BubbleExpiry {
    /// Construct.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Modifier for BubbleExpiry {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "BubbleExpiry",
            description: "Clears face.bubble once expires_at passes; runs first in \
                          Phase::Decoration so trigger paths see a clean slate.",
            phase: Phase::Decoration,
            priority: -10,
            reads: &[Field::Bubble],
            writes: &[Field::Bubble],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        if let Some(state) = entity.face.bubble
            && state.is_expired(entity.tick.now)
        {
            entity.face.bubble = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bubble::BubbleState;
    use crate::clock::Instant;

    #[test]
    fn clears_bubble_on_or_after_deadline() {
        let mut entity = Entity::default();
        entity.face.bubble = Some(BubbleState {
            text: "hi",
            expires_at: Instant::from_millis(1_000),
        });
        let mut m = BubbleExpiry::new();

        // Before deadline — held.
        entity.tick.now = Instant::from_millis(999);
        m.update(&mut entity);
        assert!(entity.face.bubble.is_some());

        // At deadline — cleared.
        entity.tick.now = Instant::from_millis(1_000);
        m.update(&mut entity);
        assert!(entity.face.bubble.is_none());
    }

    #[test]
    fn noop_on_empty_bubble() {
        let mut entity = Entity::default();
        let mut m = BubbleExpiry::new();
        entity.tick.now = Instant::from_millis(99_999);
        m.update(&mut entity);
        assert!(entity.face.bubble.is_none());
    }
}
