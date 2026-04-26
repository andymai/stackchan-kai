//! `IntentReflex`: translates IMU-derived intent transitions into
//! `mind.affect.emotion` + `mind.autonomy.manual_until` + a one-shot
//! `voice.chirp_request`.
//!
//! [`crate::skills::Handling`] owns the IMU → intent translation; this
//! modifier owns intent → emotion. Single source of truth for "the
//! avatar is being handled."
//!
//! ## Reaction map
//!
//! | Intent transition         | Emotion     | `OverrideSource`         | Chirp                       |
//! |---------------------------|-------------|--------------------------|-----------------------------|
//! | `* → PickedUp`            | `Surprised` | `Pickup`                 | `Pickup`                    |
//! | `* → Shaken`              | `Angry`     | `Shake`                  | none (no Shake clip yet)    |
//! | `* → Tilted`              | unchanged   | unchanged                | none (passive pose)         |
//!
//! ## Coordination with explicit user input
//!
//! Like the modifier it replaces, this defers when an
//! [`crate::modifiers::EmotionTouch`]-set hold is active — explicit
//! taps beat reflexive pickup/shake reactions.
//!
//! ## Frame-ordering caveat
//!
//! Modifiers run before skills in [`crate::Director::run`], so the
//! `mind.intent` value this modifier reads is whatever
//! [`crate::skills::Handling`] wrote on the *previous* frame. The
//! reaction therefore lands one render frame (~33 ms at 30 FPS) after
//! the intent transition — well below human perception.

use super::MANUAL_HOLD_MS;
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::mind::{Intent, OverrideSource};
use crate::modifier::Modifier;
use crate::voice::ChirpKind;

/// Modifier that watches `mind.intent` and reacts to transitions into
/// IMU-derived states.
#[derive(Debug, Clone, Copy)]
pub struct IntentReflex {
    /// Last-tick intent. Initialised to [`Intent::Idle`] so the first
    /// observed `Idle → X` transition fires correctly without a
    /// startup blip.
    last_intent: Intent,
}

impl IntentReflex {
    /// Construct an unarmed reflex.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_intent: Intent::Idle,
        }
    }
}

impl Default for IntentReflex {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for IntentReflex {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "IntentReflex",
            description: "Reacts to mind.intent transitions: PickedUp → Surprised + Pickup chirp; \
                          Shaken → Angry; Tilted → no reflex. Defers to active EmotionTouch \
                          holds.",
            phase: Phase::Affect,
            // After `EmotionTouch` clears expired holds, before
            // `EmotionCycle` advances autonomously.
            priority: -80,
            reads: &[Field::Intent, Field::Autonomy],
            writes: &[Field::Emotion, Field::Autonomy, Field::ChirpRequest],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let intent = entity.mind.intent;

        // Edge detection: only react when intent *changes* into a
        // reactive variant. Sustained intent doesn't keep re-firing
        // (which would spam chirps and re-extend the manual hold).
        let entered = if intent == self.last_intent {
            None
        } else {
            Some(intent)
        };
        self.last_intent = intent;

        let Some(curr) = entered else { return };

        // Pull the (emotion, source, chirp) triple for this transition.
        // `Tilted` and any non-reactive intent return `None` and we
        // exit without writing.
        let Some((emotion, source, chirp)) = react(curr) else {
            return;
        };

        // Defer to an active explicit-input hold. `EmotionTouch` clears
        // expired holds on its tick (which runs before this one), so a
        // `manual_until` still in the future means a real user
        // override is in effect.
        if let Some(until) = entity.mind.autonomy.manual_until
            && now < until
        {
            return;
        }

        entity.mind.affect.emotion = emotion;
        entity.mind.autonomy.manual_until = Some(now + MANUAL_HOLD_MS);
        entity.mind.autonomy.source = Some(source);
        if let Some(c) = chirp {
            entity.voice.chirp_request = Some(c);
        }
    }
}

/// Per-intent reaction map. `None` = no reflex (intent doesn't react,
/// or isn't IMU-derived). Returning a `chirp` of `None` means a
/// reactive transition that doesn't request audio.
const fn react(intent: Intent) -> Option<(Emotion, OverrideSource, Option<ChirpKind>)> {
    match intent {
        Intent::PickedUp => Some((
            Emotion::Surprised,
            OverrideSource::Pickup,
            Some(ChirpKind::Pickup),
        )),
        Intent::Shaken => Some((Emotion::Angry, OverrideSource::Shake, None)),
        // `Startled` reaction is owned by `IntentFromLoud` (single-tick
        // latency: it writes emotion + chirp + hold itself in
        // `Phase::Affect`). Returning `None` here keeps IntentReflex
        // out of the way so we don't double-emit.
        Intent::Tilted | Intent::Idle | Intent::Listen | Intent::BeingPet | Intent::Startled => {
            None
        }
    }
}

#[cfg(test)]
#[allow(clippy::missing_docs_in_private_items)]
mod tests {
    use super::*;
    use crate::clock::Instant;

    fn at(now_ms: u64) -> Entity {
        let mut e = Entity::default();
        e.tick.now = Instant::from_millis(now_ms);
        e
    }

    #[test]
    fn idle_does_not_fire() {
        let mut m = IntentReflex::new();
        let mut entity = at(0);
        for t in (0..500).step_by(33) {
            entity.tick.now = Instant::from_millis(t);
            m.update(&mut entity);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert!(entity.mind.autonomy.manual_until.is_none());
        assert!(entity.voice.chirp_request.is_none());
    }

    #[test]
    fn picked_up_transition_fires_surprised_with_chirp() {
        let mut m = IntentReflex::new();
        let mut entity = at(0);
        m.update(&mut entity);

        entity.tick.now = Instant::from_millis(33);
        entity.mind.intent = Intent::PickedUp;
        m.update(&mut entity);

        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(33 + MANUAL_HOLD_MS))
        );
        assert_eq!(entity.mind.autonomy.source, Some(OverrideSource::Pickup));
        assert_eq!(entity.voice.chirp_request, Some(ChirpKind::Pickup));
    }

    #[test]
    fn shaken_transition_fires_angry_no_chirp() {
        let mut m = IntentReflex::new();
        let mut entity = at(0);
        m.update(&mut entity);

        entity.tick.now = Instant::from_millis(33);
        entity.mind.intent = Intent::Shaken;
        m.update(&mut entity);

        assert_eq!(entity.mind.affect.emotion, Emotion::Angry);
        assert_eq!(entity.mind.autonomy.source, Some(OverrideSource::Shake));
        assert!(
            entity.voice.chirp_request.is_none(),
            "no Shake chirp variant exists yet"
        );
    }

    #[test]
    fn tilted_does_not_fire() {
        let mut m = IntentReflex::new();
        let mut entity = at(0);
        m.update(&mut entity);

        entity.tick.now = Instant::from_millis(33);
        entity.mind.intent = Intent::Tilted;
        m.update(&mut entity);

        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert!(entity.mind.autonomy.manual_until.is_none());
        assert!(entity.voice.chirp_request.is_none());
    }

    #[test]
    fn sustained_picked_up_only_fires_once() {
        let mut m = IntentReflex::new();
        let mut entity = at(0);
        entity.mind.intent = Intent::PickedUp;
        m.update(&mut entity);
        let first_until = entity.mind.autonomy.manual_until;
        // Drain the chirp the way firmware would after dispatch.
        entity.voice.chirp_request = None;

        for t in (33..1_000).step_by(33) {
            entity.tick.now = Instant::from_millis(t);
            m.update(&mut entity);
        }
        assert_eq!(
            entity.mind.autonomy.manual_until, first_until,
            "sustained intent must not keep extending the hold"
        );
        assert!(
            entity.voice.chirp_request.is_none(),
            "sustained intent must not re-chirp"
        );
    }

    #[test]
    fn touch_hold_blocks_pickup_reflex() {
        let mut m = IntentReflex::new();
        let mut entity = at(0);
        entity.mind.affect.emotion = Emotion::Happy;
        entity.mind.autonomy.manual_until = Some(Instant::from_millis(MANUAL_HOLD_MS));
        m.update(&mut entity);

        entity.tick.now = Instant::from_millis(33);
        entity.mind.intent = Intent::PickedUp;
        m.update(&mut entity);

        assert_eq!(
            entity.mind.affect.emotion,
            Emotion::Happy,
            "explicit touch hold beats pickup reflex"
        );
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(MANUAL_HOLD_MS)),
            "touch hold deadline must not be extended"
        );
        assert!(entity.voice.chirp_request.is_none());
    }

    #[test]
    fn reflex_fires_after_touch_hold_expires() {
        let mut m = IntentReflex::new();
        let mut entity = at(0);
        entity.mind.affect.emotion = Emotion::Happy;
        entity.mind.autonomy.manual_until = Some(Instant::from_millis(1_000));
        m.update(&mut entity);

        // Pickup begins mid-hold — suppressed.
        entity.tick.now = Instant::from_millis(500);
        entity.mind.intent = Intent::PickedUp;
        m.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);

        // Hold expires (in real code, EmotionTouch clears it). Re-arm
        // by transitioning back to Idle, then in to PickedUp again so
        // a fresh edge is observed.
        entity.tick.now = Instant::from_millis(1_500);
        entity.mind.intent = Intent::Idle;
        entity.mind.autonomy.manual_until = None;
        m.update(&mut entity);

        entity.tick.now = Instant::from_millis(2_000);
        entity.mind.intent = Intent::PickedUp;
        m.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
    }

    #[test]
    fn pickup_then_release_then_pickup_re_fires() {
        let mut m = IntentReflex::new();
        let mut entity = at(0);
        entity.mind.intent = Intent::PickedUp;
        m.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);

        // Release. Settle.
        entity.mind.intent = Intent::Idle;
        entity.mind.autonomy.manual_until = None;
        entity.mind.affect.emotion = Emotion::Neutral;
        entity.tick.now = Instant::from_millis(40_000);
        m.update(&mut entity);

        // Pick up again — fresh edge.
        entity.tick.now = Instant::from_millis(40_033);
        entity.mind.intent = Intent::PickedUp;
        m.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
    }

    #[test]
    fn picked_up_to_shaken_re_fires_with_angry() {
        let mut m = IntentReflex::new();
        let mut entity = at(0);
        entity.mind.intent = Intent::PickedUp;
        m.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
        // Clear the hold the way EmotionTouch would after expiry —
        // otherwise the second transition would be blocked.
        entity.mind.autonomy.manual_until = None;
        entity.voice.chirp_request = None;

        entity.tick.now = Instant::from_millis(33);
        entity.mind.intent = Intent::Shaken;
        m.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Angry);
        assert_eq!(entity.mind.autonomy.source, Some(OverrideSource::Shake));
    }
}
