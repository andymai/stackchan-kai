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
    fn fine_grained_ticks_with_baseline_drift_still_undo_cleanly() {
        // Step at 1 ms granularity over a long span. Between fires
        // we mutate `mouth.center.y` from "outside" to mimic Breath
        // applying its own offset; the modifier must still remove
        // only *its* prior contribution and leave the baseline drift
        // untouched.
        let mut entity = at(0);
        let baseline = entity.face.mouth.center.y;
        let mut m = IdleMicroExpression::with_seed(NonZeroU32::new(13).unwrap());

        let mut external_drift: i32 = 0;
        let mut last_module_dy: i32 = 0;

        for ms in 0..=DEFAULT_INTERVAL_MAX_MS * 5 {
            // "Breath-like" external offset advances each ms.
            let next_drift = i32::try_from(ms / 100).unwrap_or(i32::MAX) % 5 - 2;
            entity.face.mouth.center.y += next_drift - external_drift;
            external_drift = next_drift;

            // What the modifier had contributed before this tick.
            let module_before = entity.face.mouth.center.y - baseline - external_drift;
            assert_eq!(module_before, last_module_dy);

            entity.tick.now = Instant::from_millis(ms);
            m.update(&mut entity);

            // What the modifier contributed after this tick.
            let module_after = entity.face.mouth.center.y - baseline - external_drift;
            assert!(
                module_after.abs() <= DEFAULT_MAX_MOUTH_Y_PX,
                "modifier offset escaped bound at ms={ms}: {module_after}"
            );
            last_module_dy = module_after;
        }
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

    #[test]
    fn same_seed_produces_lockstep_mouth_offsets() {
        // Two instances seeded identically must walk in step over a
        // long-enough horizon to cover several fires (the schedule
        // itself is RNG-driven, so the y values land only at firing
        // ticks — but the cumulative `mouth.center.y` must match at
        // every shared tick).
        let seed = NonZeroU32::new(0x1234_5678).unwrap();
        let mut a_entity = at(0);
        let mut b_entity = at(0);
        let mut a = IdleMicroExpression::with_seed(seed);
        let mut b = IdleMicroExpression::with_seed(seed);
        for ms in 0..=DEFAULT_INTERVAL_MAX_MS * 6 {
            a_entity.tick.now = Instant::from_millis(ms);
            b_entity.tick.now = Instant::from_millis(ms);
            a.update(&mut a_entity);
            b.update(&mut b_entity);
            assert_eq!(
                a_entity.face.mouth.center.y, b_entity.face.mouth.center.y,
                "diverged at ms={ms}"
            );
        }
    }

    #[test]
    fn distinct_seeds_diverge_within_a_few_fires() {
        // Different seeds must produce different y values at some
        // point — otherwise the seed parameter is a no-op and the
        // schedules would synchronise across the avatar family
        // (Breath / Blink / Soliloquy / IdleDrift / IdleHeadDrift),
        // which would defeat the whole reason this modifier carries
        // its own [`DEFAULT_SEED`] distinct from the rest.
        let mut a_entity = at(0);
        let mut b_entity = at(0);
        let mut a = IdleMicroExpression::with_seed(NonZeroU32::new(0xAAAA_AAAA).unwrap());
        let mut b = IdleMicroExpression::with_seed(NonZeroU32::new(0x5555_5555).unwrap());
        let mut diverged = false;
        for ms in 0..=DEFAULT_INTERVAL_MAX_MS * 10 {
            a_entity.tick.now = Instant::from_millis(ms);
            b_entity.tick.now = Instant::from_millis(ms);
            a.update(&mut a_entity);
            b.update(&mut b_entity);
            if a_entity.face.mouth.center.y != b_entity.face.mouth.center.y {
                diverged = true;
                break;
            }
        }
        assert!(
            diverged,
            "distinct seeds produced identical y sequences over a 60s horizon"
        );
    }

    #[test]
    fn rand_offset_returns_zero_for_nonpositive_max() {
        // The `max <= 0` early-return short-circuits the divide-by-zero
        // and the asymmetric `[-max, +max]` range that would otherwise
        // collapse. Pin it directly so a future "support tighter
        // ranges" tweak can't accidentally drop the guard and start
        // returning garbage offsets at boot.
        let mut m = IdleMicroExpression::new();
        assert_eq!(m.rand_offset(0), 0);
        assert_eq!(m.rand_offset(-5), 0);
        assert_eq!(m.rand_offset(i32::MIN), 0);
    }

    #[test]
    fn next_delay_collapses_to_min_when_range_is_degenerate() {
        // If a custom configuration accidentally sets max <= min the
        // delay degenerates to a single value rather than wrapping
        // negatively or panicking on the `% (span + 1)` step.
        let mut m = IdleMicroExpression::new();
        m.interval_min_ms = 1_000;
        m.interval_max_ms = 1_000;
        assert_eq!(m.next_delay_ms(), 1_000);
        m.interval_max_ms = 500; // pathological: max < min
        assert_eq!(m.next_delay_ms(), 1_000);
    }

    #[test]
    fn default_matches_new() {
        let from_default = <IdleMicroExpression as Default>::default();
        let from_new = IdleMicroExpression::new();
        assert_eq!(from_default.interval_min_ms, from_new.interval_min_ms);
        assert_eq!(from_default.interval_max_ms, from_new.interval_max_ms);
        assert_eq!(from_default.max_mouth_y_px, from_new.max_mouth_y_px);
        assert_eq!(from_default.rng_state, from_new.rng_state);
    }

    #[test]
    fn meta_declares_expression_phase_and_mouth_writes() {
        let m = IdleMicroExpression::new();
        let meta = m.meta();
        assert_eq!(meta.name, "IdleMicroExpression");
        assert_eq!(meta.phase, Phase::Expression);
        assert_eq!(meta.priority, 1);
        assert_eq!(meta.reads, &[Field::MouthCenter]);
        assert_eq!(meta.writes, &[Field::MouthCenter]);
    }
}
