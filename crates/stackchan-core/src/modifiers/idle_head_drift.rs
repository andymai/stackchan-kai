//! Idle head drift: occasional brief head glances at randomized
//! intervals, mostly still in between. Mirrors [`super::IdleDrift`]'s
//! shape for eyes — discrete events, xorshift-randomized timing,
//! ease in / hold / ease out — translated to head pose.
//!
//! ## Why event-driven, not continuous
//!
//! A continuous triangle-wave sway (the previous behavior) reads as
//! "robot trying to look alive" — the constant motion is more
//! distracting than lifelike, especially when the user is sitting at
//! the desk looking at the avatar. Real people at rest sit mostly
//! still and occasionally glance at things; that's the pattern this
//! modifier produces. Combined with [`super::IdleDrift`] on the eyes
//! and the [`crate::mind::Dormancy`] gate, the avatar reads as
//! present-but-unobtrusive when idle.
//!
//! ## Event shape
//!
//! Each glance has three phases:
//!
//! - **Ease-in** ([`GLANCE_EASE_IN_MS`]): linear ramp from 0 to a
//!   randomly-chosen `(pan, tilt)` target within
//!   `±GLANCE_PAN_MAX_DEG` / `±GLANCE_TILT_MAX_DEG`.
//! - **Hold** ([`GLANCE_HOLD_MS`]): keep the offset.
//! - **Ease-out** ([`GLANCE_EASE_OUT_MS`]): linear ramp back to 0.
//!
//! Between glances the modifier holds at zero contribution — the head
//! sits still. The next-glance instant is rolled inside
//! `[GLANCE_INTERVAL_MIN_MS, GLANCE_INTERVAL_MAX_MS]` from the end of
//! the prior glance (or boot).
//!
//! ## Composition
//!
//! Same diff-and-undo pattern as the rest of the Motion-phase stack:
//! subtract the previous tick's *applied* (post-clamp) contribution
//! before adding the new one, so upstream pose writes survive and
//! asymmetric clamping doesn't accumulate into a permanent offset.
//! Gates on [`crate::mind::Dormancy`] — when `Asleep`, contribution
//! is zero (and any in-flight glance is cancelled) so the head goes
//! still without waiting for the current glance to finish.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::head::Pose;
use crate::modifier::Modifier;
use core::num::NonZeroU32;

/// Default xorshift32 seed used by [`IdleHeadDrift::new`]. Different
/// from [`super::IdleDrift`]'s default seed so the eye and head
/// idle motions don't synchronise out of the box.
#[allow(
    clippy::unwrap_used,
    reason = "const-evaluated against a non-zero literal: unwrap can't fire at runtime"
)]
const DEFAULT_SEED: NonZeroU32 = NonZeroU32::new(0xBEEF_CAFE).unwrap();

/// Minimum interval between glances, in milliseconds.
///
/// `5_000` reads as "occupied human" pacing — long enough that the
/// avatar reads as still rather than busy.
pub const GLANCE_INTERVAL_MIN_MS: u64 = 5_000;
/// Maximum interval between glances, in milliseconds.
pub const GLANCE_INTERVAL_MAX_MS: u64 = 15_000;
/// Linear ease-in window, in ms. ~250 ms approximates a real head's
/// reach time on an ambient glance without feeling snappy.
pub const GLANCE_EASE_IN_MS: u64 = 250;
/// Hold duration at the random target, in ms. ~600 ms gives the
/// audience time to register the look without feeling deliberate.
pub const GLANCE_HOLD_MS: u64 = 600;
/// Linear ease-out window, in ms.
///
/// For an ambient (non-engaging) glance, the brain disengages once
/// the look has done its job, so the return is *faster* than the
/// reach — the opposite asymmetry from an emotionally-loaded look.
/// `350` keeps that snappy "OK, back to what I was doing" feel
/// without crossing into a snap-back twitch.
pub const GLANCE_EASE_OUT_MS: u64 = 350;
/// Maximum pan offset for a single glance, in degrees. Small enough
/// to read as "looking at something nearby," not a deliberate
/// head-turn-to-track.
pub const GLANCE_PAN_MAX_DEG: f32 = 6.0;
/// Maximum tilt offset for a single glance, in degrees.
pub const GLANCE_TILT_MAX_DEG: f32 = 3.0;

/// Modifier that contributes occasional brief head glances to
/// `entity.motor.head_pose`. Mirrors [`super::IdleDrift`]'s pattern
/// for eyes.
#[derive(Debug, Clone, Copy)]
pub struct IdleHeadDrift {
    /// xorshift32 PRNG state. Same algorithm + interface as
    /// [`super::IdleDrift::next_u32`].
    rng_state: u32,
    /// Active glance, if any. `None` between events.
    active: Option<Glance>,
    /// Wall-clock instant at which the next glance starts. `None`
    /// until the first tick anchors the schedule.
    next_glance_at: Option<Instant>,
    /// Pan contribution **as actually applied** on the previous tick
    /// (post-clamp). Subtracted before writing the new contribution.
    last_pan_deg: f32,
    /// Tilt contribution as actually applied on the previous tick
    /// (post-clamp). See [`Self::last_pan_deg`].
    last_tilt_deg: f32,
}

/// Active-glance record. Captured on the start tick and replayed
/// across the ease-in / hold / ease-out windows.
#[derive(Debug, Clone, Copy)]
struct Glance {
    /// Random pan offset for this glance, in degrees.
    target_pan_deg: f32,
    /// Random tilt offset for this glance, in degrees.
    target_tilt_deg: f32,
    /// Wall-clock instant the glance started (ease-in begins here).
    started_at: Instant,
}

impl IdleHeadDrift {
    /// Construct with the default xorshift32 seed.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_seed(DEFAULT_SEED)
    }

    /// Construct with a custom xorshift32 seed. Pass a fresh seed
    /// per device (firmware seeds from the ESP32-S3 hardware RNG at
    /// boot) so multi-unit deployments don't tick in unison.
    #[must_use]
    pub const fn with_seed(seed: NonZeroU32) -> Self {
        Self {
            rng_state: seed.get(),
            active: None,
            next_glance_at: None,
            last_pan_deg: 0.0,
            last_tilt_deg: 0.0,
        }
    }

    /// Advance the xorshift32 state and return the next pseudo-random
    /// `u32`. Same algorithm as [`super::IdleDrift::next_u32`].
    const fn next_u32(&mut self) -> u32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        x
    }

    /// Pick a uniform `f32` in `[-max_abs, +max_abs]` from the next
    /// PRNG draw. Caller must pass `max_abs >= 0`; negative values
    /// are clamped to 0.
    fn rand_signed(&mut self, max_abs: f32) -> f32 {
        let max = max_abs.max(0.0);
        // Map u32 → [-1, +1] via the high 24 bits (f32 mantissa width).
        let raw = self.next_u32() >> 8; // 24 bits
        // 0..=2^24-1 → 0..=1
        #[allow(
            clippy::cast_precision_loss,
            reason = "raw is bounded by 2^24, well inside f32 mantissa precision"
        )]
        let unit = raw as f32 / ((1u32 << 24) - 1) as f32;
        // [0, 1] → [-1, +1]
        (unit * 2.0 - 1.0) * max
    }

    /// Pick a uniform `u64` in `[lo, hi]` from the next PRNG draw.
    fn rand_interval(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        let span = hi - lo + 1;
        let draw = u64::from(self.next_u32()) % span;
        lo + draw
    }

    /// Compute the in-flight contribution `(pan, tilt)` of an active
    /// glance at `now`. Returns `None` once the glance has fully
    /// completed (caller drops it). Pure function so the timing math
    /// is easy to unit-test in isolation.
    fn active_contribution(glance: Glance, now: Instant) -> Option<(f32, f32)> {
        let elapsed = now.saturating_duration_since(glance.started_at);
        let total = GLANCE_EASE_IN_MS + GLANCE_HOLD_MS + GLANCE_EASE_OUT_MS;
        if elapsed >= total {
            return None;
        }
        let scale = if elapsed < GLANCE_EASE_IN_MS {
            // Ease-in: 0 → 1 over the window.
            ramp(elapsed, GLANCE_EASE_IN_MS)
        } else if elapsed < GLANCE_EASE_IN_MS + GLANCE_HOLD_MS {
            1.0
        } else {
            // Ease-out: 1 → 0 over the window.
            let into_out = elapsed - GLANCE_EASE_IN_MS - GLANCE_HOLD_MS;
            1.0 - ramp(into_out, GLANCE_EASE_OUT_MS)
        };
        Some((
            glance.target_pan_deg * scale,
            glance.target_tilt_deg * scale,
        ))
    }
}

/// Linear `0..=1` ramp over `window_ms` from start. Saturates at 1
/// once `elapsed >= window_ms`. Returns `0.0` if `window_ms == 0`.
#[allow(
    clippy::cast_precision_loss,
    reason = "elapsed and window_ms both well under 2^24 in practice"
)]
fn ramp(elapsed: u64, window_ms: u64) -> f32 {
    if window_ms == 0 {
        return 0.0;
    }
    (elapsed as f32 / window_ms as f32).clamp(0.0, 1.0)
}

impl Default for IdleHeadDrift {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for IdleHeadDrift {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "IdleHeadDrift",
            description: "Occasional brief head glances at randomised 5-15 s intervals, \
                          mostly still in between. Mirrors IdleDrift's pattern for eyes \
                          (discrete events, xorshift-randomised timing, ease in / hold / \
                          ease out). Composes additively with upstream pose via diff-and-undo. \
                          Gates to no contribution while mind.dormancy == Asleep.",
            phase: Phase::Motion,
            priority: 0,
            reads: &[Field::HeadPose, Field::Dormancy],
            writes: &[Field::HeadPose],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;

        // Dormancy gate: Asleep cancels any in-flight glance and
        // holds at zero contribution. Diff-and-undo unwinds the
        // prior tick's offset so the head returns silently.
        if entity.mind.dormancy.is_asleep() {
            self.active = None;
            self.next_glance_at = None;
            self.apply(entity, 0.0, 0.0);
            return;
        }

        // Anchor the schedule on the first wakeful tick.
        if self.next_glance_at.is_none() && self.active.is_none() {
            let dwell = self.rand_interval(GLANCE_INTERVAL_MIN_MS, GLANCE_INTERVAL_MAX_MS);
            self.next_glance_at = Some(now + dwell);
        }

        // Start a new glance if scheduled and we're not mid-glance.
        if self.active.is_none()
            && let Some(at) = self.next_glance_at
            && now >= at
        {
            self.active = Some(Glance {
                target_pan_deg: self.rand_signed(GLANCE_PAN_MAX_DEG),
                target_tilt_deg: self.rand_signed(GLANCE_TILT_MAX_DEG),
                started_at: now,
            });
            self.next_glance_at = None;
        }

        let mut pan = 0.0;
        let mut tilt = 0.0;
        if let Some(glance) = self.active {
            if let Some((p, t)) = Self::active_contribution(glance, now) {
                pan = p;
                tilt = t;
            } else {
                self.active = None;
                let dwell = self.rand_interval(GLANCE_INTERVAL_MIN_MS, GLANCE_INTERVAL_MAX_MS);
                self.next_glance_at = Some(now + dwell);
            }
        }

        self.apply(entity, pan, tilt);
    }
}

impl IdleHeadDrift {
    /// Apply the requested `(pan, tilt)` contribution via diff-and-undo.
    /// Storing the *post-clamp* applied contribution keeps the unwind
    /// honest under `Pose::clamped` truncation.
    fn apply(&mut self, entity: &mut Entity, pan: f32, tilt: f32) {
        let upstream_pan = entity.motor.head_pose.pan_deg - self.last_pan_deg;
        let upstream_tilt = entity.motor.head_pose.tilt_deg - self.last_tilt_deg;
        let combined = Pose::new(upstream_pan + pan, upstream_tilt + tilt).clamped();
        self.last_pan_deg = combined.pan_deg - upstream_pan;
        self.last_tilt_deg = combined.tilt_deg - upstream_tilt;
        entity.motor.head_pose = combined;
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::panic,
    reason = "tests assert exact outputs of our own ramp + diff-and-undo math"
)]
#[allow(
    clippy::expect_used,
    reason = "test literals are compile-time non-zero; the expect can't fire"
)]
mod tests {
    use super::*;
    use crate::Entity;
    use crate::mind::Dormancy;

    fn at(now_ms: u64) -> Entity {
        let mut e = Entity::default();
        e.tick.now = Instant::from_millis(now_ms);
        e
    }

    /// Drive the modifier across `duration_ms` at 33 ms ticks,
    /// returning the sequence of `(now_ms, head_pose)` pairs.
    fn sample(m: &mut IdleHeadDrift, duration_ms: u64) -> Vec<(u64, Pose)> {
        let mut out = Vec::new();
        let mut entity = Entity::default();
        let mut t_ms = 0;
        while t_ms <= duration_ms {
            entity.tick.now = Instant::from_millis(t_ms);
            m.update(&mut entity);
            out.push((t_ms, entity.motor.head_pose));
            t_ms += 33;
        }
        out
    }

    #[test]
    fn quiet_until_first_glance_fires() {
        // First glance fires no earlier than GLANCE_INTERVAL_MIN_MS
        // after boot. Sample the first MIN-1 ms; head pose must
        // stay at neutral.
        let mut m = IdleHeadDrift::new();
        let trace = sample(&mut m, GLANCE_INTERVAL_MIN_MS - 100);
        for (t, pose) in &trace {
            assert_eq!(
                *pose,
                Pose::default(),
                "head should be still before first glance, got {pose:?} at {t}ms",
            );
        }
    }

    #[test]
    fn glance_contribution_stays_within_per_axis_max() {
        // Drive across multiple full glance cycles; every observed
        // pose must respect ±GLANCE_PAN_MAX_DEG and ±GLANCE_TILT_MAX_DEG
        // (with a small slack for the `Pose::clamped` floor on tilt).
        let mut m = IdleHeadDrift::new();
        let trace = sample(&mut m, 60_000); // 60 s, multiple events
        for (t, pose) in &trace {
            assert!(
                pose.pan_deg.abs() <= GLANCE_PAN_MAX_DEG + 0.01,
                "pan {} exceeds GLANCE_PAN_MAX_DEG at {t}ms",
                pose.pan_deg,
            );
            assert!(
                pose.tilt_deg <= GLANCE_TILT_MAX_DEG + 0.01,
                "tilt {} exceeds GLANCE_TILT_MAX_DEG at {t}ms",
                pose.tilt_deg,
            );
        }
    }

    #[test]
    fn at_least_one_glance_fires_within_max_interval_plus_total() {
        // Within MAX_INTERVAL + EASE_IN + HOLD + EASE_OUT we must have
        // observed at least one non-zero contribution.
        let total =
            GLANCE_INTERVAL_MAX_MS + GLANCE_EASE_IN_MS + GLANCE_HOLD_MS + GLANCE_EASE_OUT_MS;
        let mut m = IdleHeadDrift::new();
        let trace = sample(&mut m, total + 500);
        let saw_motion = trace.iter().any(|(_, p)| *p != Pose::default());
        assert!(
            saw_motion,
            "no glance fired within {total} ms (max interval + glance window)",
        );
    }

    #[test]
    fn head_returns_to_neutral_between_glances() {
        // After at least one glance completes, there must exist a
        // period of rest where the head pose is exactly neutral —
        // proves the diff-and-undo doesn't accumulate any permanent
        // offset across the ease-in / hold / ease-out cycle.
        //
        // We can't sample at a single end-of-trace tick (the next
        // glance might already have fired by then), so we look for
        // a transition in the trace: an active stretch (non-neutral)
        // followed by at least one neutral tick.
        let mut m = IdleHeadDrift::new();
        let trace = sample(
            &mut m,
            GLANCE_INTERVAL_MAX_MS + GLANCE_EASE_IN_MS + GLANCE_HOLD_MS + GLANCE_EASE_OUT_MS + 500,
        );
        let mut saw_active = false;
        let mut saw_rest_after_active = false;
        for (_, pose) in &trace {
            if *pose != Pose::default() {
                saw_active = true;
            } else if saw_active {
                saw_rest_after_active = true;
                break;
            }
        }
        assert!(saw_active, "no glance fired in the trace");
        assert!(
            saw_rest_after_active,
            "head never returned to neutral after a glance — diff-and-undo \
             may have left a residual offset",
        );
    }

    #[test]
    fn dormant_state_holds_zero_contribution() {
        // While Asleep, the modifier must not contribute regardless
        // of where in the schedule we'd otherwise be.
        let mut m = IdleHeadDrift::new();
        let mut entity = at(0);
        entity.mind.dormancy = Dormancy::Asleep {
            since: Instant::from_millis(0),
        };
        for t_ms in (0..60_000).step_by(33) {
            entity.tick.now = Instant::from_millis(t_ms);
            m.update(&mut entity);
            assert_eq!(
                entity.motor.head_pose,
                Pose::default(),
                "Asleep should hold zero contribution at {t_ms}ms",
            );
        }
    }

    #[test]
    fn distinct_seeds_produce_distinct_glance_sequences() {
        let mut a = IdleHeadDrift::with_seed(NonZeroU32::new(0x1234_5678).expect("non-zero"));
        let mut b = IdleHeadDrift::with_seed(NonZeroU32::new(0xCAFE_BABE).expect("non-zero"));
        let trace_a = sample(&mut a, 30_000);
        let trace_b = sample(&mut b, 30_000);
        let any_diff = trace_a
            .iter()
            .zip(trace_b.iter())
            .any(|((_, pa), (_, pb))| pa != pb);
        assert!(
            any_diff,
            "two distinct seeds produced identical glance sequences over 30 s",
        );
    }

    #[test]
    fn upstream_pose_survives_via_diff_and_undo() {
        // With a non-zero upstream pose contribution, the modifier's
        // diff-and-undo must preserve upstream when not glancing.
        let mut m = IdleHeadDrift::new();
        let mut entity = at(0);

        for t_ms in (0..GLANCE_INTERVAL_MIN_MS).step_by(33) {
            entity.tick.now = Instant::from_millis(t_ms);
            entity.motor.head_pose = Pose::new(2.0, 1.0); // upstream re-write
            m.update(&mut entity);
            // Pre-first-glance: contribution is 0, so head_pose stays at upstream.
            assert_eq!(entity.motor.head_pose, Pose::new(2.0, 1.0).clamped());
        }
    }
}
