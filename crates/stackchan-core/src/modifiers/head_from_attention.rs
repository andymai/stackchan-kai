//! `HeadFromAttention`: motion modifier that biases head pose upward when
//! `mind.attention` is `Listening`, producing a cocked-head listening
//! posture.
//!
//! Driven by [`crate::skills::Listening`] (or any other source that
//! sets `mind.attention = Listening`). Stays at zero bias when
//! attention is `None`.
//!
//! ## Composition
//!
//! Runs after [`super::IdleSway`] (priority 0) and
//! [`super::HeadFromEmotion`] (priority 10) within [`Phase::Motion`], so
//! its bias rides on top of the baseline sway and emotion-keyed bias.
//! Same diff-and-undo pattern as `HeadFromEmotion` / `IdleSway`: subtract
//! the previous *applied* contribution before adding the new one,
//! storing the post-clamp delta so asymmetric clamping doesn't
//! accumulate into a permanent offset.
//!
//! ## Ease
//!
//! Linear ramp from 0 → [`LISTEN_HEAD_TILT_DEG`] over [`LISTEN_HEAD_EASE_MS`]
//! when entering Listening; symmetric ramp back to 0 over the same
//! window when leaving.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::head::Pose;
use crate::mind::Attention;
use crate::modifier::Modifier;

/// Peak upward tilt added when fully attentive, in degrees.
///
/// Combined with `IdleSway`'s ±2.5° tilt and `HeadFromEmotion`'s
/// up-to-+3° (Happy) the worst-case tilt stays comfortably inside
/// [`MAX_TILT_DEG`](crate::head::MAX_TILT_DEG).
pub const LISTEN_HEAD_TILT_DEG: f32 = 8.0;

/// Ease-in / ease-out window, in ms.
///
/// The bias ramps linearly from 0 → `LISTEN_HEAD_TILT_DEG` after
/// attention enters `Listening` and back to 0 after it leaves. 200 ms
/// reads as deliberate without feeling sluggish.
pub const LISTEN_HEAD_EASE_MS: u64 = 200;

/// Number of render frames the head's tracking-target lags behind the
/// instantaneous `Attention::Tracking{target}`. Eyes update without
/// delay (via `GazeFromAttention` in `Phase::Expression`); the head
/// reads a frame from this many ticks ago. At 30 FPS this is
/// ~132 ms — within the 100–150 ms ILM/Disney convention for
/// eyes-lead-head.
pub const HEAD_TRACKING_LAG_FRAMES: usize = 4;

/// Modifier that translates `mind.attention` variants into a
/// `motor.head_pose` contribution.
///
/// - `Attention::Listening` → eased upward tilt bias (cocked-head
///   listening posture).
/// - `Attention::Tracking { target }` → snap pose toward `target`
///   (the firmware tracker's already-slewed target). No ease — the
///   tracker handles smoothing via its own slew limit.
/// - `Attention::None` → no contribution (release ease for
///   Listening; instant for Tracking).
#[derive(Debug, Clone, Copy)]
pub struct HeadFromAttention {
    /// Pan contribution as actually applied on the previous tick
    /// (post-clamp). Subtracted before writing the new contribution.
    /// `0.0` for the Listening case (which only modifies tilt).
    last_pan_deg: f32,
    /// Tilt contribution as actually applied on the previous tick
    /// (post-clamp). Subtracted before writing the new contribution
    /// — see `IdleSway::last_pan_deg` for the same pattern.
    last_tilt_deg: f32,
    /// Instant attention transitioned `None` → `Listening`. `None`
    /// when not currently in (or easing into) a listening run.
    listen_since: Option<Instant>,
    /// Instant attention transitioned `Listening` → `None`. `None`
    /// unless currently easing out.
    release_since: Option<Instant>,
    /// Ring buffer of recent `Tracking{target}` poses, oldest at
    /// `lag_idx`. The head consumes the entry at `lag_idx` (oldest)
    /// before overwriting it with the current target — producing the
    /// eye-leads-head delay without a separate timing buffer.
    /// Cleared on transition out of `Tracking`.
    lag_buf: [Option<Pose>; HEAD_TRACKING_LAG_FRAMES],
    /// Next write index into [`Self::lag_buf`] (also the read index
    /// — entries form a circular buffer where the oldest pose is at
    /// the next-to-overwrite slot).
    lag_idx: u8,
}

impl HeadFromAttention {
    /// Construct an idle modifier with no in-flight ease state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_pan_deg: 0.0,
            last_tilt_deg: 0.0,
            listen_since: None,
            release_since: None,
            lag_buf: [None; HEAD_TRACKING_LAG_FRAMES],
            lag_idx: 0,
        }
    }
}

impl Default for HeadFromAttention {
    fn default() -> Self {
        Self::new()
    }
}

/// Linear `0..=1` ramp over `window_ms` from `start`. Saturates at
/// 1.0 once the elapsed exceeds the window. Returns 0.0 if
/// `window_ms == 0` (no ease — caller should snap).
fn ease(start: Instant, now: Instant, window_ms: u64) -> f32 {
    if window_ms == 0 {
        return 1.0;
    }
    let elapsed = now.saturating_duration_since(start);
    if elapsed >= window_ms {
        return 1.0;
    }
    // Both elapsed and window_ms are bounded by LISTEN_HEAD_EASE_MS in
    // practice; the cast is far inside f32 mantissa range. Match the
    // pattern in `IdleSway::unit_triangle`.
    #[allow(
        clippy::cast_precision_loss,
        reason = "elapsed and window_ms are both well under 2^24"
    )]
    let t = elapsed as f32 / window_ms as f32;
    t.clamp(0.0, 1.0)
}

impl Modifier for HeadFromAttention {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "HeadFromAttention",
            description: "When mind.attention is Listening, eases an upward tilt bias on \
                          motor.head_pose for a cocked-head listening posture. When attention \
                          is Tracking{target}, snaps pose toward the tracker's slewed target \
                          (no ease — the tracker handles smoothing). Composes additively after \
                          IdleSway and HeadFromEmotion via diff-and-undo.",
            phase: Phase::Motion,
            priority: 20,
            reads: &[Field::Attention, Field::HeadPose],
            writes: &[Field::HeadPose],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;

        // Edge detection for the Listening ease state machine. Only
        // observes Listening transitions — Tracking has no ease.
        let listening = matches!(entity.mind.attention, Attention::Listening { .. });
        match (listening, self.listen_since.is_some()) {
            (true, false) => {
                self.listen_since = Some(now);
                self.release_since = None;
            }
            (false, true) => {
                self.listen_since = None;
                self.release_since = Some(now);
            }
            _ => {}
        }

        // Recover upstream by subtracting our previous applied
        // contribution. Same diff-and-undo pattern as `HeadFromEmotion`.
        let upstream_pan = entity.motor.head_pose.pan_deg - self.last_pan_deg;
        let upstream_tilt = entity.motor.head_pose.tilt_deg - self.last_tilt_deg;

        // Pick the contribution shape per attention variant.
        let (target_pan_contrib, target_tilt_contrib) = match entity.mind.attention {
            Attention::Tracking { target, .. } => {
                // Eye-leads-head: the head consumes a target from
                // ~HEAD_TRACKING_LAG_FRAMES ticks ago; eyes already
                // moved to the current target via GazeFromAttention
                // earlier in Phase::Expression. Read the oldest entry
                // (the slot we're about to overwrite) before pushing
                // the current target.
                let idx = usize::from(self.lag_idx);
                let delayed = self.lag_buf[idx].unwrap_or(target);
                self.lag_buf[idx] = Some(target);
                self.lag_idx =
                    (self.lag_idx + 1) % u8::try_from(HEAD_TRACKING_LAG_FRAMES).unwrap_or(u8::MAX);
                (
                    delayed.pan_deg - upstream_pan,
                    delayed.tilt_deg - upstream_tilt,
                )
            }
            Attention::Listening { .. } | Attention::None => {
                // Clear the lag buffer so a future Tracking run
                // starts with an empty history (no stale targets).
                self.lag_buf = [None; HEAD_TRACKING_LAG_FRAMES];
                self.lag_idx = 0;
                // Tilt-only contribution, eased by the listen / release
                // anchors. Pan stays at zero (no contribution).
                let target_tilt = match (self.listen_since, self.release_since) {
                    (Some(since), _) => {
                        LISTEN_HEAD_TILT_DEG * ease(since, now, LISTEN_HEAD_EASE_MS)
                    }
                    (None, Some(rel_at)) => {
                        let t = ease(rel_at, now, LISTEN_HEAD_EASE_MS);
                        if t >= 1.0 {
                            self.release_since = None;
                            0.0
                        } else {
                            LISTEN_HEAD_TILT_DEG * (1.0 - t)
                        }
                    }
                    (None, None) => 0.0,
                };
                (0.0, target_tilt)
            }
        };

        let combined = Pose::new(
            upstream_pan + target_pan_contrib,
            upstream_tilt + target_tilt_contrib,
        )
        .clamped();
        self.last_pan_deg = combined.pan_deg - upstream_pan;
        self.last_tilt_deg = combined.tilt_deg - upstream_tilt;
        entity.motor.head_pose = combined;
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare bit-exact outputs of our own ease math, not accumulated FP arithmetic"
)]
mod tests {
    use super::*;
    use crate::Entity;
    use crate::mind::Attention;

    fn listening(since_ms: u64) -> Attention {
        Attention::Listening {
            since: Instant::from_millis(since_ms),
        }
    }

    #[test]
    fn no_attention_leaves_pose_alone() {
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();
        // Use an in-range tilt: MIN_TILT_DEG is 0 (asymmetric clamp).
        entity.motor.head_pose = Pose::new(2.0, 1.0);
        entity.tick.now = Instant::from_millis(500);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose, Pose::new(2.0, 1.0));
    }

    #[test]
    fn ease_in_starts_at_zero_and_reaches_full_after_window() {
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();

        // Tick 0: attention transitions to Listening. Anchor is set
        // this tick, elapsed = 0 → bias = 0.
        entity.mind.attention = listening(0);
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.tilt_deg, 0.0);

        // After full ease window: bias is at peak.
        entity.tick.now = Instant::from_millis(LISTEN_HEAD_EASE_MS);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.tilt_deg, LISTEN_HEAD_TILT_DEG);
    }

    #[test]
    fn ease_in_is_monotonically_non_decreasing_across_window() {
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();
        entity.mind.attention = listening(0);

        let mut last_tilt = f32::NEG_INFINITY;
        for ms in 0..=LISTEN_HEAD_EASE_MS {
            entity.tick.now = Instant::from_millis(ms);
            m.update(&mut entity);
            let tilt = entity.motor.head_pose.tilt_deg;
            assert!(
                tilt >= last_tilt - 0.001,
                "tilt regressed at {ms}ms: {tilt} < {last_tilt}",
            );
            last_tilt = tilt;
        }
    }

    #[test]
    fn ease_out_returns_to_zero_after_window() {
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();

        // Tick 1 anchors `listen_since`; bias starts at 0 on the
        // entry tick. Tick 2 (one full window later) reaches peak.
        entity.mind.attention = listening(0);
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        entity.tick.now = Instant::from_millis(LISTEN_HEAD_EASE_MS);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.tilt_deg, LISTEN_HEAD_TILT_DEG);

        // Attention drops back to None — anchors `release_since`.
        entity.mind.attention = Attention::None;
        entity.tick.now = Instant::from_millis(LISTEN_HEAD_EASE_MS + 1);
        m.update(&mut entity);
        // First release tick: still at near-full bias.
        assert!(entity.motor.head_pose.tilt_deg >= LISTEN_HEAD_TILT_DEG * 0.95);

        // Full ease-out window past release anchor → bias = 0.
        entity.tick.now = Instant::from_millis(LISTEN_HEAD_EASE_MS + 1 + LISTEN_HEAD_EASE_MS);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.tilt_deg, 0.0);
    }

    #[test]
    fn ease_out_is_monotonically_non_increasing_across_window() {
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();

        // Get to full attention.
        entity.mind.attention = listening(0);
        entity.tick.now = Instant::from_millis(LISTEN_HEAD_EASE_MS);
        m.update(&mut entity);

        entity.mind.attention = Attention::None;
        let release_start = LISTEN_HEAD_EASE_MS + 10;
        let mut last_tilt = f32::INFINITY;
        for offset in 0..=LISTEN_HEAD_EASE_MS {
            entity.tick.now = Instant::from_millis(release_start + offset);
            m.update(&mut entity);
            let tilt = entity.motor.head_pose.tilt_deg;
            assert!(
                tilt <= last_tilt + 0.001,
                "tilt grew during ease-out at +{offset}ms: {tilt} > {last_tilt}",
            );
            last_tilt = tilt;
        }
    }

    #[test]
    fn additive_composition_with_upstream_tilt() {
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();
        entity.mind.attention = listening(0);
        let upstream_tilt = -2.0;

        for i in 0..30 {
            entity.motor.head_pose = Pose::new(0.0, upstream_tilt);
            entity.tick.now = Instant::from_millis(i * 10);
            m.update(&mut entity);
            // Our contribution should be the difference from upstream.
            let attend_contribution = entity.motor.head_pose.tilt_deg - upstream_tilt;
            assert!(
                (0.0..=LISTEN_HEAD_TILT_DEG + 0.01).contains(&attend_contribution),
                "contribution {attend_contribution} out of range at tick {i}",
            );
        }
    }

    #[test]
    fn re_enter_during_ease_out_resumes_ease_in() {
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();

        // Anchor + reach full attention (entry tick + one window).
        entity.mind.attention = listening(0);
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        entity.tick.now = Instant::from_millis(LISTEN_HEAD_EASE_MS);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.tilt_deg, LISTEN_HEAD_TILT_DEG);

        // Begin release. The anchor tick produces full bias still
        // (elapsed = 0). Advance one more tick into the window to
        // observe the decrement.
        entity.mind.attention = Attention::None;
        entity.tick.now = Instant::from_millis(LISTEN_HEAD_EASE_MS + 1);
        m.update(&mut entity);
        entity.tick.now = Instant::from_millis(LISTEN_HEAD_EASE_MS + 50);
        m.update(&mut entity);
        let mid_release_tilt = entity.motor.head_pose.tilt_deg;
        assert!(mid_release_tilt < LISTEN_HEAD_TILT_DEG);

        // Re-enter Listening mid-release. This anchors a fresh
        // `listen_since`; bias starts at 0 on this tick.
        entity.mind.attention = listening(LISTEN_HEAD_EASE_MS + 100);
        entity.tick.now = Instant::from_millis(LISTEN_HEAD_EASE_MS + 100);
        m.update(&mut entity);
        // After a full ease window from re-entry, bias is at peak.
        entity.tick.now = Instant::from_millis(LISTEN_HEAD_EASE_MS + 100 + LISTEN_HEAD_EASE_MS);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.tilt_deg, LISTEN_HEAD_TILT_DEG);
    }

    fn tracking(target: Pose) -> Attention {
        Attention::Tracking {
            target,
            since: Instant::from_millis(0),
        }
    }

    #[test]
    fn tracking_snaps_pose_to_target() {
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();
        let target = Pose::new(15.0, 8.0);
        entity.mind.attention = tracking(target);
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);

        // No ease — pose snaps to target on the entry tick.
        assert_eq!(entity.motor.head_pose, target);
    }

    #[test]
    fn tracking_overrides_upstream_pan_and_tilt() {
        // With a non-zero upstream pose (sway + emotion bias), the
        // tracking branch contributes (target - upstream) so the
        // combined pose lands exactly on target.
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();
        let target = Pose::new(20.0, 12.0);
        entity.mind.attention = tracking(target);
        entity.motor.head_pose = Pose::new(-3.0, 4.0); // upstream
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);

        assert_eq!(entity.motor.head_pose, target);
    }

    #[test]
    fn tracking_target_updates_per_tick() {
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();
        let t1 = Pose::new(10.0, 5.0);
        entity.mind.attention = tracking(t1);
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose, t1);

        let t2 = Pose::new(-5.0, 10.0);
        entity.mind.attention = tracking(t2);
        // Simulate upstream wandering on the next tick.
        entity.motor.head_pose = Pose::new(t1.pan_deg + 1.0, t1.tilt_deg + 1.0);
        entity.tick.now = Instant::from_millis(33);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose, t2);
    }

    #[test]
    fn tracking_to_none_clears_pan_contribution() {
        // After dropping attention, the modifier's `last_pan_deg`
        // should converge back to 0 within a tick (no contribution
        // when attention is None). Verifies the diff-and-undo state
        // doesn't get stuck.
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();

        entity.mind.attention = tracking(Pose::new(15.0, 8.0));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);

        entity.mind.attention = Attention::None;
        entity.tick.now = Instant::from_millis(33);
        m.update(&mut entity);

        // Pan contribution should be zero (no attention, no
        // listening ease for pan).
        assert_eq!(m.last_pan_deg, 0.0);
    }
}
