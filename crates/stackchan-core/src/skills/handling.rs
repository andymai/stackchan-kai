//! `Handling`: skill that translates IMU readings into
//! [`Intent::PickedUp`], [`Intent::Shaken`], or [`Intent::Tilted`].
//!
//! Reads `entity.perception.accel_g` (written by the firmware IMU task
//! into perception each render frame) and runs three sub-detectors:
//!
//! - **Pickup**: magnitude outside the rest band sustained for at
//!   least [`PICKUP_SUSTAIN_MS`]. Captures both lifts and freefalls.
//!   Threshold: [`PICKUP_DEVIATION_G`].
//! - **Shake**: magnitude alternates between *high* and *low* extremes
//!   (each separated from rest by at least [`SHAKE_DEVIATION_G`]) at
//!   least [`SHAKE_REQUIRED_TRANSITIONS`] times within
//!   [`SHAKE_WINDOW_MS`].
//! - **Tilt**: `accel.z` stays under [`TILT_Z_THRESHOLD_G`] for at
//!   least [`TILT_SUSTAIN_MS`]. Fires when the gravity vector leaves
//!   the `+Z` axis (sideways or upside-down).
//!
//! ## Priority
//!
//! When multiple detectors fire on the same tick, this skill applies
//! the project-wide intent priority `PickedUp > Shaken > Tilted` and
//! writes the highest. It also overrides
//! [`Intent::Petted`](crate::skills::Petting) — physical handling of
//! the whole avatar dominates over local back-of-head touch — except
//! `Tilted`, which is a passive pose and yields to `BeingPet`.
//!
//! ## Coexistence
//!
//! The reflex layer ([`crate::modifiers::EmotionFromIntent`]) reads `intent`
//! transitions and supplies the matching emotion + manual-hold + chirp.
//! This skill writes intent only.
//!
//! ## Cleanup
//!
//! On a tick where no IMU detector fires, this skill clears intent only
//! if it currently holds one of `PickedUp` / `Shaken` / `Tilted`.
//! `BeingPet` / `Listen` / `Idle` are left alone — same pattern as
//! [`crate::skills::Petting`].

use crate::clock::Instant;
use crate::director::{Field, SkillMeta};
use crate::entity::Entity;
use crate::mind::Intent;
use crate::skill::{Skill, SkillStatus};

/// Squared magnitude of a 3-axis g-scaled acceleration vector.
///
/// `||v| - 1| > k` is equivalent to `|v|² ∉ [(1-k)², (1+k)²]`, so
/// callers compare squared magnitude against squared thresholds and
/// avoid pulling in `libm` for `sqrt`. `stackchan-core` is `no_std`
/// without `libm`.
#[allow(
    clippy::suboptimal_flops,
    reason = "f32::mul_add needs libm on no_std; stackchan-core stays libm-free"
)]
const fn magnitude_squared((x, y, z): (f32, f32, f32)) -> f32 {
    x * x + y * y + z * z
}

// ---- Pickup constants -----------------------------------------------

/// How far `|accel|` must deviate from the resting 1 g value, in g
/// units, to count as "in motion" for [`Intent::PickedUp`].
///
/// Matches the existing reflex-layer threshold so `EmotionFromIntent` and
/// `Handling` agree on what "lifted" means; only the sustain durations
/// differ.
pub const PICKUP_DEVIATION_G: f32 = 0.5;

/// How long the pickup deviation must persist before
/// [`Intent::PickedUp`] fires, in milliseconds.
///
/// Six times the reflex modifier's 50 ms debounce. The reflex fires on
/// the rising edge (instant startle); this skill fires on sustained
/// handling (now-being-held).
pub const PICKUP_SUSTAIN_MS: u64 = 300;

/// `(1 - PICKUP_DEVIATION_G)²` — squared lower bound of the rest band.
const PICKUP_LOW_SQ: f32 = (1.0 - PICKUP_DEVIATION_G) * (1.0 - PICKUP_DEVIATION_G);
/// `(1 + PICKUP_DEVIATION_G)²` — squared upper bound of the rest band.
const PICKUP_HIGH_SQ: f32 = (1.0 + PICKUP_DEVIATION_G) * (1.0 + PICKUP_DEVIATION_G);

// ---- Shake constants ------------------------------------------------

/// How far `|accel|` must deviate from 1 g (in g) to register one
/// extreme transition for shake detection.
pub const SHAKE_DEVIATION_G: f32 = 0.7;

/// Sliding window for counting shake transitions, in milliseconds.
pub const SHAKE_WINDOW_MS: u64 = 600;

/// Number of high↔low magnitude transitions that count as a shake
/// within [`SHAKE_WINDOW_MS`]. Four transitions = two full
/// oscillations.
pub const SHAKE_REQUIRED_TRANSITIONS: u8 = 4;

/// `(1 - SHAKE_DEVIATION_G)²` — squared *low* extreme threshold.
const SHAKE_LOW_SQ: f32 = (1.0 - SHAKE_DEVIATION_G) * (1.0 - SHAKE_DEVIATION_G);
/// `(1 + SHAKE_DEVIATION_G)²` — squared *high* extreme threshold.
const SHAKE_HIGH_SQ: f32 = (1.0 + SHAKE_DEVIATION_G) * (1.0 + SHAKE_DEVIATION_G);

// ---- Tilt constants -------------------------------------------------

/// Maximum `accel.z` (g) that still counts as "face-up." Below this,
/// the gravity vector has rotated away from `+Z` — the avatar is on
/// its side or upside-down.
pub const TILT_Z_THRESHOLD_G: f32 = 0.7;

/// How long `accel.z` must stay below [`TILT_Z_THRESHOLD_G`] before
/// [`Intent::Tilted`] fires, in milliseconds. Long enough that brief
/// re-orientations during pickup don't trigger.
pub const TILT_SUSTAIN_MS: u64 = 2_000;

// ---- Detectors ------------------------------------------------------

/// Direction of an above/below-rest extreme used by the shake
/// detector's transition counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Extreme {
    /// `|accel|² > SHAKE_HIGH_SQ` — magnitude well above 1 g.
    High,
    /// `|accel|² < SHAKE_LOW_SQ` — magnitude well below 1 g.
    Low,
}

/// Pickup sub-detector. Tracks how long magnitude has been outside the
/// rest band, fires once that duration crosses [`PICKUP_SUSTAIN_MS`].
#[derive(Debug, Clone, Copy, Default)]
struct Pickup {
    /// First tick the magnitude was outside the rest band, or `None`
    /// while inside it.
    out_of_band_since: Option<Instant>,
}

impl Pickup {
    /// `true` once the rest-band excursion has persisted long enough
    /// to count as a sustained pickup. State updates every call.
    fn fires_at(&mut self, accel: (f32, f32, f32), now: Instant) -> bool {
        let m2 = magnitude_squared(accel);
        let out = !(PICKUP_LOW_SQ..=PICKUP_HIGH_SQ).contains(&m2);
        if !out {
            self.out_of_band_since = None;
            return false;
        }
        let started = *self.out_of_band_since.get_or_insert(now);
        now.saturating_duration_since(started) >= PICKUP_SUSTAIN_MS
    }
}

/// Shake sub-detector. Counts high↔low magnitude transitions inside a
/// rolling [`SHAKE_WINDOW_MS`] and fires once the count reaches
/// [`SHAKE_REQUIRED_TRANSITIONS`].
#[derive(Debug, Clone, Copy, Default)]
struct Shake {
    /// Most recent observed extreme classification. Persists across
    /// rest-band ticks so a high → rest → low pattern still counts as
    /// one transition.
    last_extreme: Option<Extreme>,
    /// Number of high↔low transitions accumulated in the current
    /// window. Saturates at `u8::MAX`.
    transitions: u8,
    /// Timestamp of the first transition in the current window. The
    /// window expires `SHAKE_WINDOW_MS` after this.
    window_start: Option<Instant>,
}

impl Shake {
    /// `true` once enough alternating extremes have landed inside the
    /// rolling window to count as a shake. Updates state every call.
    fn fires_at(&mut self, accel: (f32, f32, f32), now: Instant) -> bool {
        // Drop stale transitions before classifying. Otherwise a
        // single transition lingers forever and a slow second one
        // (an hour later) wrongly counts.
        if let Some(start) = self.window_start
            && now.saturating_duration_since(start) > SHAKE_WINDOW_MS
        {
            self.transitions = 0;
            self.window_start = None;
        }

        let m2 = magnitude_squared(accel);
        let current = if m2 > SHAKE_HIGH_SQ {
            Some(Extreme::High)
        } else if m2 < SHAKE_LOW_SQ {
            Some(Extreme::Low)
        } else {
            None
        };

        if let Some(c) = current
            && self.last_extreme != Some(c)
        {
            if self.last_extreme.is_some() {
                // Genuine high↔low transition.
                self.transitions = self.transitions.saturating_add(1);
                self.window_start.get_or_insert(now);
            }
            self.last_extreme = Some(c);
        }

        self.transitions >= SHAKE_REQUIRED_TRANSITIONS
    }
}

/// Tilt sub-detector. Tracks how long the gravity vector has been off
/// the `+Z` axis and fires once that duration reaches
/// [`TILT_SUSTAIN_MS`].
#[derive(Debug, Clone, Copy, Default)]
struct Tilt {
    /// First tick `accel.z` dropped below [`TILT_Z_THRESHOLD_G`], or
    /// `None` while face-up.
    tilted_since: Option<Instant>,
}

impl Tilt {
    /// `true` once the device has been off-axis long enough to count
    /// as tilted. Updates state every call.
    fn fires_at(&mut self, accel: (f32, f32, f32), now: Instant) -> bool {
        let z = accel.2;
        if z >= TILT_Z_THRESHOLD_G {
            self.tilted_since = None;
            return false;
        }
        let started = *self.tilted_since.get_or_insert(now);
        now.saturating_duration_since(started) >= TILT_SUSTAIN_MS
    }
}

// ---- Skill ----------------------------------------------------------

/// IMU-derived intent skill. See module docs.
#[derive(Debug, Clone, Copy, Default)]
pub struct Handling {
    /// Sustained-deviation detector for [`Intent::PickedUp`].
    pickup: Pickup,
    /// Alternating-extreme detector for [`Intent::Shaken`].
    shake: Shake,
    /// Sustained-off-axis detector for [`Intent::Tilted`].
    tilt: Tilt,
    /// The intent this skill last wrote. Tracked so cleanup only
    /// clears intents Handling itself set; `BeingPet` / `Listen`
    /// stay untouched.
    last_set: Option<Intent>,
}

impl Handling {
    /// Construct a fresh `Handling` skill with no in-flight detector
    /// state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pickup: Pickup {
                out_of_band_since: None,
            },
            shake: Shake {
                last_extreme: None,
                transitions: 0,
                window_start: None,
            },
            tilt: Tilt { tilted_since: None },
            last_set: None,
        }
    }

    /// Decide which IMU intent (if any) this tick should produce,
    /// applying the priority `PickedUp > Shaken > Tilted`.
    fn decide(&mut self, accel: (f32, f32, f32), now: Instant) -> Option<Intent> {
        // Run all three detectors so each updates its internal state
        // even when a higher-priority one wins. Otherwise a sustained
        // pickup would keep the tilt detector's `tilted_since` armed
        // from however far back, and firing a stale tilt the moment
        // the pickup ends.
        let pickup = self.pickup.fires_at(accel, now);
        let shake = self.shake.fires_at(accel, now);
        let tilt = self.tilt.fires_at(accel, now);

        if pickup {
            Some(Intent::PickedUp)
        } else if shake {
            Some(Intent::Shaken)
        } else if tilt {
            Some(Intent::Tilted)
        } else {
            None
        }
    }
}

impl Skill for Handling {
    fn meta(&self) -> &'static SkillMeta {
        static META: SkillMeta = SkillMeta {
            name: "Handling",
            description: "IMU → intent. Sustained accel deviation → PickedUp; alternating \
                          high/low magnitude → Shaken; sustained off-axis Z → Tilted. Overrides \
                          BeingPet (handling beats local touch); yields to BeingPet on Tilted.",
            // Lower than Petting (50) so Handling fires AFTER Petting,
            // letting it observe + override the BeingPet Petting just
            // wrote on the same tick.
            priority: 40,
            writes: &[Field::Intent],
        };
        &META
    }

    fn should_fire(&self, _entity: &Entity) -> bool {
        // Detectors are stateful and need to advance every tick.
        true
    }

    fn invoke(&mut self, entity: &mut Entity) -> SkillStatus {
        let detected = self.decide(entity.perception.accel_g, entity.tick.now);

        match detected {
            Some(Intent::PickedUp | Intent::Shaken) => {
                // PickedUp / Shaken always win over whatever was set
                // earlier this tick — including Petting's BeingPet.
                let intent = detected.unwrap_or(Intent::Idle);
                entity.mind.intent = intent;
                self.last_set = Some(intent);
                SkillStatus::Continuing
            }
            Some(Intent::Tilted) => {
                // Tilted yields to BeingPet (lower priority than touch
                // per project intent ordering). Don't clobber if the
                // user is actively petting.
                if !matches!(entity.mind.intent, Intent::Petted) {
                    entity.mind.intent = Intent::Tilted;
                    self.last_set = Some(Intent::Tilted);
                    return SkillStatus::Continuing;
                }
                SkillStatus::Done
            }
            // No IMU condition met. Clear our previous write only.
            // The match arm above is `Intent::PickedUp | Shaken | Tilted`
            // — `Idle` and others come through as `None` from `decide`.
            Some(_) | None => {
                if let Some(prev) = self.last_set
                    && entity.mind.intent == prev
                {
                    entity.mind.intent = Intent::Idle;
                }
                self.last_set = None;
                SkillStatus::Done
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::missing_docs_in_private_items)]
mod tests {
    use super::*;
    use crate::perception::BodyTouch;

    /// Ms-per-tick used in tests. Matches the firmware render cadence
    /// closely enough that sample-count-driven sustains land where you
    /// expect.
    const TICK_MS: u64 = 30;

    fn at_rest() -> Entity {
        Entity::default()
    }

    fn step(skill: &mut Handling, entity: &mut Entity, ticks: u64) {
        let mut now = entity.tick.now;
        for _ in 0..ticks {
            now = now + TICK_MS;
            entity.tick.now = now;
            let _ = skill.invoke(entity);
        }
    }

    fn set_accel(entity: &mut Entity, accel: (f32, f32, f32)) {
        entity.perception.accel_g = accel;
    }

    #[test]
    fn rest_keeps_idle() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        step(&mut skill, &mut entity, 200);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn sustained_lift_sets_picked_up() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        set_accel(&mut entity, (0.0, 0.0, 1.8));

        // Just over PICKUP_SUSTAIN_MS at TICK_MS cadence.
        let ticks = PICKUP_SUSTAIN_MS / TICK_MS + 1;
        step(&mut skill, &mut entity, ticks);
        assert_eq!(entity.mind.intent, Intent::PickedUp);
    }

    #[test]
    fn brief_lift_under_sustain_does_not_fire() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        set_accel(&mut entity, (0.0, 0.0, 1.8));

        // Two ticks ≈ 60 ms — well under PICKUP_SUSTAIN_MS.
        step(&mut skill, &mut entity, 2);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn freefall_sets_picked_up_via_below_band() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        // Magnitude ≈ 0.1 g — well below the rest band.
        set_accel(&mut entity, (0.0, 0.0, 0.1));
        step(&mut skill, &mut entity, PICKUP_SUSTAIN_MS / TICK_MS + 1);
        assert_eq!(entity.mind.intent, Intent::PickedUp);
    }

    #[test]
    fn rapid_oscillation_sets_shaken() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        // Synthesize four high↔low transitions inside the window:
        // tick 0: high (sets last_extreme=High, no transition yet)
        // tick 1: low  → +1
        // tick 2: high → +1
        // tick 3: low  → +1
        // tick 4: high → +1 (= 4, fires)
        let pattern = [(0.0, 0.0, 2.0), (0.0, 0.0, 0.1)];
        let mut now = entity.tick.now;
        for i in 0..5 {
            now = now + TICK_MS;
            entity.tick.now = now;
            entity.perception.accel_g = pattern[i % 2];
            let _ = skill.invoke(&mut entity);
        }
        assert_eq!(entity.mind.intent, Intent::Shaken);
    }

    #[test]
    fn slow_swap_outside_window_does_not_shake() {
        let mut skill = Handling::new();
        let mut entity = at_rest();

        // First transition: rest → high → low (1 transition).
        set_accel(&mut entity, (0.0, 0.0, 2.0));
        entity.tick.now = entity.tick.now + 50;
        let _ = skill.invoke(&mut entity);
        set_accel(&mut entity, (0.0, 0.0, 0.1));
        entity.tick.now = entity.tick.now + 50;
        let _ = skill.invoke(&mut entity);

        // Wait past the window.
        set_accel(&mut entity, (0.0, 0.0, 1.0));
        entity.tick.now = entity.tick.now + SHAKE_WINDOW_MS + 100;
        let _ = skill.invoke(&mut entity);

        // Next "shake" attempt — only 1 transition lands inside the
        // fresh window because the old one expired.
        set_accel(&mut entity, (0.0, 0.0, 2.0));
        entity.tick.now = entity.tick.now + 30;
        let _ = skill.invoke(&mut entity);
        set_accel(&mut entity, (0.0, 0.0, 0.1));
        entity.tick.now = entity.tick.now + 30;
        let _ = skill.invoke(&mut entity);

        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn sustained_sideways_sets_tilted() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        // Sideways: gravity points along X, Z reads near 0.
        set_accel(&mut entity, (1.0, 0.0, 0.0));
        // Ceiling-divide + 1 so the elapsed time clears `TILT_SUSTAIN_MS`
        // even when `TILT_SUSTAIN_MS` doesn't divide evenly by `TICK_MS`.
        let ticks = TILT_SUSTAIN_MS.div_ceil(TICK_MS) + 1;
        step(&mut skill, &mut entity, ticks);
        assert_eq!(entity.mind.intent, Intent::Tilted);
    }

    #[test]
    fn brief_tilt_does_not_fire() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        set_accel(&mut entity, (1.0, 0.0, 0.0));
        // 5 ticks ≈ 150 ms — far under TILT_SUSTAIN_MS.
        step(&mut skill, &mut entity, 5);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn picked_up_overrides_being_pet() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        // Some other skill set BeingPet earlier this tick.
        entity.mind.intent = Intent::Petted;
        set_accel(&mut entity, (0.0, 0.0, 1.8));
        step(&mut skill, &mut entity, PICKUP_SUSTAIN_MS / TICK_MS + 1);
        assert_eq!(entity.mind.intent, Intent::PickedUp);
    }

    #[test]
    fn tilted_yields_to_being_pet() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        entity.mind.intent = Intent::Petted;
        // Force perception to also indicate active body touch so the
        // semantics match a real concurrent-sustained-pet scenario.
        entity.perception.body_touch = Some(BodyTouch {
            centre: 3,
            ..BodyTouch::default()
        });
        set_accel(&mut entity, (1.0, 0.0, 0.0));
        let ticks = TILT_SUSTAIN_MS.div_ceil(TICK_MS) + 1;
        step(&mut skill, &mut entity, ticks);
        // BeingPet survived; Handling stood down.
        assert_eq!(entity.mind.intent, Intent::Petted);
    }

    #[test]
    fn release_clears_picked_up_back_to_idle() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        set_accel(&mut entity, (0.0, 0.0, 1.8));
        step(&mut skill, &mut entity, PICKUP_SUSTAIN_MS / TICK_MS + 1);
        assert_eq!(entity.mind.intent, Intent::PickedUp);

        // Settle.
        set_accel(&mut entity, (0.0, 0.0, 1.0));
        step(&mut skill, &mut entity, 5);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn release_does_not_clobber_unrelated_intent() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        // Some other skill (Petting) set BeingPet; Handling never
        // touched intent.
        entity.mind.intent = Intent::Petted;
        // Idle accel for several ticks — Handling should not write.
        step(&mut skill, &mut entity, 10);
        assert_eq!(entity.mind.intent, Intent::Petted);
    }

    #[test]
    fn picked_up_to_idle_then_to_being_pet_no_clobber() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        set_accel(&mut entity, (0.0, 0.0, 1.8));
        step(&mut skill, &mut entity, PICKUP_SUSTAIN_MS / TICK_MS + 1);
        assert_eq!(entity.mind.intent, Intent::PickedUp);

        // Release.
        set_accel(&mut entity, (0.0, 0.0, 1.0));
        step(&mut skill, &mut entity, 1);
        assert_eq!(entity.mind.intent, Intent::Idle);

        // Now Petting writes BeingPet — Handling must not clear it
        // back to Idle.
        entity.mind.intent = Intent::Petted;
        step(&mut skill, &mut entity, 5);
        assert_eq!(entity.mind.intent, Intent::Petted);
    }

    #[test]
    fn pickup_and_tilt_at_once_picks_picked_up() {
        let mut skill = Handling::new();
        let mut entity = at_rest();
        // High lateral accel: m² > 2.25 (pickup) AND z=0 (tilt).
        set_accel(&mut entity, (1.6, 0.0, 0.0));
        step(
            &mut skill,
            &mut entity,
            (TILT_SUSTAIN_MS / TICK_MS).max(PICKUP_SUSTAIN_MS / TICK_MS) + 1,
        );
        // Pickup wins.
        assert_eq!(entity.mind.intent, Intent::PickedUp);
    }
}
