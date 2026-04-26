//! `MicrosaccadeFromAttention`: small involuntary eye darts during
//! sustained gaze. Disney's IROS 2020 robot-gaze paper identifies
//! these as the single biggest realism contributor for tracking
//! behaviour — without them, even a perfectly-locked gaze reads as
//! "dead stare."
//!
//! ## Mechanism
//!
//! While `mind.attention` is `Tracking{..}`, schedule a tiny eye
//! displacement every [`MICROSACCADE_INTERVAL_MIN_MS`] to
//! [`MICROSACCADE_INTERVAL_MAX_MS`] ms. Each displacement is up to
//! [`MICROSACCADE_AMPLITUDE_PX`] pixels in either axis, persists for
//! [`MICROSACCADE_DURATION_MS`], then snaps back to zero before the
//! next interval starts.
//!
//! Composes additively with [`super::GazeFromAttention`] (the gross
//! tracking offset) via diff-and-undo on `face.{left,right}_eye.center`.
//!
//! Resets to zero on transition out of `Tracking`.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::mind::Attention;
use crate::modifier::Modifier;
use core::num::NonZeroU32;

/// Minimum interval between microsaccades, in ms.
///
/// Real human microsaccades occur at 0.5–1.5 s during fixation
/// (Tobii / NCBI references). Lower bound is the floor.
pub const MICROSACCADE_INTERVAL_MIN_MS: u64 = 500;

/// Maximum interval between microsaccades, in ms.
pub const MICROSACCADE_INTERVAL_MAX_MS: u64 = 1_500;

/// How long a single microsaccade displacement is held, in ms.
///
/// Real saccades transit in ~30 ms; at 30 FPS this is one frame.
/// Holding the displacement for one extra frame (~66 ms total) makes
/// it visible without being jittery.
pub const MICROSACCADE_DURATION_MS: u64 = 66;

/// Maximum per-axis displacement, in pixels.
///
/// `2 px` matches the Disney IROS guidance of 0.5–2° amplitude on a
/// QVGA face (where the iris radius is ≈ 30 px and the 0.5° FOV
/// projection lands around 1–2 px).
pub const MICROSACCADE_AMPLITUDE_PX: i32 = 2;

/// Default xorshift32 seed used by [`MicrosaccadeFromAttention::new`].
/// Firmware overrides via [`Self::with_seed`] from the ESP32-S3 RNG
/// so two units don't synchronise.
const DEFAULT_SEED: NonZeroU32 = match NonZeroU32::new(0xA17E_5ACC) {
    Some(s) => s,
    None => unreachable!(),
};

/// Modifier that adds microsaccade jitter to both eye centres while
/// `mind.attention` is `Tracking`.
#[derive(Debug, Clone, Copy)]
pub struct MicrosaccadeFromAttention {
    /// xorshift32 state for the per-saccade direction + interval rolls.
    rng_state: u32,
    /// Currently-applied per-axis offset in pixels. Subtracted on the
    /// next tick before the new offset is applied (diff-and-undo with
    /// other Expression modifiers that touch eye centres).
    last_offset: (i32, i32),
    /// Instant the currently-active microsaccade ends (offset returns
    /// to zero). `None` between events.
    saccade_until: Option<Instant>,
    /// Instant the next microsaccade fires. `None` until the first
    /// `Tracking` tick anchors the schedule.
    next_saccade_at: Option<Instant>,
}

impl MicrosaccadeFromAttention {
    /// Construct with the default seed.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_seed(DEFAULT_SEED)
    }

    /// Construct with a custom xorshift32 seed. Pass a fresh seed
    /// per device so multi-unit deployments don't tick in unison.
    #[must_use]
    pub const fn with_seed(seed: NonZeroU32) -> Self {
        Self {
            rng_state: seed.get(),
            last_offset: (0, 0),
            saccade_until: None,
            next_saccade_at: None,
        }
    }

    /// Advance the xorshift32 state and return the next pseudo-random
    /// `u32`. Same algorithm as `IdleDrift::next_u32`.
    const fn next_u32(&mut self) -> u32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        x
    }

    /// Pick a uniform `i32` in `[-max, max]` from the next PRNG draw.
    /// Caller must pass `max >= 0`; negative values are clamped to 0.
    fn rand_offset(&mut self, max: i32) -> i32 {
        let max_abs = u32::try_from(max.max(0)).unwrap_or(0);
        let span = max_abs.saturating_mul(2).saturating_add(1);
        let draw = self.next_u32() % span.max(1);
        let signed = i32::try_from(draw).unwrap_or(i32::MAX);
        signed - i32::try_from(max_abs).unwrap_or(i32::MAX)
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
}

impl Default for MicrosaccadeFromAttention {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for MicrosaccadeFromAttention {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "MicrosaccadeFromAttention",
            description: "Adds 0.5–1.5 s involuntary eye darts (≤±2 px) to both eye centres \
                          while mind.attention is Tracking. Disney IROS 2020 calls this the \
                          single biggest realism contributor for tracking behaviour. Composes \
                          via diff-and-undo with GazeFromAttention.",
            phase: Phase::Expression,
            // After GazeFromAttention (priority 5) so the gross
            // tracking offset is in place before we add jitter.
            priority: 6,
            reads: &[
                Field::Attention,
                Field::LeftEyeCenter,
                Field::RightEyeCenter,
            ],
            writes: &[Field::LeftEyeCenter, Field::RightEyeCenter],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let tracking = matches!(entity.mind.attention, Attention::Tracking { .. });

        // Compute the desired offset for this tick.
        let mut target_offset: (i32, i32) = (0, 0);
        if tracking {
            // Anchor schedule on entry.
            if self.next_saccade_at.is_none() {
                let dwell =
                    self.rand_interval(MICROSACCADE_INTERVAL_MIN_MS, MICROSACCADE_INTERVAL_MAX_MS);
                self.next_saccade_at = Some(now + dwell);
            }

            // End an in-flight microsaccade if its duration elapsed.
            if let Some(until) = self.saccade_until
                && now >= until
            {
                self.saccade_until = None;
                let dwell =
                    self.rand_interval(MICROSACCADE_INTERVAL_MIN_MS, MICROSACCADE_INTERVAL_MAX_MS);
                self.next_saccade_at = Some(now + dwell);
            }

            // Fire a new microsaccade if scheduled and not currently
            // mid-saccade.
            if self.saccade_until.is_none()
                && let Some(at) = self.next_saccade_at
                && now >= at
            {
                let dx = self.rand_offset(MICROSACCADE_AMPLITUDE_PX);
                let dy = self.rand_offset(MICROSACCADE_AMPLITUDE_PX);
                target_offset = (dx, dy);
                self.saccade_until = Some(now + MICROSACCADE_DURATION_MS);
            } else if self.saccade_until.is_some() {
                // Hold the existing offset; we keep it in
                // `last_offset` and propagate by re-applying.
                target_offset = self.last_offset;
            }
        } else {
            // Reset on transition out of Tracking so a future Tracking
            // run starts with an empty schedule.
            self.next_saccade_at = None;
            self.saccade_until = None;
        }

        // Diff-and-undo: subtract previous, add current.
        let (prev_x, prev_y) = self.last_offset;
        let (curr_x, curr_y) = target_offset;
        let delta_x = curr_x - prev_x;
        let delta_y = curr_y - prev_y;
        if delta_x != 0 || delta_y != 0 {
            entity.face.left_eye.center.x += delta_x;
            entity.face.left_eye.center.y += delta_y;
            entity.face.right_eye.center.x += delta_x;
            entity.face.right_eye.center.y += delta_y;
        }
        self.last_offset = target_offset;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pose;

    fn tracking() -> Attention {
        Attention::Tracking {
            target: Pose::new(0.0, 0.0),
            since: Instant::from_millis(0),
        }
    }

    #[test]
    fn no_attention_leaves_eyes_alone() {
        let mut m = MicrosaccadeFromAttention::new();
        let mut entity = Entity::default();
        let baseline = entity.face.left_eye.center;
        for ms in (0..3000).step_by(33) {
            entity.tick.now = Instant::from_millis(ms);
            m.update(&mut entity);
        }
        assert_eq!(entity.face.left_eye.center, baseline);
    }

    #[test]
    fn tracking_eventually_jitters_eyes() {
        // Drive long enough that a microsaccade has to fire (max
        // interval is 1500 ms, microsaccade lasts 66 ms — within
        // 5 s we should see the offset depart from zero at least
        // once).
        let mut m = MicrosaccadeFromAttention::new();
        let mut entity = Entity::default();
        entity.mind.attention = tracking();
        let baseline_x = entity.face.left_eye.center.x;
        let mut saw_jitter = false;
        for ms in (0..5000).step_by(33) {
            entity.tick.now = Instant::from_millis(ms);
            m.update(&mut entity);
            if entity.face.left_eye.center.x != baseline_x {
                saw_jitter = true;
                break;
            }
        }
        assert!(saw_jitter, "no microsaccade fired in 5s of Tracking");
    }

    #[test]
    fn transition_out_of_tracking_clears_offset() {
        let mut m = MicrosaccadeFromAttention::new();
        let mut entity = Entity::default();
        entity.mind.attention = tracking();
        let baseline_x = entity.face.left_eye.center.x;
        // Drive until we observe a jitter.
        for ms in (0..5000).step_by(33) {
            entity.tick.now = Instant::from_millis(ms);
            m.update(&mut entity);
            if entity.face.left_eye.center.x != baseline_x {
                break;
            }
        }
        // Now drop attention. After the next tick, last_offset must
        // get subtracted out → eyes back to baseline.
        entity.mind.attention = Attention::None;
        entity.tick.now = Instant::from_millis(5_100);
        m.update(&mut entity);
        assert_eq!(entity.face.left_eye.center.x, baseline_x);
    }
}
