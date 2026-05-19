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

    #[test]
    fn re_arm_after_intent_clears_and_returns_to_shaken() {
        // Two distinct shake events separated by a non-shaken frame
        // must each arm Dizzy with a fresh expiry. Otherwise the edge
        // detector would silently miss the second shake of a
        // double-shake gesture.
        let mut entity = Entity::default();
        let mut m = DecoratorFromShake::with_hold_ms(4_000);

        step(&mut m, &mut entity, Intent::Idle, 0);
        step(&mut m, &mut entity, Intent::Shaken, 33);
        let first = entity.face.decorator.expect("first shake arms").expires_at;

        // Intent clears (post-shake settle) so the edge detector
        // re-arms on the next Shaken frame.
        step(&mut m, &mut entity, Intent::Idle, 1_000);

        step(&mut m, &mut entity, Intent::Shaken, 5_000);
        let second = entity
            .face
            .decorator
            .expect("second shake re-arms")
            .expires_at;

        assert!(
            second > first,
            "second arm should produce a later expiry ({second:?} vs {first:?})"
        );
        assert_eq!(second, Instant::from_millis(5_000 + 4_000));
    }

    #[test]
    fn shake_overwrites_existing_decorator_on_edge() {
        // The edge handler unconditionally writes `face.decorator` —
        // a Heart from a recent petting moment is replaced by Dizzy
        // on a rising shake edge. Cross-modifier priority lives in
        // `meta.priority`, not in any conditional inside `update`.
        let mut entity = Entity::default();
        entity.face.decorator = Some(DecoratorState::hold_for(
            Decorator::Heart,
            Instant::from_millis(0),
            5_000,
        ));
        let mut m = DecoratorFromShake::new();
        step(&mut m, &mut entity, Intent::Idle, 0);
        step(&mut m, &mut entity, Intent::Shaken, 33);
        let state = entity.face.decorator.expect("decorator should be present");
        assert_eq!(
            state.kind,
            Decorator::Dizzy,
            "Dizzy must overwrite the prior Heart on the rising shake edge"
        );
        assert_eq!(
            state.expires_at,
            Instant::from_millis(33 + DIZZY_HOLD_MS),
            "expiry must be anchored to the shake edge, not inherited from the Heart it replaced"
        );
    }

    #[test]
    fn boot_directly_into_shaken_arms_on_first_tick() {
        // `last_intent` starts as `None`, so the very first frame
        // observed-as-Shaken is treated as a rising edge. Pins this
        // behavior: a device powered on while being shaken (e.g. in
        // a bag mid-transit) lands on Dizzy immediately rather than
        // waiting for a subsequent non-shaken frame to disambiguate
        // the edge.
        let mut entity = Entity::default();
        let mut m = DecoratorFromShake::new();
        step(&mut m, &mut entity, Intent::Shaken, 0);
        let state = entity.face.decorator.expect("boot-into-shaken arms Dizzy");
        assert_eq!(state.kind, Decorator::Dizzy);
        assert_eq!(state.expires_at, Instant::from_millis(DIZZY_HOLD_MS));
    }

    #[test]
    fn default_matches_new_hold() {
        let from_default = <DecoratorFromShake as Default>::default();
        let from_new = DecoratorFromShake::new();
        assert_eq!(from_default.hold_ms, from_new.hold_ms);
        assert_eq!(from_default.hold_ms, DIZZY_HOLD_MS);
    }

    #[test]
    fn meta_declares_decoration_phase_and_decorator_write() {
        let m = DecoratorFromShake::new();
        let meta = m.meta();
        assert_eq!(meta.name, "DecoratorFromShake");
        assert_eq!(meta.phase, Phase::Decoration);
        assert_eq!(meta.priority, 20);
        assert_eq!(meta.writes, &[Field::Decorator]);
        assert_eq!(meta.reads, &[Field::Intent]);
    }
}
