//! Blink modifier: drives both eyes through an open → closed → open cycle.

use super::Modifier;
use crate::avatar::{Avatar, EyePhase};
use crate::clock::Instant;

/// Default time eyes stay open between blinks, in milliseconds.
const DEFAULT_OPEN_MS: u64 = 5_200;
/// Default duration of a blink (eyes closed), in milliseconds.
const DEFAULT_CLOSED_MS: u64 = 180;

/// A modifier that periodically closes both eyes for a short duration.
///
/// The weight on both eyes stays at 100 while open and drops to 0 during the
/// closed window. `phase` flips between [`EyePhase::Open`] and
/// [`EyePhase::Closed`] in lockstep so renderers can vary their drawing
/// strategy (e.g. draw a thin horizontal line when closed).
#[derive(Debug, Clone, Copy)]
pub struct Blink {
    /// Milliseconds eyes remain open between blinks.
    open_ms: u64,
    /// Milliseconds eyes remain closed during a blink.
    closed_ms: u64,
    /// Internal state machine.
    state: BlinkState,
}

/// Internal state tracking when the next transition fires.
#[derive(Debug, Clone, Copy)]
enum BlinkState {
    /// Waiting to initialize on the first `update` call. Used so the very
    /// first tick establishes `next_transition` relative to the clock the
    /// caller is actually using -- modifiers don't know the starting time.
    Uninitialized,
    /// Eyes currently open; they close at `transition_at`.
    Open {
        /// Monotonic time at which the next open->closed transition happens.
        transition_at: Instant,
    },
    /// Eyes currently closed; they open at `transition_at`.
    Closed {
        /// Monotonic time at which the next closed->open transition happens.
        transition_at: Instant,
    },
}

impl Blink {
    /// Construct a new `Blink` with default timing (~5.2 s open, 180 ms closed).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            open_ms: DEFAULT_OPEN_MS,
            closed_ms: DEFAULT_CLOSED_MS,
            state: BlinkState::Uninitialized,
        }
    }

    /// Construct a `Blink` with custom timing.
    #[must_use]
    pub const fn with_timing(open_ms: u64, closed_ms: u64) -> Self {
        Self {
            open_ms,
            closed_ms,
            state: BlinkState::Uninitialized,
        }
    }
}

impl Default for Blink {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for Blink {
    fn update(&mut self, avatar: &mut Avatar, now: Instant) {
        match self.state {
            BlinkState::Uninitialized => {
                // First tick: eyes are currently open; schedule the first blink.
                self.state = BlinkState::Open {
                    transition_at: now + self.open_ms,
                };
                set_both_eyes(avatar, EyePhase::Open, 100);
            }
            BlinkState::Open { transition_at } if now >= transition_at => {
                self.state = BlinkState::Closed {
                    transition_at: now + self.closed_ms,
                };
                set_both_eyes(avatar, EyePhase::Closed, 0);
            }
            BlinkState::Closed { transition_at } if now >= transition_at => {
                self.state = BlinkState::Open {
                    transition_at: now + self.open_ms,
                };
                set_both_eyes(avatar, EyePhase::Open, 100);
            }
            // Still in the current phase; nothing to change.
            BlinkState::Open { .. } | BlinkState::Closed { .. } => {}
        }
    }
}

/// Apply a (phase, weight) pair to both eyes on `avatar`.
const fn set_both_eyes(avatar: &mut Avatar, phase: EyePhase, weight: u8) {
    avatar.left_eye.phase = phase;
    avatar.left_eye.weight = weight;
    avatar.right_eye.phase = phase;
    avatar.right_eye.weight = weight;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avatar::Avatar;

    #[test]
    fn first_update_leaves_eyes_open() {
        let mut avatar = Avatar::default();
        let mut blink = Blink::new();
        blink.update(&mut avatar, Instant::from_millis(0));
        assert_eq!(avatar.left_eye.phase, EyePhase::Open);
        assert_eq!(avatar.right_eye.phase, EyePhase::Open);
        assert_eq!(avatar.left_eye.weight, 100);
    }

    #[test]
    fn blinks_after_open_window_elapses() {
        let mut avatar = Avatar::default();
        let mut blink = Blink::with_timing(100, 20);

        blink.update(&mut avatar, Instant::from_millis(0));
        assert_eq!(avatar.left_eye.phase, EyePhase::Open);

        // Just before the transition -- still open.
        blink.update(&mut avatar, Instant::from_millis(99));
        assert_eq!(avatar.left_eye.phase, EyePhase::Open);

        // At the transition -- eyes close.
        blink.update(&mut avatar, Instant::from_millis(100));
        assert_eq!(avatar.left_eye.phase, EyePhase::Closed);
        assert_eq!(avatar.left_eye.weight, 0);
        assert_eq!(avatar.right_eye.phase, EyePhase::Closed);
    }

    #[test]
    fn reopens_after_closed_window_elapses() {
        let mut avatar = Avatar::default();
        let mut blink = Blink::with_timing(100, 20);

        // Cycle through: init -> open -> closed -> open again.
        blink.update(&mut avatar, Instant::from_millis(0));
        blink.update(&mut avatar, Instant::from_millis(100));
        assert_eq!(avatar.left_eye.phase, EyePhase::Closed);

        blink.update(&mut avatar, Instant::from_millis(120));
        assert_eq!(avatar.left_eye.phase, EyePhase::Open);
        assert_eq!(avatar.left_eye.weight, 100);
    }

    #[test]
    fn cycle_repeats_indefinitely() {
        let mut avatar = Avatar::default();
        let mut blink = Blink::with_timing(100, 20);

        // Simulate ~5 full cycles.
        let mut transitions = 0_u32;
        let mut last_phase = EyePhase::Open;
        for ms in 0..=600 {
            blink.update(&mut avatar, Instant::from_millis(ms));
            if avatar.left_eye.phase != last_phase {
                transitions += 1;
                last_phase = avatar.left_eye.phase;
            }
        }
        // 600 ms / (100 + 20) = 5 cycles -> 10 transitions (open->closed->open pairs).
        // First tick is an initialization, not a transition.
        assert!(transitions >= 8, "only saw {transitions} transitions");
    }
}
