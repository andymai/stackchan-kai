//! Breath modifier: gently shifts all features up and down on a sine-like cycle.
//!
//! v0.1.0 uses a coarse triangle-wave approximation to keep the core crate
//! `no_std` + dependency-free (no `libm`). The wave period defaults to ~6 s
//! with a 2-pixel peak-to-peak amplitude, which reads as a subtle breathing
//! animation at 30 FPS.

use super::Modifier;
use crate::avatar::Avatar;
use crate::clock::Instant;

/// Default full breath cycle (inhale + exhale), in milliseconds.
const DEFAULT_CYCLE_MS: u64 = 6_000;
/// Default peak-to-peak vertical amplitude, in pixels.
const DEFAULT_AMPLITUDE_PX: i32 = 2;

/// A modifier that applies a slow rise-and-fall vertical offset to every
/// facial feature (both eyes + mouth), evoking breathing.
#[derive(Debug, Clone, Copy)]
pub struct Breath {
    /// Milliseconds per complete breath cycle.
    cycle_ms: u64,
    /// Peak-to-peak amplitude in pixels.
    amplitude_px: i32,
    /// Offset applied on the previous tick; used to diff-and-undo so the
    /// modifier composes cleanly with modifiers that set absolute positions.
    last_offset_px: i32,
}

impl Breath {
    /// Default breath: 6 s cycle, 2 px amplitude.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cycle_ms: DEFAULT_CYCLE_MS,
            amplitude_px: DEFAULT_AMPLITUDE_PX,
            last_offset_px: 0,
        }
    }

    /// Construct a `Breath` with custom cycle and amplitude.
    #[must_use]
    pub const fn with_params(cycle_ms: u64, amplitude_px: i32) -> Self {
        Self {
            cycle_ms,
            amplitude_px,
            last_offset_px: 0,
        }
    }

    /// Compute the current offset for time `now` as an integer-pixel triangle
    /// wave in `[-amplitude/2, +amplitude/2]`.
    fn offset_at(&self, now: Instant) -> i32 {
        if self.cycle_ms == 0 || self.amplitude_px == 0 {
            return 0;
        }
        let phase = now.as_millis() % self.cycle_ms;
        let half = self.cycle_ms / 2;
        // Ascend in the first half, descend in the second half.
        let ascending = phase < half;
        let within_half = if ascending { phase } else { phase - half };
        let half_i64 = i64::try_from(half).unwrap_or(i64::MAX);
        let within_i64 = i64::try_from(within_half).unwrap_or(i64::MAX);
        // Map within_half in 0..half to 0..amplitude linearly, then shift
        // down by half-amplitude so the wave oscillates around 0.
        let scaled = i64::from(self.amplitude_px) * within_i64 / half_i64.max(1);
        #[allow(clippy::cast_possible_truncation)]
        let progress = scaled as i32;
        if ascending {
            progress - self.amplitude_px / 2
        } else {
            self.amplitude_px / 2 - progress
        }
    }
}

impl Default for Breath {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for Breath {
    fn update(&mut self, avatar: &mut Avatar, now: Instant) {
        let target = self.offset_at(now);
        let delta = target - self.last_offset_px;
        if delta == 0 {
            return;
        }
        avatar.left_eye.center.y += delta;
        avatar.right_eye.center.y += delta;
        avatar.mouth.center.y += delta;
        self.last_offset_px = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avatar::Avatar;

    #[test]
    fn zero_amplitude_is_noop() {
        let mut avatar = Avatar::default();
        let baseline_y = avatar.left_eye.center.y;
        let mut breath = Breath::with_params(1000, 0);
        for ms in 0..2000 {
            breath.update(&mut avatar, Instant::from_millis(ms));
        }
        assert_eq!(avatar.left_eye.center.y, baseline_y);
    }

    #[test]
    fn offset_stays_within_amplitude() {
        let breath = Breath::with_params(1000, 4);
        // Sample across an entire cycle; offset must never exceed amplitude/2.
        for ms in 0..1000 {
            let o = breath.offset_at(Instant::from_millis(ms));
            assert!((-2..=2).contains(&o), "offset {o} at ms {ms} out of range");
        }
    }

    #[test]
    fn composes_across_ticks_without_drift() {
        let mut avatar = Avatar::default();
        let baseline_y = avatar.left_eye.center.y;
        let mut breath = Breath::with_params(1000, 4);

        // Drive through two complete cycles; at the end of every cycle the
        // offset returns to the starting phase so there should be zero net
        // drift.
        for ms in 0..=2000 {
            breath.update(&mut avatar, Instant::from_millis(ms));
        }
        // At ms=2000 we're back at phase 0 -- offset is -amplitude/2.
        // The key invariant is "no cumulative drift" rather than "exactly zero".
        let final_y = avatar.left_eye.center.y;
        let drift = (final_y - baseline_y).abs();
        assert!(drift <= 2, "drift {drift}px exceeds half-amplitude");
    }
}
