//! [`DecoratorExpiry`] — runs first in [`Phase::Decoration`] and clears
//! `face.decorator` once its `expires_at` deadline passes.
//!
//! Splitting expiry into its own modifier keeps the trigger modifiers
//! stateless on the time axis: each trigger only writes a fresh
//! [`crate::decorator::DecoratorState`] when its condition fires, and
//! never has to know whether the current decorator is "still alive."
//!
//! [`Phase::Decoration`]: crate::director::Phase::Decoration

use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::modifier::Modifier;

/// Stateless modifier that clears expired decorators.
///
/// Runs at priority `-10` in [`Phase::Decoration`] so trigger
/// modifiers (priority `0+`) see a clean slate before deciding whether
/// to re-arm a decorator.
///
/// [`Phase::Decoration`]: crate::director::Phase::Decoration
#[derive(Debug, Default, Clone, Copy)]
pub struct DecoratorExpiry;

impl DecoratorExpiry {
    /// Construct.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Modifier for DecoratorExpiry {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "DecoratorExpiry",
            description: "Clears face.decorator once expires_at passes; runs first in \
                          Phase::Decoration so trigger modifiers see a clean slate.",
            phase: Phase::Decoration,
            priority: -10,
            reads: &[Field::Decorator],
            writes: &[Field::Decorator],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        if let Some(state) = entity.face.decorator
            && state.is_expired(entity.tick.now)
        {
            entity.face.decorator = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Instant;
    use crate::decorator::{Decorator, DecoratorState};

    #[test]
    fn clears_decorator_on_or_after_deadline() {
        let mut entity = Entity::default();
        entity.face.decorator = Some(DecoratorState {
            kind: Decorator::Heart,
            expires_at: Instant::from_millis(1_000),
        });
        let mut m = DecoratorExpiry::new();

        // Before deadline — held.
        entity.tick.now = Instant::from_millis(999);
        m.update(&mut entity);
        assert!(entity.face.decorator.is_some());

        // At deadline — cleared.
        entity.tick.now = Instant::from_millis(1_000);
        m.update(&mut entity);
        assert!(entity.face.decorator.is_none());
    }

    #[test]
    fn noop_on_empty_decorator() {
        let mut entity = Entity::default();
        let mut m = DecoratorExpiry::new();
        entity.tick.now = Instant::from_millis(99_999);
        m.update(&mut entity);
        assert!(entity.face.decorator.is_none());
    }
}
