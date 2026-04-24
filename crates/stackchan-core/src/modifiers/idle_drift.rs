//! Idle eye drift: periodically shifts both eyes a small random-looking amount
//! to avoid the uncanny "perfectly centered forever" stare.
//!
//! v0.1.0 uses a deterministic pseudo-random sequence (xorshift32) seeded at
//! construction so sim tests are reproducible. A future release may swap in
//! a hardware-RNG-backed source for the firmware build.

use super::Modifier;
use crate::avatar::Avatar;
use crate::clock::Instant;
use core::num::NonZeroU32;

/// Default xorshift32 seed used by [`IdleDrift::new`]. The `.unwrap()`
/// is const-evaluated against a compile-time non-zero literal, so
/// there's no runtime branch.
#[allow(
    clippy::unwrap_used,
    reason = "const-evaluated against a non-zero literal: unwrap can't fire at runtime"
)]
const DEFAULT_SEED: NonZeroU32 = NonZeroU32::new(0x1234_5678).unwrap();

/// Default interval between drifts, in milliseconds.
const DEFAULT_INTERVAL_MS: u64 = 4_000;
/// Maximum horizontal drift in either direction, in pixels.
const DEFAULT_MAX_X: i32 = 6;
/// Maximum vertical drift in either direction, in pixels.
const DEFAULT_MAX_Y: i32 = 4;

/// Modifier that occasionally offsets both eyes' centers by a small amount.
/// The offset is removed on the next tick, so the eyes return to baseline
/// between drifts rather than walking off the face.
#[derive(Debug, Clone, Copy)]
pub struct IdleDrift {
    /// Milliseconds between successive drifts.
    interval_ms: u64,
    /// Maximum horizontal drift in either direction.
    max_x: i32,
    /// Maximum vertical drift in either direction.
    max_y: i32,
    /// xorshift32 state.
    rng_state: u32,
    /// Monotonic time of the next drift; set on first tick.
    next_drift_at: Option<Instant>,
    /// Offset applied on the previous drift; used to undo before the next
    /// drift so drifts don't accumulate.
    last_offset: (i32, i32),
}

impl IdleDrift {
    /// Construct with default timing + a fixed seed so sim tests are
    /// reproducible. Firmware overrides the seed at boot.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_seed(DEFAULT_SEED)
    }

    /// Construct with a custom xorshift32 seed.
    ///
    /// Accepts [`NonZeroU32`] at the type level rather than silently
    /// substituting a default when the caller passes zero — a zero
    /// seed would leave xorshift32 stuck at zero and drift would never
    /// fire.
    #[must_use]
    pub const fn with_seed(seed: NonZeroU32) -> Self {
        Self {
            interval_ms: DEFAULT_INTERVAL_MS,
            max_x: DEFAULT_MAX_X,
            max_y: DEFAULT_MAX_Y,
            rng_state: seed.get(),
            next_drift_at: None,
            last_offset: (0, 0),
        }
    }

    /// Advance the xorshift32 state and return the next pseudo-random `u32`.
    const fn next_u32(&mut self) -> u32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        x
    }

    /// Produce a signed offset in `[-max, +max]` from the RNG.
    fn rand_offset(&mut self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        // `max` is >0, so max*2+1 is always positive; casting the i32 into
        // u32 via `cast_unsigned` is defined and lossless in this branch.
        let span = max.saturating_mul(2).saturating_add(1).cast_unsigned();
        let raw = self.next_u32() % span.max(1);
        // `raw` is in [0, span), so `raw` - `max` fits in i32.
        #[allow(clippy::cast_possible_wrap)]
        let offset = raw as i32 - max;
        offset
    }
}

impl Default for IdleDrift {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for IdleDrift {
    fn update(&mut self, avatar: &mut Avatar, now: Instant) {
        let due = match self.next_drift_at {
            None => {
                // Schedule the first drift `interval_ms` from now.
                self.next_drift_at = Some(now + self.interval_ms);
                return;
            }
            Some(t) => now >= t,
        };
        if !due {
            return;
        }

        // Undo previous offset before applying a new one.
        let (dx, dy) = self.last_offset;
        avatar.left_eye.center.x -= dx;
        avatar.left_eye.center.y -= dy;
        avatar.right_eye.center.x -= dx;
        avatar.right_eye.center.y -= dy;

        // Apply a new drift.
        let nx = self.rand_offset(self.max_x);
        let ny = self.rand_offset(self.max_y);
        avatar.left_eye.center.x += nx;
        avatar.left_eye.center.y += ny;
        avatar.right_eye.center.x += nx;
        avatar.right_eye.center.y += ny;

        self.last_offset = (nx, ny);
        self.next_drift_at = Some(now + self.interval_ms);
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test literals are compile-time non-zero; the unwrap can't fire"
)]
mod tests {
    use super::*;
    use crate::avatar::Avatar;

    #[test]
    fn first_tick_schedules_first_drift() {
        let mut avatar = Avatar::default();
        let baseline_x = avatar.left_eye.center.x;
        let mut drift = IdleDrift::new();
        drift.update(&mut avatar, Instant::from_millis(0));
        // No drift applied yet on the scheduling tick.
        assert_eq!(avatar.left_eye.center.x, baseline_x);
    }

    #[test]
    fn drift_applies_at_interval() {
        let mut avatar = Avatar::default();
        let baseline = (avatar.left_eye.center.x, avatar.left_eye.center.y);
        let mut drift = IdleDrift::with_seed(NonZeroU32::new(42).unwrap());

        drift.update(&mut avatar, Instant::from_millis(0));
        drift.update(&mut avatar, Instant::from_millis(DEFAULT_INTERVAL_MS));
        // After the interval, the eye position must differ from baseline by
        // at most ±max in each axis.
        let dx = avatar.left_eye.center.x - baseline.0;
        let dy = avatar.left_eye.center.y - baseline.1;
        assert!(dx.abs() <= DEFAULT_MAX_X);
        assert!(dy.abs() <= DEFAULT_MAX_Y);
    }

    #[test]
    fn drifts_do_not_accumulate() {
        let mut avatar = Avatar::default();
        let baseline = (avatar.left_eye.center.x, avatar.left_eye.center.y);
        let mut drift = IdleDrift::with_seed(NonZeroU32::new(42).unwrap());

        for i in 0..10 {
            drift.update(&mut avatar, Instant::from_millis(i * DEFAULT_INTERVAL_MS));
        }
        // After many drifts, eye must still be within ±max of baseline, not
        // walked off-screen.
        let dx = avatar.left_eye.center.x - baseline.0;
        let dy = avatar.left_eye.center.y - baseline.1;
        assert!(dx.abs() <= DEFAULT_MAX_X, "dx={dx}");
        assert!(dy.abs() <= DEFAULT_MAX_Y, "dy={dy}");
    }

    #[test]
    fn seeded_rng_is_deterministic() {
        let mut a = IdleDrift::with_seed(NonZeroU32::new(42).unwrap());
        let mut b = IdleDrift::with_seed(NonZeroU32::new(42).unwrap());
        for _ in 0..100 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }
}
