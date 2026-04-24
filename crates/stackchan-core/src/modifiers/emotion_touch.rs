//! `EmotionTouch`: advances `Avatar::emotion` on explicit user input
//! and pins the chosen emotion for a configurable hold window.
//!
//! ## Coordination with `EmotionCycle`
//!
//! A "tap" (in the hardware sense: a FT6336U rising-edge) translates
//! into a single [`EmotionTouch::tap`] call. The next [`Modifier::update`]
//! tick then:
//!
//! 1. advances `Avatar::emotion` to the next variant in
//!    [`EMOTION_ORDER`];
//! 2. sets `Avatar::manual_until` to `now + MANUAL_HOLD_MS` (see
//!    [`MANUAL_HOLD_MS`]) so [`super::EmotionCycle`] stands down;
//! 3. clears an expired `manual_until` on any later tick, handing
//!    autonomy back to `EmotionCycle`.
//!
//! The tap event itself is *not* stored with a timestamp — the modifier
//! just remembers "a tap is pending." The hardware polling task can
//! signal a tap at any time between render ticks; the avatar state only
//! changes when the render loop picks it up on the next `update` call.
//!
//! ## Sim-testability
//!
//! Because `tap()` is an ordinary method (not a signal/channel), sim
//! tests drive the modifier the same way firmware does: call `tap()`,
//! then call `update(avatar, now)` and assert on the resulting
//! `avatar.emotion` + `avatar.manual_until`.

use super::Modifier;
use crate::avatar::Avatar;
use crate::clock::Instant;
use crate::emotion::Emotion;

/// How long a tap pins the chosen emotion, in milliseconds.
///
/// 30 s feels intentional without being permanent: long enough for the
/// eased `EmotionStyle` transition to read visually + for the user to
/// notice their tap stuck, short enough that Stack-chan resumes its
/// autonomous cycle before it seems "frozen."
pub const MANUAL_HOLD_MS: u64 = 30_000;

/// Order in which [`EmotionTouch`] cycles through emotions on each tap.
///
/// Defined independently of [`super::EmotionCycle::DEFAULT_SEQUENCE`] so
/// touch cycling can use a different ordering from autonomous cycling
/// if a future tune-up wants it. Currently identical to the autonomy
/// order for consistency.
pub const EMOTION_ORDER: [Emotion; 5] = [
    Emotion::Neutral,
    Emotion::Happy,
    Emotion::Sleepy,
    Emotion::Surprised,
    Emotion::Sad,
];

/// Modifier that advances `Avatar::emotion` on each queued tap and
/// gates `EmotionCycle` by writing `Avatar::manual_until`.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmotionTouch {
    /// `true` after [`Self::tap`] was called and before the next
    /// `update` consumed it. The modifier is edge-triggered: a held
    /// finger doesn't re-fire as long as the hardware task only
    /// publishes rising edges.
    pending_tap: bool,
}

impl EmotionTouch {
    /// Construct a modifier with no pending tap and no in-flight hold.
    #[must_use]
    pub const fn new() -> Self {
        Self { pending_tap: false }
    }

    /// Queue a tap to be applied on the next `update` tick. Idempotent
    /// within a single render frame: multiple `tap()` calls between
    /// consecutive `update`s collapse to a single emotion advance.
    pub const fn tap(&mut self) {
        self.pending_tap = true;
    }
}

/// Next emotion in the touch-cycle order.
///
/// Pattern-matches on every [`Emotion`] variant explicitly. `Emotion`
/// is `#[non_exhaustive]`, but within the defining crate exhaustiveness
/// is still enforced, so adding a new variant produces a compile
/// error here — a deliberate hint to slot the new variant into the
/// cycle.
const fn next_emotion(current: Emotion) -> Emotion {
    match current {
        Emotion::Neutral => Emotion::Happy,
        Emotion::Happy => Emotion::Sleepy,
        Emotion::Sleepy => Emotion::Surprised,
        Emotion::Surprised => Emotion::Sad,
        Emotion::Sad => Emotion::Neutral,
    }
}

impl Modifier for EmotionTouch {
    fn update(&mut self, avatar: &mut Avatar, now: Instant) {
        if self.pending_tap {
            self.pending_tap = false;
            avatar.emotion = next_emotion(avatar.emotion);
            avatar.manual_until = Some(now + MANUAL_HOLD_MS);
            return;
        }

        // Clear an expired hold so autonomous drivers know the user's
        // done. Running *every* tick (not just on taps) ensures the
        // handoff happens even if no new touch events arrive.
        if let Some(until) = avatar.manual_until
            && now >= until
        {
            avatar.manual_until = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_tap_advances_emotion_and_sets_hold() {
        let mut avatar = Avatar::default();
        assert_eq!(avatar.emotion, Emotion::Neutral);

        let mut touch = EmotionTouch::new();
        touch.tap();
        touch.update(&mut avatar, Instant::from_millis(1_000));

        assert_eq!(avatar.emotion, Emotion::Happy);
        assert_eq!(
            avatar.manual_until,
            Some(Instant::from_millis(1_000 + MANUAL_HOLD_MS)),
        );
    }

    #[test]
    fn repeated_taps_cycle_through_order() {
        let mut avatar = Avatar::default();
        let mut touch = EmotionTouch::new();

        for (i, expected) in [
            Emotion::Happy,
            Emotion::Sleepy,
            Emotion::Surprised,
            Emotion::Sad,
            Emotion::Neutral,
            Emotion::Happy,
        ]
        .into_iter()
        .enumerate()
        {
            touch.tap();
            touch.update(&mut avatar, Instant::from_millis((i as u64) * 10));
            assert_eq!(avatar.emotion, expected, "step {i}");
        }
    }

    #[test]
    fn update_without_tap_is_a_noop_on_emotion() {
        let mut avatar = Avatar {
            emotion: Emotion::Happy,
            ..Avatar::default()
        };
        let mut touch = EmotionTouch::new();

        for step in 0..10 {
            touch.update(&mut avatar, Instant::from_millis(step * 100));
        }
        assert_eq!(avatar.emotion, Emotion::Happy);
        assert!(avatar.manual_until.is_none());
    }

    #[test]
    fn held_finger_does_not_re_fire() {
        // The firmware task only publishes rising-edge taps, so from
        // EmotionTouch's perspective a held finger is just one tap.
        let mut avatar = Avatar::default();
        let mut touch = EmotionTouch::new();

        touch.tap();
        touch.update(&mut avatar, Instant::from_millis(0));
        assert_eq!(avatar.emotion, Emotion::Happy);

        // Simulate 500 ms of ticks without another tap signal — emotion
        // must stay pinned.
        for step in 1..=30 {
            touch.update(&mut avatar, Instant::from_millis(step * 16));
        }
        assert_eq!(avatar.emotion, Emotion::Happy);
    }

    #[test]
    fn expired_hold_is_cleared() {
        let mut avatar = Avatar::default();
        let mut touch = EmotionTouch::new();

        touch.tap();
        touch.update(&mut avatar, Instant::from_millis(1_000));
        let hold_ends = 1_000 + MANUAL_HOLD_MS;
        assert_eq!(avatar.manual_until, Some(Instant::from_millis(hold_ends)));

        // Still inside the hold window.
        touch.update(&mut avatar, Instant::from_millis(hold_ends - 1));
        assert!(avatar.manual_until.is_some(), "hold must still be active");

        // At the deadline.
        touch.update(&mut avatar, Instant::from_millis(hold_ends));
        assert!(
            avatar.manual_until.is_none(),
            "hold ends exactly at `now >= until`",
        );
    }

    #[test]
    fn tap_during_active_hold_extends_it() {
        let mut avatar = Avatar::default();
        let mut touch = EmotionTouch::new();

        touch.tap();
        touch.update(&mut avatar, Instant::from_millis(0));
        assert_eq!(
            avatar.manual_until,
            Some(Instant::from_millis(MANUAL_HOLD_MS))
        );

        // Second tap 5 s later: emotion advances again and the hold
        // deadline moves forward to 5_000 + MANUAL_HOLD_MS.
        touch.tap();
        touch.update(&mut avatar, Instant::from_millis(5_000));
        assert_eq!(avatar.emotion, Emotion::Sleepy);
        assert_eq!(
            avatar.manual_until,
            Some(Instant::from_millis(5_000 + MANUAL_HOLD_MS)),
        );
    }
}
