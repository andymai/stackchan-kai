//! Headless simulator for `stackchan-core`.
//!
//! Provides a [`FakeClock`] that advances under manual control, letting
//! tests drive [`Modifier`]s through precise time sequences without
//! hardware or threads.
//!
//! [`Modifier`]: stackchan_core::Modifier

#![deny(unsafe_code)]

use core::cell::Cell;
use stackchan_core::{Clock, Instant};

/// A [`Clock`] whose current time is set explicitly by tests.
///
/// Unlike a wall-clock source, `FakeClock` never drifts and never advances
/// on its own. Tests call [`FakeClock::advance`] or
/// [`FakeClock::set`] between assertions.
#[derive(Debug, Default)]
pub struct FakeClock {
    /// Current time; uses `Cell` so `Clock::now` can take `&self`.
    now: Cell<Instant>,
}

impl FakeClock {
    /// Construct a new `FakeClock` at `Instant::ZERO`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            now: Cell::new(Instant::ZERO),
        }
    }

    /// Advance the clock by `delta_ms` milliseconds.
    pub fn advance(&self, delta_ms: u64) {
        self.now.set(self.now.get() + delta_ms);
    }

    /// Set the clock to an absolute instant. Callers are responsible for
    /// monotonicity; this is a test helper that trusts the test author.
    pub fn set(&self, to: Instant) {
        self.now.set(to);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.now.get()
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use stackchan_core::modifiers::{Blink, Breath, IdleDrift};
    use stackchan_core::{Avatar, EyePhase, Modifier};

    /// End-to-end: drive a Blink + Breath + `IdleDrift` stack for 60 simulated
    /// seconds at 30 FPS and verify the avatar never enters a nonsensical
    /// state (e.g. eyes walking off-screen, weight out of range).
    #[test]
    fn sixty_second_composition_is_stable() {
        let clock = FakeClock::new();
        let mut avatar = Avatar::default();
        let mut blink = Blink::new();
        let mut breath = Breath::new();
        let mut drift = IdleDrift::with_seed(0xDEAD_BEEF);

        let tick_ms = 33; // ~30 FPS
        let total_ticks = 60_000 / tick_ms;

        for _ in 0..total_ticks {
            blink.update(&mut avatar, clock.now());
            breath.update(&mut avatar, clock.now());
            drift.update(&mut avatar, clock.now());

            // Invariants that must hold every frame:
            assert!(avatar.left_eye.weight <= 100);
            assert!(avatar.right_eye.weight <= 100);
            // Framebuffer is 320x240; eyes must stay reasonably on-face.
            assert!(
                (0..320).contains(&avatar.left_eye.center.x),
                "left eye x = {}",
                avatar.left_eye.center.x
            );
            assert!(
                (0..320).contains(&avatar.right_eye.center.x),
                "right eye x = {}",
                avatar.right_eye.center.x
            );

            clock.advance(tick_ms);
        }
    }

    #[test]
    fn blink_frequency_over_one_minute() {
        let clock = FakeClock::new();
        let mut avatar = Avatar::default();
        let mut blink = Blink::new();

        let tick_ms = 10;
        let total_ticks = 60_000 / tick_ms;

        let mut blink_count = 0_u32;
        let mut prev_phase = EyePhase::Open;

        for _ in 0..total_ticks {
            blink.update(&mut avatar, clock.now());
            if avatar.left_eye.phase == EyePhase::Closed && prev_phase == EyePhase::Open {
                blink_count += 1;
            }
            prev_phase = avatar.left_eye.phase;
            clock.advance(tick_ms);
        }

        // Default timing is ~5.2s open + 180ms closed = ~11 blinks per minute.
        // Allow wide tolerance for the approximate nature of the state machine.
        assert!(
            (9..=13).contains(&blink_count),
            "expected ~11 blinks in 60s, saw {blink_count}"
        );
    }
}
