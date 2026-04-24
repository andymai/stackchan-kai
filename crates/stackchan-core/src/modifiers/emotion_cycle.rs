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

use super::Modifier;
use crate::avatar::Avatar;
use crate::clock::Instant;
use crate::emotion::Emotion;

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
    fn update(&mut self, avatar: &mut Avatar, now: Instant) {
        // Empty sequence: nothing to drive.
        if self.sequence.is_empty() || self.dwell_ms == 0 {
            return;
        }

        // Manual-override active: user input (e.g. `EmotionTouch`) has
        // pinned the emotion until a deadline. Re-anchor `started_at` so
        // the cycle resumes cleanly when the hold expires instead of
        // snap-advancing by however many dwell windows passed.
        if let Some(until) = avatar.manual_until
            && now < until
        {
            self.started_at = Some(now);
            return;
        }

        let Some(start) = self.started_at else {
            // First tick: anchor to now and apply index 0 immediately.
            self.started_at = Some(now);
            if let Some(first) = self.sequence.first() {
                avatar.emotion = *first;
            }
            return;
        };

        let elapsed = now.saturating_duration_since(start);
        let steps = elapsed / self.dwell_ms;
        if steps == 0 {
            return;
        }

        // Advance by as many dwell windows as elapsed; this makes the
        // cycle robust to missed ticks without skipping visually.
        let new_index =
            (self.index + usize::try_from(steps).unwrap_or(usize::MAX)) % self.sequence.len();
        if new_index != self.index {
            self.index = new_index;
            avatar.emotion = self.sequence[self.index];
        }
        // Re-anchor to the current dwell window boundary rather than
        // `now`, so drift doesn't accumulate across many cycles.
        let consumed_ms = steps.saturating_mul(self.dwell_ms);
        self.started_at = Some(start + consumed_ms);
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn first_tick_applies_head_of_sequence() {
        let mut avatar = Avatar::default();
        avatar.emotion = Emotion::Surprised; // something other than the head
        let mut cycle = EmotionCycle::new();

        cycle.update(&mut avatar, Instant::from_millis(0));
        assert_eq!(
            avatar.emotion,
            EmotionCycle::DEFAULT_SEQUENCE[0],
            "first tick should snap to the head of the sequence"
        );
    }

    #[test]
    fn advances_on_dwell_boundary() {
        let mut avatar = Avatar::default();
        let mut cycle =
            EmotionCycle::with_params(&[Emotion::Neutral, Emotion::Happy, Emotion::Sad], 1_000);

        cycle.update(&mut avatar, Instant::from_millis(0));
        assert_eq!(avatar.emotion, Emotion::Neutral);

        cycle.update(&mut avatar, Instant::from_millis(999));
        assert_eq!(avatar.emotion, Emotion::Neutral);

        cycle.update(&mut avatar, Instant::from_millis(1_000));
        assert_eq!(avatar.emotion, Emotion::Happy);

        cycle.update(&mut avatar, Instant::from_millis(2_000));
        assert_eq!(avatar.emotion, Emotion::Sad);
    }

    #[test]
    fn wraps_back_to_head() {
        let mut avatar = Avatar::default();
        let mut cycle = EmotionCycle::with_params(&[Emotion::Neutral, Emotion::Happy], 500);

        for step in 0..=6 {
            cycle.update(&mut avatar, Instant::from_millis(step * 500));
        }
        // 6 windows over 2 emotions = back to index 0.
        assert_eq!(avatar.emotion, Emotion::Neutral);
    }

    #[test]
    fn skipped_ticks_advance_correct_number_of_steps() {
        let mut avatar = Avatar::default();
        let mut cycle = EmotionCycle::with_params(
            &[
                Emotion::Neutral,
                Emotion::Happy,
                Emotion::Sad,
                Emotion::Sleepy,
            ],
            1_000,
        );

        cycle.update(&mut avatar, Instant::from_millis(0));
        // Jump forward 2.5 s -- should land on index 2 (Sad), not index 1.
        cycle.update(&mut avatar, Instant::from_millis(2_500));
        assert_eq!(avatar.emotion, Emotion::Sad);
    }

    #[test]
    fn manual_until_suppresses_advancement() {
        let mut avatar = Avatar::default();
        let mut cycle =
            EmotionCycle::with_params(&[Emotion::Neutral, Emotion::Happy, Emotion::Sad], 1_000);

        // Establish the baseline.
        cycle.update(&mut avatar, Instant::from_millis(0));
        assert_eq!(avatar.emotion, Emotion::Neutral);

        // User taps at t=500: the touch modifier would set Happy + hold
        // until t=30_500. Simulate that here.
        avatar.emotion = Emotion::Happy;
        avatar.manual_until = Some(Instant::from_millis(30_500));

        // 29 s later (still inside the hold): EmotionCycle must NOT
        // advance, and must not overwrite the manually-set emotion.
        cycle.update(&mut avatar, Instant::from_millis(29_500));
        assert_eq!(avatar.emotion, Emotion::Happy);
    }

    #[test]
    fn cycle_resumes_cleanly_after_manual_hold() {
        let mut avatar = Avatar::default();
        let mut cycle =
            EmotionCycle::with_params(&[Emotion::Neutral, Emotion::Happy, Emotion::Sad], 1_000);

        cycle.update(&mut avatar, Instant::from_millis(0));

        // Manual hold from t=500 to t=30_500.
        avatar.emotion = Emotion::Happy;
        avatar.manual_until = Some(Instant::from_millis(30_500));
        cycle.update(&mut avatar, Instant::from_millis(15_000));
        cycle.update(&mut avatar, Instant::from_millis(29_000));

        // Hold expires; someone (EmotionTouch in real code) clears it.
        avatar.manual_until = None;

        // One dwell window after the hold ends the cycle advances by
        // exactly one step — not by the 30 windows that passed while
        // paused.
        cycle.update(&mut avatar, Instant::from_millis(29_000 + 1_000));
        assert_eq!(
            avatar.emotion,
            Emotion::Happy,
            "first post-hold tick carries the user's emotion forward"
        );
        cycle.update(&mut avatar, Instant::from_millis(29_000 + 2_000));
        assert_eq!(
            avatar.emotion,
            Emotion::Sad,
            "second post-hold dwell advances by exactly one step"
        );
    }

    #[test]
    fn empty_sequence_is_noop() {
        let mut avatar = Avatar::default();
        avatar.emotion = Emotion::Happy;
        let mut cycle = EmotionCycle::with_params(&[], 1_000);
        cycle.update(&mut avatar, Instant::from_millis(0));
        cycle.update(&mut avatar, Instant::from_millis(10_000));
        assert_eq!(
            avatar.emotion,
            Emotion::Happy,
            "emotion should be unchanged"
        );
    }
}
