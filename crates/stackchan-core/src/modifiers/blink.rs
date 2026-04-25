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

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::face::{EyePhase, SCALE_DEFAULT};
use crate::modifier::Modifier;

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

/// Internal state tracking the current blink phase.
///
/// Each phase records the instant it started rather than its scheduled
/// transition time. That way, rate changes mid-phase take effect on the
/// next tick without stranding a stale absolute deadline — we always
/// compute `phase_start + scaled_open_ms(current_rate)` from the
/// current `entity.face.style.blink_rate_scale`.
#[derive(Debug, Clone, Copy)]
enum BlinkState {
    /// Waiting to initialize on the first `update` call. Used so the very
    /// first tick anchors `phase_start` to the caller's actual clock.
    Uninitialized,
    /// Eyes currently open; they close once the phase has run for
    /// `scaled_open_ms(rate)` since `phase_start`.
    Open {
        /// Instant the phase began.
        phase_start: Instant,
    },
    /// Eyes currently closed; they open once the phase has run for
    /// `closed_ms` since `phase_start`.
    Closed {
        /// Instant the phase began.
        phase_start: Instant,
    },
    /// Rate scale is 0 — blinks are suppressed until the rate goes
    /// non-zero again, at which point we transition back to `Open` with
    /// a fresh `phase_start`. Explicit state (rather than parking
    /// `Open` with a far-future deadline) avoids the ~49-day wraparound
    /// hazard and makes the "Surprised → Neutral" recovery path trivial.
    Suppressed,
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
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "Blink",
            description: "Drives both eyes through open→closed→open cycles. Reads \
                          face.style.blink_rate_scale for cadence (0 = blinks suppressed) \
                          and Eye::open_weight for the open-amplitude bound.",
            phase: Phase::Expression,
            priority: 0,
            reads: &[
                Field::BlinkRateScale,
                Field::LeftEyeOpenWeight,
                Field::RightEyeOpenWeight,
            ],
            writes: &[
                Field::LeftEyePhase,
                Field::LeftEyeWeight,
                Field::RightEyePhase,
                Field::RightEyeWeight,
            ],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let rate = entity.face.style.blink_rate_scale;

        // Suppression path: scale == 0 forces eyes open. Stored as an
        // explicit state so a later non-zero scale cleanly resumes the
        // cycle from `Open` rather than relying on a far-future deadline
        // to elapse first.
        if rate == 0 {
            self.state = BlinkState::Suppressed;
            open_both_eyes(entity);
            return;
        }

        let open_ms = self.scaled_open_ms(rate);

        match self.state {
            // First tick, or resuming from suppression: start a fresh
            // open phase anchored to `now`.
            BlinkState::Uninitialized | BlinkState::Suppressed => {
                self.state = BlinkState::Open { phase_start: now };
                open_both_eyes(entity);
            }
            // Open phase: elapsed since phase_start is always compared
            // against the *current* scaled_open_ms, so a rate change
            // mid-open takes effect on the next tick without leaving a
            // stale absolute deadline behind.
            BlinkState::Open { phase_start } => {
                let elapsed = now.saturating_duration_since(phase_start);
                if elapsed >= open_ms {
                    self.state = BlinkState::Closed { phase_start: now };
                    close_both_eyes(entity);
                }
                // Else: still open; no writes required.
            }
            BlinkState::Closed { phase_start } => {
                let elapsed = now.saturating_duration_since(phase_start);
                if elapsed >= self.closed_ms {
                    self.state = BlinkState::Open { phase_start: now };
                    open_both_eyes(entity);
                }
            }
        }
    }
}

/// Open both eyes, each honoring its own `open_weight`. Split per-eye so
/// emotion code can animate the two lids independently (e.g. a wink
/// variant could set one eye's `open_weight` to 0 without Blink
/// clobbering the asymmetry).
const fn open_both_eyes(entity: &mut Entity) {
    entity.face.left_eye.phase = EyePhase::Open;
    entity.face.left_eye.weight = entity.face.left_eye.open_weight;
    entity.face.right_eye.phase = EyePhase::Open;
    entity.face.right_eye.weight = entity.face.right_eye.open_weight;
}

/// Close both eyes. Weight is 0 in both; renderers distinguish closed
/// from almost-open via the `phase` field rather than a weight threshold.
const fn close_both_eyes(entity: &mut Entity) {
    entity.face.left_eye.phase = EyePhase::Closed;
    entity.face.left_eye.weight = 0;
    entity.face.right_eye.phase = EyePhase::Closed;
    entity.face.right_eye.weight = 0;
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::director::{Field, ModifierMeta, Phase};
    use crate::entity::Entity;
    use crate::modifier::Modifier;

    #[test]
    fn first_update_leaves_eyes_open() {
        let mut entity = Entity::default();
        let mut blink = Blink::new();
        entity.tick.now = Instant::from_millis(0);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.phase, EyePhase::Open);
        assert_eq!(entity.face.right_eye.phase, EyePhase::Open);
        assert_eq!(entity.face.left_eye.weight, 100);
    }

    #[test]
    fn blinks_after_open_window_elapses() {
        let mut entity = Entity::default();
        let mut blink = Blink::with_timing(100, 20);

        entity.tick.now = Instant::from_millis(0);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.phase, EyePhase::Open);

        // Just before the transition -- still open.
        entity.tick.now = Instant::from_millis(99);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.phase, EyePhase::Open);

        // At the transition -- eyes close.
        entity.tick.now = Instant::from_millis(100);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.phase, EyePhase::Closed);
        assert_eq!(entity.face.left_eye.weight, 0);
        assert_eq!(entity.face.right_eye.phase, EyePhase::Closed);
    }

    #[test]
    fn reopens_after_closed_window_elapses() {
        let mut entity = Entity::default();
        let mut blink = Blink::with_timing(100, 20);

        // Cycle through: init -> open -> closed -> open again.
        entity.tick.now = Instant::from_millis(0);
        blink.update(&mut entity);
        entity.tick.now = Instant::from_millis(100);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.phase, EyePhase::Closed);

        entity.tick.now = Instant::from_millis(120);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.phase, EyePhase::Open);
        assert_eq!(entity.face.left_eye.weight, 100);
    }

    #[test]
    fn cycle_repeats_indefinitely() {
        let mut entity = Entity::default();
        let mut blink = Blink::with_timing(100, 20);

        // Simulate ~5 full cycles.
        let mut transitions = 0_u32;
        let mut last_phase = EyePhase::Open;
        for ms in 0..=600 {
            entity.tick.now = Instant::from_millis(ms);
            blink.update(&mut entity);
            if entity.face.left_eye.phase != last_phase {
                transitions += 1;
                last_phase = entity.face.left_eye.phase;
            }
        }
        // 600 ms / (100 + 20) = 5 cycles -> 10 transitions (open->closed->open pairs).
        // First tick is an initialization, not a transition.
        assert!(transitions >= 8, "only saw {transitions} transitions");
    }

    #[test]
    fn open_weight_caps_reopen_amount() {
        let mut entity = Entity::default();
        // Sleepy-style droopy lid: cap the open weight at 55.
        entity.face.left_eye.open_weight = 55;
        entity.face.right_eye.open_weight = 55;

        let mut blink = Blink::with_timing(100, 20);
        entity.tick.now = Instant::from_millis(0);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.weight, 55);

        // Close and reopen -- the reopen still honors the cap.
        entity.tick.now = Instant::from_millis(100);
        blink.update(&mut entity);
        entity.tick.now = Instant::from_millis(120);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.phase, EyePhase::Open);
        assert_eq!(entity.face.left_eye.weight, 55);
    }

    #[test]
    fn blink_rate_scale_zero_suppresses_blinks() {
        let mut entity = Entity::default();
        entity.face.style.blink_rate_scale = 0;

        let mut blink = Blink::with_timing(100, 20);
        // Drive through what would normally be many blinks.
        for ms in 0..1_000 {
            entity.tick.now = Instant::from_millis(ms);
            blink.update(&mut entity);
            assert_eq!(entity.face.left_eye.phase, EyePhase::Open, "ms={ms}");
        }
    }

    #[test]
    fn blink_resumes_after_rate_returns_to_nonzero() {
        // Regression for the "Surprised emotion parks Blink for 49 days"
        // bug: previously, rate==0 set an Open state with a far-future
        // transition deadline, and when rate came back to 128 the
        // deadline stayed in the future so blinks never resumed. With
        // the explicit Suppressed state, the first non-zero tick anchors
        // a fresh Open phase and the normal cycle fires within open_ms.
        let mut entity = Entity::default();
        let mut blink = Blink::with_timing(100, 20);

        // Enter suppression.
        entity.face.style.blink_rate_scale = 0;
        entity.tick.now = Instant::from_millis(0);
        blink.update(&mut entity);
        entity.tick.now = Instant::from_millis(500);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.phase, EyePhase::Open, "suppressed");

        // Rate returns to default; blinks should resume within the
        // normal open window relative to the resume instant, not stay
        // parked in the distant future.
        entity.face.style.blink_rate_scale = SCALE_DEFAULT;
        entity.tick.now = Instant::from_millis(500);
        blink.update(&mut entity);
        entity.tick.now = Instant::from_millis(600);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.phase, EyePhase::Closed);
    }

    #[test]
    fn rate_change_mid_open_takes_effect_on_next_tick() {
        // Regression for "stale transition_at when rate changes mid-open":
        // previously, Open stored an absolute transition_at computed at
        // phase entry, so shortening the open window later didn't fire
        // the blink any sooner. With phase_start + current-tick rate,
        // a rate change is observed immediately.
        let mut entity = Entity::default();
        entity.face.style.blink_rate_scale = 64; // slower cadence → longer open
        let mut blink = Blink::with_timing(200, 20);

        entity.tick.now = Instant::from_millis(0);
        blink.update(&mut entity);
        // scaled_open_ms(64) = 200 * 128 / 64 = 400 ms. At ms=200 we're
        // only halfway through the slow open window.
        entity.tick.now = Instant::from_millis(200);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.phase, EyePhase::Open);

        // Speed up to 4x — scaled_open_ms(255) = 200 * 128 / 255 ≈ 100 ms.
        // We're already past that window since phase_start=0, ms=200.
        entity.face.style.blink_rate_scale = 255;
        entity.tick.now = Instant::from_millis(200);
        blink.update(&mut entity);
        assert_eq!(
            entity.face.left_eye.phase,
            EyePhase::Closed,
            "faster rate should fire the blink on the same tick we crossed the new window"
        );
    }

    #[test]
    fn open_weight_is_per_eye() {
        // Regression for "Blink uses left_eye.open_weight for both eyes":
        // asymmetric open_weights (e.g. a wink) must survive Blink's
        // re-open writes rather than being clobbered by whatever the
        // left eye happens to hold.
        let mut entity = Entity::default();
        entity.face.left_eye.open_weight = 100;
        entity.face.right_eye.open_weight = 30; // partially closed right eye

        let mut blink = Blink::with_timing(100, 20);
        entity.tick.now = Instant::from_millis(0);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.weight, 100);
        assert_eq!(
            entity.face.right_eye.weight, 30,
            "right eye's open_weight preserved"
        );

        // Close + reopen preserves the asymmetry.
        entity.tick.now = Instant::from_millis(100);
        blink.update(&mut entity);
        entity.tick.now = Instant::from_millis(120);
        blink.update(&mut entity);
        assert_eq!(entity.face.left_eye.phase, EyePhase::Open);
        assert_eq!(entity.face.left_eye.weight, 100);
        assert_eq!(entity.face.right_eye.weight, 30);
    }

    #[test]
    fn blink_rate_scale_slows_cadence() {
        let mut slow = Entity::default();
        slow.face.style.blink_rate_scale = 64; // half the default speed

        let mut fast = Entity::default(); // default = SCALE_DEFAULT

        let mut slow_blink = Blink::with_timing(100, 20);
        let mut fast_blink = Blink::with_timing(100, 20);

        let mut slow_blinks = 0;
        let mut fast_blinks = 0;
        let (mut last_slow, mut last_fast) = (EyePhase::Open, EyePhase::Open);

        for ms in 0..=1_000 {
            slow.tick.now = Instant::from_millis(ms);
            slow_blink.update(&mut slow);
            fast.tick.now = Instant::from_millis(ms);
            fast_blink.update(&mut fast);

            if slow.face.left_eye.phase == EyePhase::Closed && last_slow == EyePhase::Open {
                slow_blinks += 1;
            }
            if fast.face.left_eye.phase == EyePhase::Closed && last_fast == EyePhase::Open {
                fast_blinks += 1;
            }
            last_slow = slow.face.left_eye.phase;
            last_fast = fast.face.left_eye.phase;
        }

        assert!(
            slow_blinks < fast_blinks,
            "slow_blinks={slow_blinks}, fast_blinks={fast_blinks}"
        );
    }
}
