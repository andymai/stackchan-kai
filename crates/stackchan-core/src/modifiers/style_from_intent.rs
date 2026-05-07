//! `StyleFromIntent`: translate `mind.intent` into face-style additions.
//!
//! Mirrors [`super::StyleFromEmotion`] in shape — `Phase::Expression`,
//! runs after `StyleFromEmotion` and before `Blink` so the canonical
//! `StyleFromEmotion → Blink` ordering still holds — but reads
//! `mind.intent` instead of `mind.affect.emotion`.
//!
//! Today it bumps `face.style.cheek_blush` when intent is
//! [`Intent::Petted`](crate::mind::Intent::Petted) so a sustained
//! pet visibly intensifies the blush regardless of which emotion is
//! active. Pure addition — `StyleFromEmotion` writes a fresh
//! `cheek_blush` baseline every tick (no persistent state to undo),
//! so this modifier just reads-then-writes with `saturating_add`.
//!
//! ## Per-intent additions
//!
//! | Intent        | `cheek_blush` add | Why                                       |
//! |---------------|-------------------|-------------------------------------------|
//! | `Idle`        |              `0`  | no override                               |
//! | `Listen`      |              `0`  | handled separately by `HeadFromAttention`        |
//! | `Startled` |              `0`  | `IntentFromLoud` writes `Surprised`, which |
//! |               |                   | `StyleFromEmotion` already renders            |
//! | `BeingPet`    |             `+30` | extra blush on top of any emotion base    |

use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::mind::Intent;
use crate::modifier::Modifier;

/// Cheek-blush bump added when `mind.intent` is
/// [`Intent::Petted`](crate::mind::Intent::Petted). Bumped on top
/// of whatever `StyleFromEmotion` set; saturates at `255`.
pub const PETTING_BLUSH_BUMP: u8 = 30;

/// Eye-curve bump added on `Intent::Petted` for the closed-eye smile.
///
/// Stacked on top of whatever `StyleFromEmotion` already set — so a
/// Happy + Petted combo curves the eyes into a bigger smile, while a
/// Sad + Petted combo softens the downward droop. Clamped to the
/// `i8` range.
pub const PETTING_EYE_CURVE_BUMP: i8 = 30;

/// Cap on `eye.open_weight` while `Intent::Petted` is active.
///
/// Sets the squint level for the closed-eye smile — eyes stay below
/// this even if `StyleFromEmotion` was holding them wide. Visibly
/// distinct from the always-closed `EyePhase::Closed` because Blink
/// continues to drive the open/close lifecycle.
pub const PETTING_OPEN_WEIGHT_CAP: u8 = 60;

/// Per-intent face-style additions.
///
/// Stateless — every tick reads the upstream `cheek_blush`
/// (`StyleFromEmotion` writes it fresh) and adds the per-intent bump.
#[derive(Debug, Clone, Copy, Default)]
pub struct StyleFromIntent;

impl StyleFromIntent {
    /// Construct.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Look up the per-intent cheek-blush addition.
///
/// `PickedUp` / `Shaken` / `Tilted` get their visible reaction from
/// [`super::EmotionFromIntent`] (emotion + autonomy hold), which then flows
/// through `StyleFromEmotion`. They contribute zero blush themselves.
const fn blush_for(intent: Intent) -> u8 {
    match intent {
        Intent::Petted => PETTING_BLUSH_BUMP,
        Intent::Idle
        | Intent::Listening
        | Intent::PickedUp
        | Intent::Shaken
        | Intent::Tilted
        | Intent::Startled => 0,
    }
}

/// Look up the per-intent eye-curve addition for the closed-eye smile
/// on `Petted`. Other intents leave `eye_curve` to upstream modifiers.
const fn eye_curve_bump_for(intent: Intent) -> i8 {
    match intent {
        Intent::Petted => PETTING_EYE_CURVE_BUMP,
        Intent::Idle
        | Intent::Listening
        | Intent::PickedUp
        | Intent::Shaken
        | Intent::Tilted
        | Intent::Startled => 0,
    }
}

/// Look up the per-intent `open_weight` cap. `None` = no clamp; only
/// `Petted` enforces a squint cap today.
const fn open_weight_cap_for(intent: Intent) -> Option<u8> {
    match intent {
        Intent::Petted => Some(PETTING_OPEN_WEIGHT_CAP),
        Intent::Idle
        | Intent::Listening
        | Intent::PickedUp
        | Intent::Shaken
        | Intent::Tilted
        | Intent::Startled => None,
    }
}

impl Modifier for StyleFromIntent {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "StyleFromIntent",
            description: "Translates mind.intent into additive face.style overrides. \
                          Petted adds +PETTING_BLUSH_BUMP to cheek_blush, +PETTING_EYE_CURVE_BUMP \
                          to eye_curve, and clamps both eyes' open_weight at \
                          PETTING_OPEN_WEIGHT_CAP for the closed-eye smile. Stateless — relies on \
                          StyleFromEmotion re-writing the baselines each tick.",
            phase: Phase::Expression,
            // Runs after `StyleFromEmotion` (priority -10) but before
            // `Blink` / `Breath` / `IdleDrift` (priority 0). Same
            // bracket the canonical-order test pins.
            priority: -5,
            reads: &[
                Field::Intent,
                Field::CheekBlush,
                Field::EyeCurve,
                Field::LeftEyeOpenWeight,
                Field::RightEyeOpenWeight,
            ],
            writes: &[
                Field::CheekBlush,
                Field::EyeCurve,
                Field::LeftEyeOpenWeight,
                Field::RightEyeOpenWeight,
            ],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let intent = entity.mind.intent;
        let bump = blush_for(intent);
        entity.face.style.cheek_blush = entity.face.style.cheek_blush.saturating_add(bump);

        let curve_bump = eye_curve_bump_for(intent);
        if curve_bump != 0 {
            entity.face.style.eye_curve = entity.face.style.eye_curve.saturating_add(curve_bump);
        }

        if let Some(cap) = open_weight_cap_for(intent) {
            // Min-with-existing: `Petted` only *narrows* the eyes
            // beyond what's already happening. A future intent that
            // wants the opposite would need a different field.
            entity.face.left_eye.open_weight = entity.face.left_eye.open_weight.min(cap);
            entity.face.right_eye.open_weight = entity.face.right_eye.open_weight.min(cap);
        }
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
        let mut m = StyleFromIntent::new();
        let mut entity = entity_with(Intent::Idle, 100);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 100);
    }

    #[test]
    fn listen_intent_does_not_change_blush() {
        let mut m = StyleFromIntent::new();
        let mut entity = entity_with(Intent::Listening, 100);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 100);
    }

    #[test]
    fn hearing_loud_intent_does_not_change_blush() {
        // IntentFromLoud writes Emotion::Surprised which gives the
        // visible reaction; this modifier stays out.
        let mut m = StyleFromIntent::new();
        let mut entity = entity_with(Intent::Startled, 100);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 100);
    }

    #[test]
    fn being_pet_adds_bump_on_top_of_upstream() {
        let mut m = StyleFromIntent::new();
        let mut entity = entity_with(Intent::Petted, 100);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 100 + PETTING_BLUSH_BUMP);
    }

    #[test]
    fn being_pet_saturates_at_max() {
        let mut m = StyleFromIntent::new();
        let mut entity = entity_with(Intent::Petted, 250);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 255);
    }

    #[test]
    fn intent_change_to_idle_returns_to_upstream_after_emotionstyle_rewrites() {
        let mut m = StyleFromIntent::new();
        let mut entity = entity_with(Intent::Petted, 100);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 130);

        // StyleFromEmotion re-writes the baseline each tick. Intent
        // flips to Idle — the bump goes away and we observe upstream.
        entity.face.style.cheek_blush = 70;
        entity.mind.intent = Intent::Idle;
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 70);
    }

    #[test]
    fn being_pet_curves_eyes_into_smile() {
        let mut m = StyleFromIntent::new();
        let mut entity = entity_with(Intent::Petted, 0);
        // Pretend StyleFromEmotion left eye_curve = 20 (slight smile).
        entity.face.style.eye_curve = 20;
        m.update(&mut entity);
        assert_eq!(
            entity.face.style.eye_curve,
            20 + PETTING_EYE_CURVE_BUMP,
            "Petted should add curve_bump on top of upstream"
        );
    }

    #[test]
    fn being_pet_caps_open_weight_for_closed_eye_smile() {
        let mut m = StyleFromIntent::new();
        let mut entity = entity_with(Intent::Petted, 0);
        // StyleFromEmotion holds eyes wide on Happy/Hi (open_weight = 100).
        entity.face.left_eye.open_weight = 100;
        entity.face.right_eye.open_weight = 100;
        m.update(&mut entity);
        assert_eq!(entity.face.left_eye.open_weight, PETTING_OPEN_WEIGHT_CAP);
        assert_eq!(entity.face.right_eye.open_weight, PETTING_OPEN_WEIGHT_CAP);
    }

    #[test]
    fn being_pet_does_not_open_eyes_above_their_existing_value() {
        // Sleepy emotion already droops open_weight to 55 — the cap of
        // 60 must not push that *up* to 60. Min-with-existing.
        let mut m = StyleFromIntent::new();
        let mut entity = entity_with(Intent::Petted, 0);
        entity.face.left_eye.open_weight = 55;
        entity.face.right_eye.open_weight = 55;
        m.update(&mut entity);
        assert_eq!(entity.face.left_eye.open_weight, 55);
        assert_eq!(entity.face.right_eye.open_weight, 55);
    }

    #[test]
    fn idle_intent_does_not_modify_eye_fields() {
        let mut m = StyleFromIntent::new();
        let mut entity = entity_with(Intent::Idle, 0);
        entity.face.style.eye_curve = 20;
        entity.face.left_eye.open_weight = 100;
        m.update(&mut entity);
        assert_eq!(entity.face.style.eye_curve, 20);
        assert_eq!(entity.face.left_eye.open_weight, 100);
    }

    #[test]
    fn being_pet_eye_curve_saturates_on_overflow() {
        // Hi emotion + Petted: eye_curve already at 50; +30 = 80,
        // well below i8::MAX. But pin saturating behaviour anyway so
        // a future emotion with eye_curve = 110 doesn't wrap.
        let mut m = StyleFromIntent::new();
        let mut entity = entity_with(Intent::Petted, 0);
        entity.face.style.eye_curve = 110;
        m.update(&mut entity);
        assert_eq!(entity.face.style.eye_curve, i8::MAX);
    }

    #[test]
    fn sustained_being_pet_keeps_bump_stable_across_ticks() {
        let mut m = StyleFromIntent::new();
        let mut entity = entity_with(Intent::Petted, 100);
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 130);

        // StyleFromEmotion re-writes the baseline (the same value, since
        // emotion is unchanged). Bump should add fresh again.
        entity.face.style.cheek_blush = 100;
        m.update(&mut entity);
        assert_eq!(entity.face.style.cheek_blush, 130);
    }
}
