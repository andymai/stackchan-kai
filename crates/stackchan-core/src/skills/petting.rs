//! `Petting`: skill that sets `mind.intent = BeingPet` after sustained
//! contact on the back-of-head strip.
//!
//! Reads `entity.perception.body_touch` (intensity-aware, populated by
//! the firmware `body_touch` task). Counts consecutive ticks where any
//! zone has non-zero intensity; once the run reaches
//! [`PETTING_SUSTAIN_TICKS`] the skill writes
//! [`Intent::Petted`](crate::mind::Intent::Petted). On release
//! (any-zone-touched goes false) the intent clears back to
//! [`Intent::Idle`](crate::mind::Intent::Idle).
//!
//! ## Coexistence with `IntentFromBodyTouch`
//!
//! [`crate::modifiers::IntentFromBodyTouch`] reacts to the SAME perception
//! field but writes emotion + autonomy on the no-touch → touch rising
//! edge (Press), or on swipes. `Petting` writes intent only, after a
//! sustain. The two are complementary: a deliberate centre pet
//! triggers Press → Happy emotion immediately, AND after 1.5 s of
//! sustained contact, intent flips to `BeingPet`. Modifiers in later
//! phases (when added) can read `mind.intent` and add visual flourish
//! (extra blush, head bobs) on top of the emotion shift.
//!
//! ## Why a Skill, not a Modifier
//!
//! The output is `mind.intent` — a Mind field. Skills write Mind /
//! Voice / Events; modifiers translate Mind into Face / Motor. This
//! puts the sustain-detection logic on the right side of the
//! architectural split, and the [`Director::add_skill`] check enforces
//! the contract at registration time.
//!
//! [`Director::add_skill`]: crate::Director::add_skill

use crate::director::{Field, SkillMeta};
use crate::entity::Entity;
use crate::mind::Intent;
use crate::skill::{Skill, SkillStatus};

/// Consecutive any-zone-touched ticks required to enter `BeingPet`.
///
/// At the firmware's 50 ms body-touch poll cadence, 30 ticks ≈ 1.5 s
/// — long enough to ignore brushing past the strip, short enough that
/// a deliberate pet is responsive.
pub const PETTING_SUSTAIN_TICKS: u8 = 30;

/// Skill that detects sustained back-of-head contact. See module docs.
#[derive(Debug, Clone, Copy)]
pub struct Petting {
    /// Consecutive any-zone-touched ticks required to fire.
    pub sustain_ticks: u8,
    /// Running count of consecutive touched ticks. Reset on any
    /// not-touched tick. Saturates at `u8::MAX`.
    consecutive: u8,
}

impl Petting {
    /// Construct with the default sustain ([`PETTING_SUSTAIN_TICKS`]).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sustain_ticks: PETTING_SUSTAIN_TICKS,
            consecutive: 0,
        }
    }

    /// Construct with a custom sustain count.
    #[must_use]
    pub const fn with_sustain_ticks(sustain_ticks: u8) -> Self {
        Self {
            sustain_ticks,
            consecutive: 0,
        }
    }
}

impl Default for Petting {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for Petting {
    fn meta(&self) -> &'static SkillMeta {
        static META: SkillMeta = SkillMeta {
            name: "Petting",
            description: "Sustained back-of-head touch (any zone, ≥ PETTING_SUSTAIN_TICKS) sets \
                          mind.intent = BeingPet. Clears to Idle on release. Coexists with \
                          IntentFromBodyTouch (which writes emotion on the rising edge / on swipes).",
            priority: 50,
            writes: &[Field::Intent],
        };
        &META
    }

    fn should_fire(&self, _entity: &Entity) -> bool {
        // Always polled — the sustain counter needs to advance every
        // frame regardless of current intent state.
        true
    }

    fn invoke(&mut self, entity: &mut Entity) -> SkillStatus {
        let touched = entity.perception.body_touch.is_some_and(|t| t.any());

        if touched {
            self.consecutive = self.consecutive.saturating_add(1);
            if self.consecutive >= self.sustain_ticks {
                entity.mind.intent = Intent::Petted;
                return SkillStatus::Continuing;
            }
            return SkillStatus::Done;
        }

        // Released. Clear counter + intent (only if we set it).
        self.consecutive = 0;
        if matches!(entity.mind.intent, Intent::Petted) {
            entity.mind.intent = Intent::Idle;
        }
        SkillStatus::Done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Instant;
    use crate::perception::BodyTouch;

    fn step(skill: &mut Petting, entity: &mut Entity, ticks: u64) {
        let mut now = entity.tick.now;
        for _ in 0..ticks {
            now = now + 50;
            entity.tick.now = now;
            let _ = skill.invoke(entity);
        }
    }

    fn touched_entity(touch: BodyTouch) -> Entity {
        let mut e = Entity::default();
        e.perception.body_touch = Some(touch);
        e
    }

    #[test]
    fn no_perception_keeps_idle() {
        let mut skill = Petting::new();
        let mut entity = Entity::default();
        step(&mut skill, &mut entity, 100);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn brief_touch_below_sustain_does_not_fire() {
        let mut skill = Petting::new();
        let mut entity = touched_entity(BodyTouch {
            centre: 3,
            ..BodyTouch::default()
        });
        step(
            &mut skill,
            &mut entity,
            u64::from(PETTING_SUSTAIN_TICKS) - 1,
        );
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn sustained_touch_sets_being_pet() {
        let mut skill = Petting::new();
        let mut entity = touched_entity(BodyTouch {
            centre: 3,
            ..BodyTouch::default()
        });
        step(&mut skill, &mut entity, u64::from(PETTING_SUSTAIN_TICKS));
        assert_eq!(entity.mind.intent, Intent::Petted);
    }

    #[test]
    fn release_clears_being_pet() {
        let mut skill = Petting::new();
        let mut entity = touched_entity(BodyTouch {
            centre: 3,
            ..BodyTouch::default()
        });
        step(&mut skill, &mut entity, u64::from(PETTING_SUSTAIN_TICKS));
        assert_eq!(entity.mind.intent, Intent::Petted);

        // Release.
        entity.perception.body_touch = Some(BodyTouch::default());
        let _ = skill.invoke(&mut entity);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn release_does_not_clobber_unrelated_intent() {
        let mut skill = Petting::new();
        let mut entity = Entity::default();
        // Some other system set Listen — petting hasn't fired.
        entity.mind.intent = Intent::Listening;
        // Release tick (no body touch).
        let _ = skill.invoke(&mut entity);
        assert_eq!(entity.mind.intent, Intent::Listening);
    }

    #[test]
    fn quiet_tick_resets_counter_mid_sustain() {
        let mut skill = Petting::new();
        let mut entity = touched_entity(BodyTouch {
            centre: 3,
            ..BodyTouch::default()
        });
        // Almost there.
        step(
            &mut skill,
            &mut entity,
            u64::from(PETTING_SUSTAIN_TICKS) - 1,
        );
        // One quiet tick resets.
        entity.perception.body_touch = Some(BodyTouch::default());
        let _ = skill.invoke(&mut entity);
        // Resume touching — must run the full sustain again.
        entity.perception.body_touch = Some(BodyTouch {
            centre: 3,
            ..BodyTouch::default()
        });
        step(
            &mut skill,
            &mut entity,
            u64::from(PETTING_SUSTAIN_TICKS) - 1,
        );
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn counter_saturates_does_not_wrap() {
        let mut skill = Petting::new();
        let mut entity = touched_entity(BodyTouch {
            centre: 3,
            ..BodyTouch::default()
        });
        step(&mut skill, &mut entity, 500); // well past u8::MAX
        assert_eq!(entity.mind.intent, Intent::Petted);
    }

    #[test]
    fn any_zone_counts_not_just_centre() {
        let mut skill = Petting::new();
        let mut entity = touched_entity(BodyTouch {
            left: 1,
            ..BodyTouch::default()
        });
        entity.tick.now = Instant::from_millis(0);
        step(&mut skill, &mut entity, u64::from(PETTING_SUSTAIN_TICKS));
        assert_eq!(entity.mind.intent, Intent::Petted);
    }
}
