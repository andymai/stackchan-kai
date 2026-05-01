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
//! Runs after [`super::IdleHeadDrift`] (priority 0) and
//! [`super::HeadFromEmotion`] (priority 10) within [`Phase::Motion`], so
//! its bias rides on top of the head-drift glances and emotion-keyed
//! bias.
//! Same diff-and-undo pattern as `HeadFromEmotion` / `IdleHeadDrift`: subtract
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
use crate::perception::{HALF_FOV_H_DEG, HALF_FOV_V_DEG};

/// Peak upward tilt added when fully attentive, in degrees.
///
/// Combined with `IdleHeadDrift`'s up-to-±3° tilt (a single glance
/// at peak amplitude) and `HeadFromEmotion`'s up-to-+3° (Happy) the
/// worst-case tilt stays comfortably inside
/// [`MAX_TILT_DEG`](crate::head::MAX_TILT_DEG).
pub const LISTEN_HEAD_TILT_DEG: f32 = 8.0;

/// Ease-in / ease-out window, in ms.
///
/// The bias ramps linearly from 0 → `LISTEN_HEAD_TILT_DEG` after
/// attention enters `Listening` and back to 0 after it leaves. 200 ms
/// reads as deliberate without feeling sluggish.
pub const LISTEN_HEAD_EASE_MS: u64 = 200;

/// Per-frame fraction of the way the head moves from its current
/// smoothed-target toward the live `Attention::Tracking{target}`.
///
/// Acts as a single-pole low-pass filter: each tick, the head's
/// effective target is `prev + α·(live − prev)`. At 30 FPS, `α =
/// 0.22` gives a ~4-frame (~132 ms) time constant — within the
/// 100–150 ms ILM/Disney eyes-lead-head convention. Naturally
/// smooths jittery tracker centroids (which the tighter v0.11
/// detection thresholds produce when only a few cells fire).
///
/// Encoded as a numerator + denominator pair so the const can stay
/// integer-only (no `f32` const-fn juggling) and so the math stays
/// exact in i64. `22 / 100` ≈ the desired α.
pub const HEAD_TRACKING_SMOOTHING_NUM: i64 = 22;
/// Denominator for [`HEAD_TRACKING_SMOOTHING_NUM`].
pub const HEAD_TRACKING_SMOOTHING_DEN: i64 = 100;

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
    /// — same diff-and-undo pattern as the rest of the Motion stack.
    last_tilt_deg: f32,
    /// Instant attention transitioned `None` → `Listening`. `None`
    /// when not currently in (or easing into) a listening run.
    listen_since: Option<Instant>,
    /// Instant attention transitioned `Listening` → `None`. `None`
    /// unless currently easing out.
    release_since: Option<Instant>,
    /// Smoothed tracking target (single-pole low-pass over the live
    /// `Attention::Tracking{target}`). `None` between Tracking runs;
    /// the next entry to Tracking re-anchors at the live target so
    /// there's no overshoot from a stale value.
    smoothed_target: Option<Pose>,
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
            smoothed_target: None,
        }
    }
}

impl Default for HeadFromAttention {
    fn default() -> Self {
        Self::new()
    }
}

/// Single-pole low-pass blend toward `target` from `prev` using the
/// `HEAD_TRACKING_SMOOTHING_NUM / DEN` α. Per-axis lerp.
fn lerp_pose(prev: Pose, target: Pose) -> Pose {
    Pose::new(
        lerp_axis(prev.pan_deg, target.pan_deg),
        lerp_axis(prev.tilt_deg, target.tilt_deg),
    )
}

/// `prev + α·(target − prev)` with α = `NUM / DEN`.
#[allow(
    clippy::cast_precision_loss,
    clippy::suboptimal_flops,
    reason = "smoothing constants are small integers (well inside f32 mantissa); \
              `mul_add` would need libm on no_std"
)]
fn lerp_axis(prev: f32, target: f32) -> f32 {
    let num = HEAD_TRACKING_SMOOTHING_NUM as f32;
    let den = HEAD_TRACKING_SMOOTHING_DEN as f32;
    prev + (target - prev) * (num / den)
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
    // practice; the cast is far inside f32 mantissa range.
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
                          IdleHeadDrift and HeadFromEmotion via diff-and-undo.",
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

        // Pick the contribution shape per attention variant. When the
        // engagement state carries a face centroid we use it (in pose-
        // degree units via the camera FOV) as the live target — the
        // head should track the face, not the noisier motion blob.
        let face_target_pose = entity
            .mind
            .engagement
            .centroid()
            .map(|(nx, ny)| Pose::new(nx * HALF_FOV_H_DEG, -ny * HALF_FOV_V_DEG).clamped());
        let (target_pan_contrib, target_tilt_contrib) =
            match (face_target_pose, entity.mind.attention) {
                (Some(face_target), _) => {
                    // Engagement-driven path: same low-pass as the motion
                    // path but anchored on the face centroid.
                    let prev = self.smoothed_target.unwrap_or(face_target);
                    let smoothed = lerp_pose(prev, face_target);
                    self.smoothed_target = Some(smoothed);
                    (
                        smoothed.pan_deg - upstream_pan,
                        smoothed.tilt_deg - upstream_tilt,
                    )
                }
                (None, Attention::Tracking { target, .. }) => {
                    // Eye-leads-head: the head's effective target is a
                    // single-pole low-pass over the live tracker target.
                    // Eyes already moved to the live target via
                    // `GazeFromAttention`; the head naturally lags + the
                    // filter smooths jittery per-frame centroids.
                    //
                    // On entry to Tracking the smoother anchors at the
                    // live target (no chase from a stale value).
                    let prev = self.smoothed_target.unwrap_or(target);
                    let smoothed = lerp_pose(prev, target);
                    self.smoothed_target = Some(smoothed);
                    (
                        smoothed.pan_deg - upstream_pan,
                        smoothed.tilt_deg - upstream_tilt,
                    )
                }
                (None, Attention::Listening { .. } | Attention::None) => {
                    // Drop the smoother anchor so a future Tracking run
                    // re-anchors at the live target (no stale chase).
                    self.smoothed_target = None;
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
    use crate::mind::{Attention, Engagement};

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
        // With a non-zero upstream pose (head-drift + emotion bias),
        // the tracking branch contributes (target - upstream) so the
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
    fn tracking_smooths_toward_new_target_across_frames() {
        // The smoother eases head pose from the previous entry-anchor
        // toward the live target each tick, so a target change does
        // NOT snap on the next frame — it bleeds over several frames.
        // Head reaches (within ~0.1°) of the new target after enough
        // ticks (~5 × time-constant ≈ 20 frames).
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();
        let t1 = Pose::new(10.0, 5.0);
        entity.mind.attention = tracking(t1);
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        // Entry-tick anchor: smoother latches on `target` so the head
        // arrives at `t1` immediately.
        assert_eq!(entity.motor.head_pose, t1);

        // Switch target. Next tick: head moves a fraction of the way.
        let t2 = Pose::new(-5.0, 10.0);
        entity.mind.attention = tracking(t2);
        entity.tick.now = Instant::from_millis(33);
        m.update(&mut entity);
        let after_one = entity.motor.head_pose;
        assert!(
            (after_one.pan_deg - t1.pan_deg).abs() > 1.0
                && (after_one.pan_deg - t2.pan_deg).abs() > 1.0,
            "head should move toward t2 but not reach it in one tick (got {after_one:?})",
        );

        // Drive many more ticks; should converge close to t2.
        for t in 2..40 {
            entity.tick.now = Instant::from_millis(t * 33);
            m.update(&mut entity);
        }
        let after_many = entity.motor.head_pose;
        assert!(
            (after_many.pan_deg - t2.pan_deg).abs() < 0.1
                && (after_many.tilt_deg - t2.tilt_deg).abs() < 0.1,
            "head should converge to ~t2 after ~40 ticks (got {after_many:?})",
        );
    }

    #[test]
    fn engaged_face_centroid_overrides_motion_target() {
        // Motion centroid says "pan +15°"; face centroid says "pan -15°".
        // Head's first-tick contribution must come from the face — the
        // smoother anchors at the face target, so the head lands there
        // exactly on tick 0.
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();
        entity.mind.attention = tracking(Pose::new(15.0, 0.0));
        entity.mind.engagement = Engagement::Locked {
            // -0.5 × HALF_FOV_H_DEG (31°) = -15.5°
            centroid: (-0.5_f32, 0.0_f32),
            at: Instant::from_millis(0),
        };
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        assert!(
            entity.motor.head_pose.pan_deg < 0.0,
            "head should pan LEFT toward the face, not right toward the motion target",
        );
    }

    #[test]
    fn listening_attention_with_engaged_face_turns_head_toward_face() {
        // The voice-attention path: a still speaker triggers `Listening`
        // while the firmware-side cascade (widened during listening) has
        // locked their face. The head must drive toward the face yaw —
        // not fall through to the tilt-only listening pose.
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();
        entity.mind.attention = listening(0);
        entity.mind.engagement = Engagement::Locked {
            // -0.5 × HALF_FOV_H_DEG (31°) → the face is left-of-centre.
            centroid: (-0.5_f32, 0.0_f32),
            at: Instant::from_millis(0),
        };
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);

        // Pan must come from the face centroid, not from the tilt-only
        // Listening fallback (which would leave pan_deg at 0).
        assert!(
            entity.motor.head_pose.pan_deg < -1.0,
            "Listening + locked face must yaw toward the face (got pan_deg = {})",
            entity.motor.head_pose.pan_deg,
        );
    }

    #[test]
    fn listening_attention_without_engagement_falls_back_to_tilt() {
        // Regression guard for the no-face-locked path: when listening
        // fires but the camera has no face, head goes through the
        // existing tilt-only ease (no yaw contribution).
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();
        entity.mind.attention = listening(0);
        // Engagement stays at default (Idle, no centroid).
        // First tick anchors `listen_since`; second tick (one full ease
        // window later) reaches peak bias.
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        entity.tick.now = Instant::from_millis(LISTEN_HEAD_EASE_MS);
        m.update(&mut entity);

        assert_eq!(entity.motor.head_pose.pan_deg, 0.0);
        assert_eq!(entity.motor.head_pose.tilt_deg, LISTEN_HEAD_TILT_DEG);
    }

    #[test]
    fn engaged_releasing_state_keeps_face_target() {
        // Releasing must keep driving the head toward the last-known
        // face centroid for the search beat — the lost-target
        // choreography (PR3) layers on top of this.
        let mut m = HeadFromAttention::new();
        let mut entity = Entity::default();
        entity.mind.attention = Attention::None;
        entity.mind.engagement = Engagement::Releasing {
            centroid: (0.4_f32, 0.0_f32),
            at: Instant::from_millis(0),
            misses: 5,
        };
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        // 0.4 × 31° ≈ +12.4°, smoother first-tick anchors at it.
        assert!(entity.motor.head_pose.pan_deg > 5.0);
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
