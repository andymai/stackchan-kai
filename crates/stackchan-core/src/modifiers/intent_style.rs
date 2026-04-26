//! `IntentStyle`: translate `mind.intent` into face-style additions.
//!
//! Mirrors [`super::EmotionStyle`] in shape — `Phase::Expression`,
//! runs after `EmotionStyle` and before `Blink` so the canonical
//! `EmotionStyle → Blink` ordering still holds — but reads
//! `mind.intent` instead of `mind.affect.emotion`.
//!
//! Today it bumps `face.style.cheek_blush` when intent is
//! [`Intent::BeingPet`](crate::mind::Intent::BeingPet) so a sustained
//! pet visibly intensifies the blush regardless of which emotion is
//! active. Pure addition — `EmotionStyle` writes a fresh
//! `cheek_blush` baseline every tick (no persistent state to undo),
//! so this modifier just reads-then-writes with `saturating_add`.
//!
//! ## Per-intent additions
//!
//! | Intent        | `cheek_blush` add | Why                                       |
//! |---------------|-------------------|-------------------------------------------|
//! | `Idle`        |              `0`  | no override                               |
//! | `Listen`      |              `0`  | handled separately by `ListenHead`        |
//! | `HearingLoud` |              `0`  | `StartleOnLoud` writes `Surprised`, which |
//! |               |                   | `EmotionStyle` already renders            |
//! | `BeingPet`    |             `+30` | extra blush on top of any emotion base    |

use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::mind::Intent;
use crate::modifier::Modifier;

/// Cheek-blush bump added when `mind.intent` is
/// [`Intent::BeingPet`](crate::mind::Intent::BeingPet). Bumped on top
/// of whatever `EmotionStyle` set; saturates at `255`.
pub const PETTING_BLUSH_BUMP: u8 = 30;

/// Per-intent face-style additions.
///
/// Stateless — every tick reads the upstream `cheek_blush`
/// (`EmotionStyle` writes it fresh) and adds the per-intent bump.
#[derive(Debug, Clone, Copy, Default)]
pub struct IntentStyle;

impl IntentStyle {
    /// Construct.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Look up the per-intent cheek-blush addition.
///
/// `PickedUp` / `Shaken` / `Tilted` get their visible reaction from
/// [`super::IntentReflex`] (emotion + autonomy hold), which then flows
/// through `EmotionStyle`. They contribute zero blush themselves.
const fn blush_for(intent: Intent) -> u8 {
    match intent {
        Intent::BeingPet => PETTING_BLUSH_BUMP,
        Intent::Idle
        | Intent::Listen
        | Intent::PickedUp
        | Intent::Shaken
        | Intent::Tilted
        | Intent::HearingLoud => 0,
    }
}

impl Modifier for IntentStyle {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "IntentStyle",
            description: "Translates mind.intent into additive face.style overrides. \
                          BeingPet adds +PETTING_BLUSH_BUMP to cheek_blush; other intents \
                          contribute 0. Stateless — relies on EmotionStyle re-writing the \
                          cheek_blush baseline each tick.",
            phase: Phase::Expression,
            // Runs after `EmotionStyle` (priority -10) but before
            // `Blink` / `Breath` / `IdleDrift` (priority 0). Same
            // bracket the canonical-order test pins.
            priority: -5,
            reads: &[Field::Intent, Field::CheekBlush],
            writes: &[Field::CheekBlush],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let bump = blush_for(entity.mind.intent);
        entity.face.style.cheek_blush = entity.face.style.cheek_blush.saturating_add(bump);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity_with(intent: Intent, base_blush: u8) -> Entity {
        let mut e = Entity::default();
        e.mind.intent = intent;
        e.face.style.cheek_blush = base_blush;
        e
    }

    #[test]
    fn idle_intent_does_not_change_blush() {
        let mut m = IntentStyle::new();
        let mut entity = entity_with(Intent::Idle, 100);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 100);
    }

    #[test]
    fn listen_intent_does_not_change_blush() {
        let mut m = IntentStyle::new();
        let mut entity = entity_with(Intent::Listen, 100);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 100);
    }

    #[test]
    fn hearing_loud_intent_does_not_change_blush() {
        // StartleOnLoud writes Emotion::Surprised which gives the
        // visible reaction; this modifier stays out.
        let mut m = IntentStyle::new();
        let mut entity = entity_with(Intent::HearingLoud, 100);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 100);
    }

    #[test]
    fn being_pet_adds_bump_on_top_of_upstream() {
        let mut m = IntentStyle::new();
        let mut entity = entity_with(Intent::BeingPet, 100);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 100 + PETTING_BLUSH_BUMP);
    }

    #[test]
    fn being_pet_saturates_at_max() {
        let mut m = IntentStyle::new();
        let mut entity = entity_with(Intent::BeingPet, 250);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 255);
    }

    #[test]
    fn intent_change_to_idle_returns_to_upstream_after_emotionstyle_rewrites() {
        let mut m = IntentStyle::new();
        let mut entity = entity_with(Intent::BeingPet, 100);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 130);

        // EmotionStyle re-writes the baseline each tick. Intent
        // flips to Idle — the bump goes away and we observe upstream.
        entity.face.style.cheek_blush = 70;
        entity.mind.intent = Intent::Idle;
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 70);
    }

    #[test]
    fn sustained_being_pet_keeps_bump_stable_across_ticks() {
        let mut m = IntentStyle::new();
        let mut entity = entity_with(Intent::BeingPet, 100);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 130);

        // EmotionStyle re-writes the baseline (the same value, since
        // emotion is unchanged). Bump should add fresh again.
        entity.face.style.cheek_blush = 100;
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 130);
    }
}
