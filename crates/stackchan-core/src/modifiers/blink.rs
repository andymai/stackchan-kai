//! Blink modifier: drives both eyes through an open → closed → open cycle.
//!
//! Reads two emotion-driven style fields from the avatar so modifier order
//! stays composable:
//!
//! - `Avatar::blink_rate_scale` — scales the open-phase duration.
//!   `SCALE_DEFAULT` (128) runs at baseline cadence; lower values slow
//!   blinks (Sleepy); `0` suppresses blinks entirely (Surprised).
//! - `Eye::open_weight` — upper bound on `weight` when re-opening. Sleepy
//!   drops this to ~55 for a droopy-lid look without changing Blink's
//!   state machine.

use super::Modifier;
use crate::avatar::{Avatar, EyePhase, SCALE_DEFAULT};
use crate::clock::Instant;

/// Default time eyes stay open between blinks, in milliseconds.
const DEFAULT_OPEN_MS: u64 = 5_200;
/// Default duration of a blink (eyes closed), in milliseconds.
const DEFAULT_CLOSED_MS: u64 = 180;

/// A modifier that periodically closes both eyes for a short duration.
///
/// The weight on both eyes stays at `Eye::open_weight` while open and drops
/// to 0 during the closed window. `phase` flips between [`EyePhase::Open`]
/// and [`EyePhase::Closed`] in lockstep so renderers can vary their drawing
/// strategy (e.g. draw a thin horizontal line when closed).
#[derive(Debug, Clone, Copy)]
pub struct Blink {
    /// Milliseconds eyes remain open between blinks at the baseline cadence.
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

    /// Effective open duration after applying `blink_rate_scale`.
    ///
    /// `scale == 0` is a sentinel for "suppress blinks"; callers must check
    /// that case separately, as `u64::MAX` here would still eventually wrap
    /// in `Instant` addition. `SCALE_DEFAULT` (128) returns `open_ms`
    /// unchanged. Higher scales shorten the open window; lower scales
    /// lengthen it.
    fn scaled_open_ms(&self, scale: u8) -> u64 {
        // `scale` is guaranteed > 0 here by the caller; use saturating
        // arithmetic so an enormous `open_ms` paired with a tiny scale
        // can't wrap `u64`.
        self.open_ms
            .saturating_mul(u64::from(SCALE_DEFAULT))
            .checked_div(u64::from(scale))
            .unwrap_or(self.open_ms)
    }
}

impl Default for Blink {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for Blink {
    fn update(&mut self, avatar: &mut Avatar, now: Instant) {
        let rate = avatar.blink_rate_scale;

        // Suppression path: scale == 0 forces eyes open and parks the
        // state machine. The next non-zero scale will resume blinking
        // naturally (re-entering from Uninitialized if we've never run,
        // or from Open which is the phase we left behind).
        if rate == 0 {
            self.state = BlinkState::Open {
                // Park "next transition" in the far future; a subsequent
                // non-zero rate will reschedule before this ever elapses.
                transition_at: now + u64::from(u32::MAX),
            };
            set_both_eyes(avatar, EyePhase::Open, avatar.left_eye.open_weight);
            return;
        }

        let open_ms = self.scaled_open_ms(rate);

        match self.state {
            BlinkState::Uninitialized => {
                // First tick: eyes are currently open; schedule the first blink.
                self.state = BlinkState::Open {
                    transition_at: now + open_ms,
                };
                set_both_eyes(avatar, EyePhase::Open, avatar.left_eye.open_weight);
            }
            BlinkState::Open { transition_at } if now >= transition_at => {
                self.state = BlinkState::Closed {
                    transition_at: now + self.closed_ms,
                };
                set_both_eyes(avatar, EyePhase::Closed, 0);
            }
            BlinkState::Closed { transition_at } if now >= transition_at => {
                self.state = BlinkState::Open {
                    transition_at: now + open_ms,
                };
                set_both_eyes(avatar, EyePhase::Open, avatar.left_eye.open_weight);
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
#[allow(clippy::field_reassign_with_default)]
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

    #[test]
    fn open_weight_caps_reopen_amount() {
        let mut avatar = Avatar::default();
        // Sleepy-style droopy lid: cap the open weight at 55.
        avatar.left_eye.open_weight = 55;
        avatar.right_eye.open_weight = 55;

        let mut blink = Blink::with_timing(100, 20);
        blink.update(&mut avatar, Instant::from_millis(0));
        assert_eq!(avatar.left_eye.weight, 55);

        // Close and reopen -- the reopen still honors the cap.
        blink.update(&mut avatar, Instant::from_millis(100));
        blink.update(&mut avatar, Instant::from_millis(120));
        assert_eq!(avatar.left_eye.phase, EyePhase::Open);
        assert_eq!(avatar.left_eye.weight, 55);
    }

    #[test]
    fn blink_rate_scale_zero_suppresses_blinks() {
        let mut avatar = Avatar::default();
        avatar.blink_rate_scale = 0;

        let mut blink = Blink::with_timing(100, 20);
        // Drive through what would normally be many blinks.
        for ms in 0..1_000 {
            blink.update(&mut avatar, Instant::from_millis(ms));
            assert_eq!(avatar.left_eye.phase, EyePhase::Open, "ms={ms}");
        }
    }

    #[test]
    fn blink_rate_scale_slows_cadence() {
        let mut slow = Avatar::default();
        slow.blink_rate_scale = 64; // half the default speed

        let mut fast = Avatar::default(); // default = SCALE_DEFAULT

        let mut slow_blink = Blink::with_timing(100, 20);
        let mut fast_blink = Blink::with_timing(100, 20);

        // Count transitions over a fixed window.
        let mut slow_blinks = 0;
        let mut fast_blinks = 0;
        let (mut last_slow, mut last_fast) = (EyePhase::Open, EyePhase::Open);

        for ms in 0..=1_000 {
            slow_blink.update(&mut slow, Instant::from_millis(ms));
            fast_blink.update(&mut fast, Instant::from_millis(ms));

            if slow.left_eye.phase == EyePhase::Closed && last_slow == EyePhase::Open {
                slow_blinks += 1;
            }
            if fast.left_eye.phase == EyePhase::Closed && last_fast == EyePhase::Open {
                fast_blinks += 1;
            }
            last_slow = slow.left_eye.phase;
            last_fast = fast.left_eye.phase;
        }

        assert!(
            slow_blinks < fast_blinks,
            "slow_blinks={slow_blinks}, fast_blinks={fast_blinks}"
        );
    }
}
