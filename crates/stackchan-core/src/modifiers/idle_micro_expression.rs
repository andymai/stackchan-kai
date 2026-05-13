//! [`IdleMicroExpression`] — small mouth-center perturbations at
//! random idle intervals so a long-quiet face doesn't read as a frozen
//! mask.
//!
//! Complement to [`super::IdleDrift`]: where that modifier jitters the
//! eyes, this one nudges the mouth a few pixels vertically. The two
//! schedules are independent (different seeds) so the avatar shows
//! occasional asymmetric liveness rather than synchronised twitches.
//!
//! ## Why mouth-only
//!
//! Upstream's `IdleExpressionModifier` adds three perturbation kinds:
//! eye offset, mouth rotation, and mouth vertical drift. `IdleDrift`
//! already covers the eye case; mouth rotation needs a renderer
//! primitive we don't have. Mouth vertical drift is the surviving
//! surface — small, cheap, and visibly alive.
//!
//! ## Phase + priority
//!
//! Runs in [`Phase::Expression`] at priority `1`, after
//! [`super::Breath`] (priority `0`) writes its own `mouth.center.y`
//! delta. Both modifiers track their own `last_offset` so the
//! additive composition undoes cleanly on each step.

use core::num::NonZeroU32;

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::modifier::Modifier;

/// Minimum interval between fires, in ms.
pub const DEFAULT_INTERVAL_MIN_MS: u64 = 2_000;
/// Maximum interval between fires, in ms.
pub const DEFAULT_INTERVAL_MAX_MS: u64 = 6_000;
/// Maximum vertical perturbation magnitude in pixels, in either direction.
pub const DEFAULT_MAX_MOUTH_Y_PX: i32 = 3;

/// Default xorshift32 seed. Different from the seeds used by
/// [`super::IdleDrift`] / [`super::IdleHeadDrift`] / [`super::Soliloquy`]
/// so the schedules don't synchronise out of the box.
#[allow(
    clippy::unwrap_used,
    reason = "const-evaluated against a non-zero literal: unwrap can't fire at runtime"
)]
const DEFAULT_SEED: NonZeroU32 = NonZeroU32::new(0xCAFE_F00D).unwrap();

/// Modifier that occasionally offsets `mouth.center.y` by a small random amount.
///
/// Undoes the previous offset before applying a new one so successive
/// fires don't accumulate. See module docs for the composition story
/// with [`super::Breath`].
#[derive(Debug, Clone, Copy)]
pub struct IdleMicroExpression {
    /// Lower bound on the random interval.
    interval_min_ms: u64,
    /// Upper bound on the random interval.
    interval_max_ms: u64,
    /// Maximum perturbation magnitude per fire.
    max_mouth_y_px: i32,
    /// xorshift32 state.
    rng_state: u32,
    /// Wall-clock instant at which the next perturbation fires.
    /// `None` until the first tick anchors the schedule.
    next_fire_at: Option<Instant>,
    /// Vertical offset applied on the previous fire; subtracted from
    /// `mouth.center.y` before applying a new offset so successive
    /// fires don't accumulate.
    last_y_offset: i32,
}

impl IdleMicroExpression {
    /// Construct with default timing + a fixed seed so sim tests are
    /// reproducible. Firmware overrides the seed at boot via
    /// [`Self::with_seed`].
    #[must_use]
    pub const fn new() -> Self {
        Self::with_seed(DEFAULT_SEED)
    }

    /// Construct with a custom xorshift32 seed.
    #[must_use]
    pub const fn with_seed(seed: NonZeroU32) -> Self {
        Self {
            interval_min_ms: DEFAULT_INTERVAL_MIN_MS,
            interval_max_ms: DEFAULT_INTERVAL_MAX_MS,
            max_mouth_y_px: DEFAULT_MAX_MOUTH_Y_PX,
            rng_state: seed.get(),
            next_fire_at: None,
            last_y_offset: 0,
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
        let span = max.saturating_mul(2).saturating_add(1).cast_unsigned();
        let raw = self.next_u32() % span.max(1);
        #[allow(
            clippy::cast_possible_wrap,
            reason = "raw is in [0, 2*max+1) which fits in i32 for any reasonable max"
        )]
        let raw_i32 = raw as i32;
        raw_i32 - max
    }

    /// Pick the next fire delay in `[interval_min_ms, interval_max_ms]`.
    fn next_delay_ms(&mut self) -> u64 {
        if self.interval_max_ms <= self.interval_min_ms {
            return self.interval_min_ms;
        }
        let span = self.interval_max_ms - self.interval_min_ms;
        let r = u64::from(self.next_u32());
        self.interval_min_ms + (r % (span + 1))
    }
}

impl Default for IdleMicroExpression {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for IdleMicroExpression {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "IdleMicroExpression",
            description: "Random small vertical perturbation of mouth.center.y every 2-6 s; \
                          undoes its previous offset before each new fire so the mouth \
                          doesn't walk off the face.",
            phase: Phase::Expression,
            priority: 1,
            reads: &[Field::MouthCenter],
            writes: &[Field::MouthCenter],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        match self.next_fire_at {
            None => {
                let delay = self.next_delay_ms();
                self.next_fire_at = Some(now + delay);
                return;
            }
            Some(t) if now < t => return,
            Some(_) => {}
        }

        entity.face.mouth.center.y -= self.last_y_offset;
        let dy = self.rand_offset(self.max_mouth_y_px);
        entity.face.mouth.center.y += dy;
        self.last_y_offset = dy;

        let delay = self.next_delay_ms();
        self.next_fire_at = Some(now + delay);
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test literals are compile-time non-zero; the unwrap can't fire"
)]
mod tests {
    use super::*;
    use crate::Entity;

    fn at(ms: u64) -> Entity {
        let mut e = Entity::default();
        e.tick.now = Instant::from_millis(ms);
        e
    }

    #[test]
    fn first_tick_only_schedules() {
        let mut entity = at(0);
        let baseline_y = entity.face.mouth.center.y;
        let mut m = IdleMicroExpression::new();
        m.update(&mut entity);
        assert_eq!(entity.face.mouth.center.y, baseline_y);
    }

    #[test]
    fn perturbation_lands_in_range() {
        let mut entity = at(0);
        let baseline_y = entity.face.mouth.center.y;
        let mut m = IdleMicroExpression::with_seed(NonZeroU32::new(42).unwrap());
        m.update(&mut entity);
        entity.tick.now = Instant::from_millis(DEFAULT_INTERVAL_MAX_MS + 1);
        m.update(&mut entity);
        let dy = entity.face.mouth.center.y - baseline_y;
        assert!(dy.abs() <= DEFAULT_MAX_MOUTH_Y_PX, "dy={dy}");
    }

    #[test]
    fn offsets_do_not_accumulate_across_many_fires() {
        let mut entity = at(0);
        let baseline_y = entity.face.mouth.center.y;
        let mut m = IdleMicroExpression::with_seed(NonZeroU32::new(7).unwrap());
        for i in 0..20 {
            entity.tick.now = Instant::from_millis(i * DEFAULT_INTERVAL_MAX_MS);
            m.update(&mut entity);
        }
        let dy = entity.face.mouth.center.y - baseline_y;
        assert!(
            dy.abs() <= DEFAULT_MAX_MOUTH_Y_PX,
            "drift accumulated: dy={dy}"
        );
    }

    #[test]
    fn ticks_before_due_do_not_perturb() {
        let mut entity = at(0);
        let mut m = IdleMicroExpression::new();
        m.update(&mut entity);
        let baseline_y = entity.face.mouth.center.y;
        for ms in 1..DEFAULT_INTERVAL_MIN_MS {
            entity.tick.now = Instant::from_millis(ms);
            m.update(&mut entity);
        }
        assert_eq!(entity.face.mouth.center.y, baseline_y);
    }
}
