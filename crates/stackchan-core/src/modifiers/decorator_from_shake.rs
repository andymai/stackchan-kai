//! [`DecoratorFromShake`] — fires the [`Decorator::Dizzy`] overlay
//! whenever [`crate::skills::Handling`] flips intent into
//! [`crate::mind::Intent::Shaken`].
//!
//! Hooks the existing shake detection rather than rolling its own
//! accel-window analysis: `Handling` already does the heavy lifting,
//! and reusing its intent surface keeps the trigger correlated with
//! the rest of the shake reaction (Angry emotion via
//! `EmotionFromIntent`, head recoil via `HeadFromIntent`, etc.).

use crate::decorator::{Decorator, DecoratorState};
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::mind::Intent;
use crate::modifier::Modifier;

/// How long the Dizzy decorator is held after a shake event.
pub const DIZZY_HOLD_MS: u64 = 4_000;

/// Modifier that arms [`Decorator::Dizzy`] on the rising edge into
/// [`Intent::Shaken`].
#[derive(Debug, Clone, Copy)]
pub struct DecoratorFromShake {
    /// Hold duration in ms.
    hold_ms: u64,
    /// Last frame's intent — needed to detect the edge into
    /// [`Intent::Shaken`] so we don't refresh the dizzy hold every
    /// frame the shake intent persists.
    last_intent: Option<Intent>,
}

impl DecoratorFromShake {
    /// Construct with default hold ([`DIZZY_HOLD_MS`]).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            hold_ms: DIZZY_HOLD_MS,
            last_intent: None,
        }
    }

    /// Construct with a custom hold duration. Test helper.
    #[must_use]
    pub const fn with_hold_ms(hold_ms: u64) -> Self {
        Self {
            hold_ms,
            last_intent: None,
        }
    }
}

impl Default for DecoratorFromShake {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for DecoratorFromShake {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "DecoratorFromShake",
            description: "Arms face.decorator = Dizzy on the rising edge into Intent::Shaken.",
            phase: Phase::Decoration,
            priority: 20,
            reads: &[Field::Intent],
            writes: &[Field::Decorator],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let intent = entity.mind.intent;
        let was_shaken = matches!(self.last_intent, Some(Intent::Shaken));
        let is_shaken = matches!(intent, Intent::Shaken);
        self.last_intent = Some(intent);

        if is_shaken && !was_shaken {
            entity.face.decorator = Some(DecoratorState::hold_for(
                Decorator::Dizzy,
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
    use crate::clock::Instant;

    fn step(m: &mut DecoratorFromShake, entity: &mut Entity, intent: Intent, ms: u64) {
        entity.mind.intent = intent;
        entity.tick.now = Instant::from_millis(ms);
        m.update(entity);
    }

    #[test]
    fn shake_edge_arms_dizzy() {
        let mut entity = Entity::default();
        let mut m = DecoratorFromShake::new();
        step(&mut m, &mut entity, Intent::Idle, 0);
        assert!(entity.face.decorator.is_none());
        step(&mut m, &mut entity, Intent::Shaken, 33);
        let state = entity.face.decorator.expect("dizzy should arm");
        assert_eq!(state.kind, Decorator::Dizzy);
        assert_eq!(state.expires_at, Instant::from_millis(33 + DIZZY_HOLD_MS));
    }

    #[test]
    fn sustained_shaken_does_not_refresh_expiry() {
        let mut entity = Entity::default();
        let mut m = DecoratorFromShake::with_hold_ms(4_000);
        step(&mut m, &mut entity, Intent::Idle, 0);
        step(&mut m, &mut entity, Intent::Shaken, 33);
        let first = entity.face.decorator.unwrap().expires_at;
        // Shaken intent persists across multiple frames — must not re-arm.
        step(&mut m, &mut entity, Intent::Shaken, 66);
        step(&mut m, &mut entity, Intent::Shaken, 99);
        let still_first = entity.face.decorator.unwrap().expires_at;
        assert_eq!(first, still_first);
    }

    #[test]
    fn intent_other_than_shaken_does_nothing() {
        let mut entity = Entity::default();
        let mut m = DecoratorFromShake::new();
        for intent in [
            Intent::Idle,
            Intent::Listening,
            Intent::Tilted,
            Intent::PickedUp,
            Intent::Petted,
            Intent::Startled,
        ] {
            step(&mut m, &mut entity, intent, 0);
        }
        assert!(entity.face.decorator.is_none());
    }
}
