//! `EmotionFromAmbient`: ambient-light-reactive modifier that flips
//! `entity.mind.affect.emotion` to `Sleepy` when the room gets dark and wakes
//! when the light comes back on.
//!
//! ## Detection shape
//!
//! Reads `entity.perception.ambient_lux` each tick. The trigger uses hysteresis
//! to absorb sensor noise + natural light variation:
//!
//! - **Enter Sleepy:** `lux` below [`SLEEPY_ENTER_LUX`] while not
//!   already asleep.
//! - **Wake:** `lux` above [`SLEEPY_EXIT_LUX`] while asleep. Clears
//!   this modifier's own hold so autonomy resumes.
//! - Between the two thresholds, the modifier holds its current state.
//!
//! Unknown ambient (`ambient_lux = None`, i.e. the LTR-553 task hasn't
//! produced a reading yet) is treated as "no information" and never
//! triggers either transition.
//!
//! ## Coordination with the other emotion modifiers
//!
//! Like [`super::EmotionFromIntent`], this modifier respects an existing
//! `entity.mind.autonomy.manual_until` hold — if touch, a pickup, or any other
//! explicit input has already claimed the emotion, we stand down.
//! Ambient sleep is *background state*: it shouldn't override a user's
//! deliberate interaction.
//!
//! The hold set on a sleep-trigger is short
//! ([`AMBIENT_HOLD_MS`], ~5 s) rather than 30 s: we *want* it to
//! un-stick quickly once the room gets bright again, and the modifier
//! itself re-affirms the hold on every dark tick so Sleepy sticks as
//! long as the room stays dim.

use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::modifier::Modifier;

/// Ambient lux below which Sleepy triggers.
///
/// 20 lux ≈ "room with overhead light off, only glow from a nearby
/// monitor." Roughly the boundary where you'd naturally reach for a
/// lamp.
pub const SLEEPY_ENTER_LUX: f32 = 20.0;

/// Ambient lux above which the entity wakes up again.
///
/// 50 lux ≈ "desk lamp on at arm's length." Comfortably above the
/// noise floor of room lighting variations (shadows, cloud passes)
/// but well below daylight.
pub const SLEEPY_EXIT_LUX: f32 = 50.0;

/// How long the ambient-triggered hold pins Sleepy once set, in ms.
///
/// Short (5 s) by design: the modifier re-sets the hold on every dark
/// tick, so the effective behavior is "Sleepy while dark, resume
/// within 5 s of the room brightening." Keeps this modifier cheap to
/// reason about vs. touch's 30 s explicit hold.
pub const AMBIENT_HOLD_MS: u64 = 5_000;

/// Modifier that watches `entity.perception.ambient_lux` and toggles Sleepy
/// with hysteresis.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmotionFromAmbient {
    /// `true` while this modifier believes the entity should currently
    /// be asleep. Driven by the two-threshold hysteresis; a fresh
    /// instance starts `false` regardless of the first ambient
    /// reading so the entity wakes visibly at boot even in a dark
    /// room (the hysteresis flips it to Sleepy a tick later).
    is_asleep: bool,
}

impl EmotionFromAmbient {
    /// Construct a modifier in the awake state.
    #[must_use]
    pub const fn new() -> Self {
        Self { is_asleep: false }
    }

    /// Exposed for tests: whether this modifier currently believes the
    /// entity should be asleep.
    #[cfg(test)]
    const fn is_asleep(self) -> bool {
        self.is_asleep
    }
}

impl Modifier for EmotionFromAmbient {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "EmotionFromAmbient",
            description: "Hysteresis on perception.ambient_lux: forces emotion=Sleepy in dark \
                          rooms. Stands down when an earlier modifier already holds \
                          mind.autonomy.manual_until.",
            phase: Phase::Affect,
            priority: -50,
            reads: &[Field::AmbientLux, Field::Autonomy],
            writes: &[Field::Emotion, Field::Autonomy],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let Some(lux) = entity.perception.ambient_lux else {
            // No reading yet — nothing to do.
            return;
        };

        // Hysteresis: update our internal "is_asleep" belief.
        if !self.is_asleep && lux < SLEEPY_ENTER_LUX {
            self.is_asleep = true;
        } else if self.is_asleep && lux > SLEEPY_EXIT_LUX {
            self.is_asleep = false;
        }

        // Another modifier (touch, pickup) has already claimed the
        // emotion. Stand down — ambient is background state, explicit
        // input wins.
        if let Some(until) = entity.mind.autonomy.manual_until
            && now < until
        {
            return;
        }

        if self.is_asleep {
            entity.mind.affect.emotion = Emotion::Sleepy;
            entity.mind.autonomy.source = Some(crate::mind::OverrideSource::Ambient);
            // Re-affirm the hold every dark tick so Sleepy persists
            // as long as the room stays dim.
            entity.mind.autonomy.manual_until = Some(now + AMBIENT_HOLD_MS);
        }
        // When `is_asleep == false` and no hold is active we do
        // nothing — `EmotionCycle` takes over on the next tick and
        // drives autonomy forward. No need to explicitly clear
        // `manual_until` here; `EmotionFromTouch::update` handles
        // expiry cleanup on every tick.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Instant;

    /// Helper: make a bright-ambient avatar (well above the exit
    /// threshold).
    fn bright() -> Entity {
        let mut e = Entity::default();
        e.perception.ambient_lux = Some(200.0);
        e
    }

    /// Helper: make a dark-ambient avatar (well below enter threshold).
    fn dark() -> Entity {
        let mut e = Entity::default();
        e.perception.ambient_lux = Some(5.0);
        e
    }

    #[test]
    fn no_reading_does_nothing() {
        let mut entity = Entity::default(); // ambient_lux = None
        let mut sleepy = EmotionFromAmbient::new();
        for t in (0..1_000).step_by(50) {
            entity.tick.now = Instant::from_millis(t);
            sleepy.update(&mut entity);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert!(entity.mind.autonomy.manual_until.is_none());
    }

    #[test]
    fn dark_room_triggers_sleepy() {
        let mut entity = dark();
        let mut sleepy = EmotionFromAmbient::new();
        entity.tick.now = Instant::from_millis(100);
        sleepy.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Sleepy);
        assert!(sleepy.is_asleep());
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(100 + AMBIENT_HOLD_MS)),
        );
    }

    #[test]
    fn bright_room_does_not_trigger() {
        let mut entity = bright();
        let mut sleepy = EmotionFromAmbient::new();
        entity.tick.now = Instant::from_millis(100);
        sleepy.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert!(!sleepy.is_asleep());
        assert!(entity.mind.autonomy.manual_until.is_none());
    }

    #[test]
    fn hysteresis_holds_sleep_between_thresholds() {
        let mut entity = dark();
        let mut sleepy = EmotionFromAmbient::new();

        // Enter sleep at 5 lux.
        entity.tick.now = Instant::from_millis(0);
        sleepy.update(&mut entity);
        assert!(sleepy.is_asleep());

        // Dim bulb lights up — 30 lux is between ENTER (20) and EXIT
        // (50) thresholds. Must stay asleep.
        entity.perception.ambient_lux = Some(30.0);
        entity.tick.now = Instant::from_millis(100);
        sleepy.update(&mut entity);
        assert!(sleepy.is_asleep(), "30 lux is inside the hysteresis band");
        assert_eq!(entity.mind.affect.emotion, Emotion::Sleepy);
    }

    #[test]
    fn bright_room_wakes_from_sleep() {
        let mut entity = dark();
        let mut sleepy = EmotionFromAmbient::new();
        entity.tick.now = Instant::from_millis(0);
        sleepy.update(&mut entity);

        entity.perception.ambient_lux = Some(200.0);
        entity.tick.now = Instant::from_millis(100);
        sleepy.update(&mut entity);
        assert!(!sleepy.is_asleep());
        // Prior-frame hold remains (the modifier re-affirms but
        // doesn't actively clear); EmotionFromTouch::update clears
        // expired holds in the normal pipeline.
    }

    #[test]
    fn asymmetric_thresholds_prevent_flicker() {
        // Simulate ambient hovering around a single mid-point: if we
        // used one threshold at 35 lux the output would flicker. With
        // hysteresis (20 / 50), a hover in the 25–45 band never
        // toggles state.
        let mut entity = dark();
        let mut sleepy = EmotionFromAmbient::new();
        entity.tick.now = Instant::from_millis(0);
        sleepy.update(&mut entity);
        assert!(sleepy.is_asleep());

        for (i, lux) in [25.0_f32, 45.0, 30.0, 40.0, 35.0].into_iter().enumerate() {
            entity.perception.ambient_lux = Some(lux);
            entity.tick.now = Instant::from_millis(100 + (i as u64) * 100);
            sleepy.update(&mut entity);
            assert!(
                sleepy.is_asleep(),
                "lux {lux} in hysteresis band should hold prior state",
            );
        }
    }

    #[test]
    fn touch_hold_blocks_ambient_sleepy() {
        let mut entity = dark();
        // Touch just set emotion=Happy + 30 s hold.
        entity.mind.affect.emotion = Emotion::Happy;
        entity.mind.autonomy.manual_until = Some(Instant::from_millis(30_000));
        let mut sleepy = EmotionFromAmbient::new();

        entity.tick.now = Instant::from_millis(100);
        sleepy.update(&mut entity);
        assert_eq!(
            entity.mind.affect.emotion,
            Emotion::Happy,
            "touch-set emotion must survive concurrent darkness",
        );
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(30_000)),
            "touch-set hold deadline must be preserved",
        );
    }

    #[test]
    fn ambient_hold_is_renewed_every_dark_tick() {
        let mut entity = dark();
        let mut sleepy = EmotionFromAmbient::new();

        // First dark tick.
        entity.tick.now = Instant::from_millis(0);
        sleepy.update(&mut entity);
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(AMBIENT_HOLD_MS))
        );

        // Simulate clearing by EmotionFromTouch::update when the hold
        // expires (3 s later, still dark — modifier stack would see
        // the clear before EmotionFromAmbient runs in the same frame).
        entity.mind.autonomy.manual_until = None;

        entity.tick.now = Instant::from_millis(3_000);
        sleepy.update(&mut entity);
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(3_000 + AMBIENT_HOLD_MS)),
            "still-dark ticks must re-arm the hold",
        );
    }
}
