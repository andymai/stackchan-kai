//! `EmotionCycle`: a demo modifier that rotates `Avatar::emotion` through
//! a fixed sequence of emotions on a timer.
//!
//! Used by the firmware to exercise the full emotion pipeline on hardware
//! without needing input (touch, network, serial). It writes *only* to
//! `Avatar::emotion`; the downstream `EmotionStyle` modifier does the
//! actual style-field mutation.
//!
//! Cycle order is the default StackChan demo rotation; see
//! [`EmotionCycle::DEFAULT_SEQUENCE`].

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::modifier::Modifier;

/// Default dwell time per emotion, in milliseconds.
const DEFAULT_DWELL_MS: u64 = 4_000;

/// A modifier that rotates `avatar.emotion` through a fixed sequence,
/// dwelling on each emotion for `dwell_ms` milliseconds.
#[derive(Debug, Clone)]
pub struct EmotionCycle {
    /// Sequence of emotions to cycle through, in order.
    sequence: &'static [Emotion],
    /// Dwell time per emotion.
    dwell_ms: u64,
    /// Index into `sequence` of the currently-active emotion.
    index: usize,
    /// Time at which the current emotion began, or `None` on the first
    /// tick (so we can anchor the sequence to whatever the caller's
    /// clock happens to be reading).
    started_at: Option<Instant>,
}

impl EmotionCycle {
    /// Default demo rotation: neutral → happy → sleepy → surprised → sad
    /// → (repeat). Ordered so adjacent emotions contrast visibly.
    pub const DEFAULT_SEQUENCE: &'static [Emotion] = &[
        Emotion::Neutral,
        Emotion::Happy,
        Emotion::Sleepy,
        Emotion::Surprised,
        Emotion::Sad,
    ];

    /// Construct a cycle with the default sequence and 4 s dwell.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_params(Self::DEFAULT_SEQUENCE, DEFAULT_DWELL_MS)
    }

    /// Construct with a custom sequence and dwell time.
    ///
    /// # Panics
    ///
    /// Does not panic, but `sequence` must be non-empty for the cycle to
    /// advance; an empty slice leaves the avatar's emotion untouched.
    #[must_use]
    pub const fn with_params(sequence: &'static [Emotion], dwell_ms: u64) -> Self {
        Self {
            sequence,
            dwell_ms,
            index: 0,
            started_at: None,
        }
    }

    /// Current emotion the cycle is dwelling on, or `None` if the sequence
    /// is empty.
    #[must_use]
    pub fn current(&self) -> Option<Emotion> {
        self.sequence.get(self.index).copied()
    }
}

impl Default for EmotionCycle {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for EmotionCycle {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "EmotionCycle",
            description: "Autonomous demo: cycles mind.affect.emotion through a fixed sequence on \
                          a dwell timer. Stands down while mind.autonomy.manual_until holds.",
            phase: Phase::Affect,
            // Runs last in Affect so it observes the final manual_until
            // state set by Touch/Remote/Pickup/Voice/Ambient/LowBattery.
            priority: 100,
            reads: &[Field::Autonomy, Field::Emotion],
            writes: &[Field::Emotion],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        // Empty sequence: nothing to drive.
        if self.sequence.is_empty() || self.dwell_ms == 0 {
            return;
        }

        // Manual-override active: user input has pinned the emotion until
        // a deadline. Re-anchor `started_at` so the cycle resumes cleanly
        // when the hold expires.
        if let Some(until) = entity.mind.autonomy.manual_until
            && now < until
        {
            self.started_at = Some(now);
            return;
        }

        let Some(start) = self.started_at else {
            // First tick: anchor to now and apply index 0 immediately.
            self.started_at = Some(now);
            if let Some(first) = self.sequence.first() {
                entity.mind.affect.emotion = *first;
            }
            return;
        };

        let elapsed = now.saturating_duration_since(start);
        let steps = elapsed / self.dwell_ms;
        if steps == 0 {
            return;
        }

        let new_index =
            (self.index + usize::try_from(steps).unwrap_or(usize::MAX)) % self.sequence.len();
        if new_index != self.index {
            self.index = new_index;
            entity.mind.affect.emotion = self.sequence[self.index];
        }
        let consumed_ms = steps.saturating_mul(self.dwell_ms);
        self.started_at = Some(start + consumed_ms);
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::Entity;

    fn step(cycle: &mut EmotionCycle, entity: &mut Entity, ms: u64) {
        entity.tick.now = Instant::from_millis(ms);
        cycle.update(entity);
    }

    #[test]
    fn first_tick_applies_head_of_sequence() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Surprised;
        let mut cycle = EmotionCycle::new();
        step(&mut cycle, &mut entity, 0);
        assert_eq!(
            entity.mind.affect.emotion,
            EmotionCycle::DEFAULT_SEQUENCE[0],
        );
    }

    #[test]
    fn advances_on_dwell_boundary() {
        let mut entity = Entity::default();
        let mut cycle =
            EmotionCycle::with_params(&[Emotion::Neutral, Emotion::Happy, Emotion::Sad], 1_000);

        step(&mut cycle, &mut entity, 0);
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        step(&mut cycle, &mut entity, 999);
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        step(&mut cycle, &mut entity, 1_000);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        step(&mut cycle, &mut entity, 2_000);
        assert_eq!(entity.mind.affect.emotion, Emotion::Sad);
    }

    #[test]
    fn wraps_back_to_head() {
        let mut entity = Entity::default();
        let mut cycle = EmotionCycle::with_params(&[Emotion::Neutral, Emotion::Happy], 500);
        for s in 0..=6 {
            step(&mut cycle, &mut entity, s * 500);
        }
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
    }

    #[test]
    fn skipped_ticks_advance_correct_number_of_steps() {
        let mut entity = Entity::default();
        let mut cycle = EmotionCycle::with_params(
            &[
                Emotion::Neutral,
                Emotion::Happy,
                Emotion::Sad,
                Emotion::Sleepy,
            ],
            1_000,
        );
        step(&mut cycle, &mut entity, 0);
        step(&mut cycle, &mut entity, 2_500);
        assert_eq!(entity.mind.affect.emotion, Emotion::Sad);
    }

    #[test]
    fn manual_until_suppresses_advancement() {
        let mut entity = Entity::default();
        let mut cycle =
            EmotionCycle::with_params(&[Emotion::Neutral, Emotion::Happy, Emotion::Sad], 1_000);

        step(&mut cycle, &mut entity, 0);
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);

        entity.mind.affect.emotion = Emotion::Happy;
        entity.mind.autonomy.manual_until = Some(Instant::from_millis(30_500));

        step(&mut cycle, &mut entity, 29_500);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
    }

    #[test]
    fn cycle_resumes_cleanly_after_manual_hold() {
        let mut entity = Entity::default();
        let mut cycle =
            EmotionCycle::with_params(&[Emotion::Neutral, Emotion::Happy, Emotion::Sad], 1_000);

        step(&mut cycle, &mut entity, 0);

        entity.mind.affect.emotion = Emotion::Happy;
        entity.mind.autonomy.manual_until = Some(Instant::from_millis(30_500));
        step(&mut cycle, &mut entity, 15_000);
        step(&mut cycle, &mut entity, 29_000);

        entity.mind.autonomy.manual_until = None;

        step(&mut cycle, &mut entity, 29_000 + 1_000);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        step(&mut cycle, &mut entity, 29_000 + 2_000);
        assert_eq!(entity.mind.affect.emotion, Emotion::Sad);
    }

    #[test]
    fn empty_sequence_is_noop() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Happy;
        let mut cycle = EmotionCycle::with_params(&[], 1_000);
        step(&mut cycle, &mut entity, 0);
        step(&mut cycle, &mut entity, 10_000);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
    }
}
