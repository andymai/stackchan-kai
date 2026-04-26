//! `LostTargetSearch`: motion-phase modifier that animates a brief
//! "where did they go?" search beat after the engagement lock drops.
//!
//! Watches [`crate::mind::Engagement`] for the falling edge from
//! engaged ([`crate::mind::Engagement::Locked`] /
//! [`crate::mind::Engagement::Releasing`]) to disengaged
//! ([`crate::mind::Engagement::Idle`]). On that edge, captures the
//! last-known face centroid and choreographs three beats:
//!
//! - **Hold** ([`SEARCH_HOLD_MS`]): keep the head pointed at the
//!   last-seen pose. Reads as "they were just there."
//! - **Saccade** ([`SEARCH_SACCADE_MS`]): extend the head past the
//!   last-seen pose by [`SEARCH_SACCADE_OVERSHOOT`]× — a single
//!   directionally-cued look-around in the same direction the face
//!   exited.
//! - **Return** ([`SEARCH_RETURN_MS`]): linear ramp back to no
//!   contribution; whatever's left of `motor.head_pose` upstream
//!   takes over.
//!
//! ## Composition
//!
//! Runs after [`super::HeadFromAttention`] (priority 25 vs. 20) so the
//! search-beat contribution rides on top of any motion-tracking pose
//! that's still active. Same diff-and-undo pattern as the other Motion
//! modifiers: store the last applied delta, subtract before adding the
//! new one. Asymmetric clamping won't accumulate into a permanent
//! offset.
//!
//! ## Why a separate modifier (not part of `HeadFromAttention`)?
//!
//! `HeadFromAttention` reacts to the *current* engagement / attention
//! state. Lost-target search needs to outlive engagement: by the time
//! the head should be doing the search saccade, engagement has
//! already transitioned to `Idle`. A modifier with its own
//! lock-loss-edge state machine keeps the lifecycle explicit.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::head::Pose;
use crate::modifier::Modifier;
use crate::perception::{HALF_FOV_H_DEG, HALF_FOV_V_DEG};

/// Duration the search beat holds the head at the last-known face
/// pose before extending into the saccade, in ms.
///
/// `500` ms reads as "I just saw you a moment ago — let me check
/// where you went." Long enough that the audience registers the
/// hold; short enough that the search feels purposeful.
pub const SEARCH_HOLD_MS: u64 = 500;

/// Duration of the search saccade itself, in ms.
///
/// `300` ms is on the slow end of human saccades (a real saccade is
/// 50–100 ms) but Stack-chan's servos can't realistically slew that
/// fast — 300 ms reads as a deliberate "look further over there"
/// rather than a twitch.
pub const SEARCH_SACCADE_MS: u64 = 300;

/// Duration of the linear ramp back to no contribution, in ms.
///
/// `1000` ms is "calmly disengaging" — long enough that the head's
/// return doesn't read as a snap, short enough that the avatar is
/// available to lock onto a new target without a multi-second
/// sluggish recovery.
pub const SEARCH_RETURN_MS: u64 = 1_000;

/// Multiplier on the last-known centroid during the saccade beat.
///
/// `1.3×` extends the head ~30 % beyond the last-seen pose — clearly
/// "look further" rather than "look at where they were." Bigger
/// multipliers push the servos near their clamps; smaller ones
/// disappear into the natural FOV jitter.
pub const SEARCH_SACCADE_OVERSHOOT: f32 = 1.3;

/// Total search-beat duration. Convenience derived from the three
/// phase constants above.
pub const SEARCH_TOTAL_MS: u64 = SEARCH_HOLD_MS + SEARCH_SACCADE_MS + SEARCH_RETURN_MS;

/// Motion-phase modifier that animates a directional look-around
/// after engagement drops.
#[derive(Debug, Clone, Copy)]
pub struct LostTargetSearch {
    /// Most recently observed face centroid while engagement was
    /// engaged. `None` until at least one engaged tick has been seen.
    /// Used as the "last-known direction" when the lost-edge fires.
    last_seen_centroid: Option<(f32, f32)>,
    /// Whether engagement was engaged on the previous tick. Edge
    /// detector for [`Engagement::is_engaged`] going `true → false`.
    prev_engaged: bool,
    /// Active search beat. `None` outside of the post-lock-loss
    /// window.
    beat: Option<Beat>,
    /// Pan contribution applied on the previous tick (post-clamp).
    /// Subtracted before applying the new delta — diff-and-undo.
    last_pan_deg: f32,
    /// Tilt contribution applied on the previous tick (post-clamp).
    last_tilt_deg: f32,
}

/// Active search-beat record. Captured once on the lock-loss edge
/// and replayed across subsequent ticks until [`SEARCH_TOTAL_MS`]
/// elapses.
#[derive(Debug, Clone, Copy)]
struct Beat {
    /// Last-known face centroid in normalised frame coords.
    centroid: (f32, f32),
    /// Wall-clock instant the lock was lost (start of `Hold`).
    started_at: Instant,
}

impl LostTargetSearch {
    /// Construct an idle modifier with no in-flight search beat.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_seen_centroid: None,
            prev_engaged: false,
            beat: None,
            last_pan_deg: 0.0,
            last_tilt_deg: 0.0,
        }
    }
}

impl Default for LostTargetSearch {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a normalised centroid to a head-pose target via the
/// camera FOV. Same conversion used by `HeadFromAttention` /
/// `GazeFromAttention`.
fn centroid_to_pose(centroid: (f32, f32)) -> Pose {
    Pose::new(centroid.0 * HALF_FOV_H_DEG, -centroid.1 * HALF_FOV_V_DEG).clamped()
}

/// Compute the search-beat head-pose contribution for the elapsed
/// time since lock loss. Pure function so the timing logic is
/// straightforward to unit-test.
///
/// Returns `(pan_deg, tilt_deg)` — the *additive* contribution this
/// modifier adds on top of upstream. Returns `(0, 0)` outside the
/// search window (caller drops the beat). Each output is run through
/// `Pose::clamped` so an extreme centroid scaled by
/// `SEARCH_SACCADE_OVERSHOOT` can't push the contribution past the
/// servos' safe range — the `update` re-clamps after combining with
/// upstream, but clamping here keeps the diff-and-undo bookkeeping
/// honest about how much of our requested contribution actually
/// landed.
fn search_contribution(centroid: (f32, f32), elapsed_ms: u64) -> (f32, f32) {
    let target = centroid_to_pose(centroid);

    let scaled = |scale: f32| -> Pose {
        Pose::new(target.pan_deg * scale, target.tilt_deg * scale).clamped()
    };

    // Hold: full target contribution, no extension yet.
    if elapsed_ms < SEARCH_HOLD_MS {
        let p = scaled(1.0);
        return (p.pan_deg, p.tilt_deg);
    }
    // Saccade: linear ramp from 1.0× target → SEARCH_SACCADE_OVERSHOOT× target.
    let saccade_end = SEARCH_HOLD_MS + SEARCH_SACCADE_MS;
    if elapsed_ms < saccade_end {
        let into_saccade = elapsed_ms - SEARCH_HOLD_MS;
        #[allow(
            clippy::cast_precision_loss,
            reason = "elapsed_ms and SEARCH_SACCADE_MS are both well under 2^24"
        )]
        let t = into_saccade as f32 / SEARCH_SACCADE_MS as f32;
        // `f32::mul_add` isn't available on `no_std` Xtensa without
        // pulling in libm; the simple form is clearer and the
        // precision difference is negligible at the resolution
        // `Pose::clamped` retains. Same trade made by `lerp_axis` in
        // `head_from_attention`.
        #[allow(
            clippy::suboptimal_flops,
            reason = "no_std Xtensa lacks f32::mul_add; libm::fmaf is heavier than the savings"
        )]
        let scale = 1.0 + (SEARCH_SACCADE_OVERSHOOT - 1.0) * t;
        let p = scaled(scale);
        return (p.pan_deg, p.tilt_deg);
    }
    // Return: linear ramp from SEARCH_SACCADE_OVERSHOOT× target → 0.
    let return_end = saccade_end + SEARCH_RETURN_MS;
    if elapsed_ms < return_end {
        let into_return = elapsed_ms - saccade_end;
        #[allow(
            clippy::cast_precision_loss,
            reason = "into_return and SEARCH_RETURN_MS both ≤ 1000, far inside f32 mantissa"
        )]
        let t = into_return as f32 / SEARCH_RETURN_MS as f32;
        let scale = SEARCH_SACCADE_OVERSHOOT * (1.0 - t);
        let p = scaled(scale);
        return (p.pan_deg, p.tilt_deg);
    }
    // Past the search window: no contribution.
    (0.0, 0.0)
}

impl Modifier for LostTargetSearch {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "LostTargetSearch",
            description: "Watches mind.engagement for the engaged → disengaged edge. \
                          On lock loss captures the last-known face centroid and animates \
                          a hold → directional saccade → linear-return choreography on \
                          motor.head_pose for SEARCH_TOTAL_MS. Composes additively after \
                          HeadFromAttention via diff-and-undo.",
            phase: Phase::Motion,
            priority: 25,
            reads: &[Field::Engagement, Field::HeadPose],
            writes: &[Field::HeadPose],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;

        // Cache the most recent centroid every engaged tick so we
        // have something to point at when the lost edge fires.
        if let Some(centroid) = entity.mind.engagement.centroid() {
            self.last_seen_centroid = Some(centroid);
        }

        // Edge detection.
        //
        // Falling edge (engaged → disengaged): start a search beat
        // anchored on the most recently cached centroid.
        //
        // Rising edge (disengaged → engaged): a fresh face was
        // acquired. Drop any in-flight beat so we don't keep
        // animating an additive saccade overlaid on the new lock.
        // The diff-and-undo math below will then cleanly unwind the
        // previous tick's contribution.
        let engaged_now = entity.mind.engagement.is_engaged();
        if self.prev_engaged
            && !engaged_now
            && let Some(centroid) = self.last_seen_centroid
        {
            self.beat = Some(Beat {
                centroid,
                started_at: now,
            });
        }
        if engaged_now && !self.prev_engaged {
            self.beat = None;
        }
        self.prev_engaged = engaged_now;

        // Reset the cached centroid when fully disengaged so a fresh
        // engagement run captures cleanly. Doing it here (rather than
        // on every Idle tick) keeps the centroid live during
        // Releasing for the cache → beat handoff.
        if !engaged_now && self.beat.is_none() {
            self.last_seen_centroid = None;
        }

        // Recover upstream by subtracting our previous contribution.
        let upstream_pan = entity.motor.head_pose.pan_deg - self.last_pan_deg;
        let upstream_tilt = entity.motor.head_pose.tilt_deg - self.last_tilt_deg;

        // Compute contribution from the active beat, if any.
        let (contrib_pan, contrib_tilt) = match self.beat {
            Some(beat) => {
                let elapsed = now.saturating_duration_since(beat.started_at);
                if elapsed >= SEARCH_TOTAL_MS {
                    self.beat = None;
                    (0.0, 0.0)
                } else {
                    search_contribution(beat.centroid, elapsed)
                }
            }
            None => (0.0, 0.0),
        };

        let combined =
            Pose::new(upstream_pan + contrib_pan, upstream_tilt + contrib_tilt).clamped();
        self.last_pan_deg = combined.pan_deg - upstream_pan;
        self.last_tilt_deg = combined.tilt_deg - upstream_tilt;
        entity.motor.head_pose = combined;
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::panic,
    reason = "tests assert exact outputs of our own ramp math, not accumulated FP arithmetic"
)]
mod tests {
    use super::*;
    use crate::Entity;
    use crate::Pose;
    use crate::mind::Engagement;

    fn locked(centroid: (f32, f32)) -> Engagement {
        Engagement::Locked {
            centroid,
            at: Instant::from_millis(0),
        }
    }

    #[test]
    fn idle_engagement_makes_no_contribution() {
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();
        entity.motor.head_pose = Pose::new(3.0, 1.0);
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose, Pose::new(3.0, 1.0));
    }

    #[test]
    fn locked_alone_makes_no_contribution() {
        // Modifier should NOT add anything while engagement is still
        // engaged — the active head modifiers handle that path.
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();
        entity.mind.engagement = locked((0.5, 0.0));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose, Pose::default());
    }

    #[test]
    fn lock_loss_starts_hold_at_last_centroid() {
        // Tick 1: engaged. Tick 2: idle (lock loss edge). Modifier
        // should now contribute a head pose pointing at the last
        // centroid for the hold beat.
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();

        entity.mind.engagement = locked((0.5, 0.0));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);

        entity.mind.engagement = Engagement::Idle;
        entity.tick.now = Instant::from_millis(33);
        m.update(&mut entity);

        // 0.5 × 31° = 15.5° pan, no tilt.
        assert!((entity.motor.head_pose.pan_deg - 15.5).abs() < 0.01);
        assert_eq!(entity.motor.head_pose.tilt_deg, 0.0);
    }

    #[test]
    fn search_extends_past_last_centroid_during_saccade() {
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();

        // Set up + lose lock at t=0.
        entity.mind.engagement = locked((0.5, 0.0));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        entity.mind.engagement = Engagement::Idle;
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);

        // Sample at end of saccade (= 1.3× target).
        entity.tick.now = Instant::from_millis(SEARCH_HOLD_MS + SEARCH_SACCADE_MS - 1);
        m.update(&mut entity);
        // Just shy of full overshoot: ~1.3 × 15.5 ≈ 20.15°. Use 18 as
        // a conservative lower bound that excludes the simple hold.
        assert!(
            entity.motor.head_pose.pan_deg > 18.0,
            "expected saccade extension past 18°, got {}",
            entity.motor.head_pose.pan_deg,
        );
    }

    #[test]
    fn search_returns_to_no_contribution_after_total_window() {
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();

        entity.mind.engagement = locked((0.5, 0.0));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        entity.mind.engagement = Engagement::Idle;
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);

        // Past the total window, contribution should be 0 again.
        entity.tick.now = Instant::from_millis(SEARCH_TOTAL_MS + 100);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose, Pose::default());
        // And `beat` should be cleared so a follow-up lock-loss
        // restarts cleanly.
        assert!(m.beat.is_none());
    }

    #[test]
    fn return_phase_blends_toward_zero() {
        // At the end of the saccade beat the contribution is at peak
        // (~1.3× target). Ten percent into the return ramp it should
        // be smaller than that peak. Halfway through it should be
        // about half of the peak.
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();

        entity.mind.engagement = locked((0.5, 0.0));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        entity.mind.engagement = Engagement::Idle;
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);

        // Peak at end-of-saccade.
        entity.tick.now = Instant::from_millis(SEARCH_HOLD_MS + SEARCH_SACCADE_MS);
        m.update(&mut entity);
        let peak = entity.motor.head_pose.pan_deg;

        // Halfway through return.
        entity.tick.now =
            Instant::from_millis(SEARCH_HOLD_MS + SEARCH_SACCADE_MS + SEARCH_RETURN_MS / 2);
        m.update(&mut entity);
        let mid = entity.motor.head_pose.pan_deg;

        // Linear ramp: at exactly half the return window the
        // contribution is exactly half of peak (within f32 rounding).
        let half_peak = peak * 0.5;
        assert!(
            (mid - half_peak).abs() < 0.05,
            "halfway return should be exactly half of peak (peak={peak}, mid={mid})",
        );
    }

    #[test]
    fn contribution_is_diff_and_undone_after_beat() {
        // Drive a full search beat with a non-zero upstream pose and
        // verify the modifier's last contribution is fully undone
        // when the beat ends — no permanent offset.
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();
        entity.motor.head_pose = Pose::new(2.0, 1.0); // upstream

        entity.mind.engagement = locked((0.5, 0.0));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        entity.mind.engagement = Engagement::Idle;
        for ms in [33_u64, 200, 600, 900, 1_500, 1_900] {
            entity.tick.now = Instant::from_millis(ms);
            m.update(&mut entity);
        }
        // Past beat: motor.head_pose should equal the upstream value.
        assert_eq!(entity.motor.head_pose, Pose::new(2.0, 1.0));
    }

    #[test]
    fn locked_to_idle_skipping_releasing_still_starts_beat() {
        // Direct Locked → Idle transition (no Releasing in between).
        // Because `is_engaged()` covers both Locked and Releasing
        // identically, the falling edge fires regardless of which
        // engaged variant preceded Idle.
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();
        entity.mind.engagement = locked((0.4, 0.0));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        entity.mind.engagement = Engagement::Idle;
        entity.tick.now = Instant::from_millis(33);
        m.update(&mut entity);
        assert!(
            entity.motor.head_pose.pan_deg > 5.0,
            "Locked → Idle skip should still start the search beat",
        );
    }

    #[test]
    fn relock_during_beat_clears_contribution() {
        // Lose lock, beat starts. A fresh face gets re-acquired
        // mid-beat; the modifier must drop the active beat so its
        // additive contribution unwinds via diff-and-undo. Otherwise
        // the new lock would be perturbed by a stale saccade overlay.
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();
        entity.mind.engagement = locked((0.5, 0.0));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);

        // Drop lock → start beat.
        entity.mind.engagement = Engagement::Idle;
        entity.tick.now = Instant::from_millis(33);
        m.update(&mut entity);
        assert!(
            entity.motor.head_pose.pan_deg > 5.0,
            "beat should be active"
        );

        // Mid-beat re-lock. Modifier's contribution must be 0; the
        // upstream pose is empty so the resulting head_pose is
        // exactly the (zero) upstream.
        entity.mind.engagement = locked((-0.2, 0.0));
        entity.tick.now = Instant::from_millis(200);
        m.update(&mut entity);
        assert_eq!(
            entity.motor.head_pose,
            Pose::default(),
            "re-lock during beat must drop the search-beat contribution",
        );
        assert!(m.beat.is_none());
    }

    #[test]
    fn two_rapid_lock_losses_use_fresh_centroid() {
        // First loss → beat with centroid A. Brief re-lock with
        // centroid B. Second loss → beat with centroid B (fresh
        // cache), not A. Drives the cache-correctness path.
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();

        // Lock A on left, lose it.
        entity.mind.engagement = locked((-0.4, 0.0));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        entity.mind.engagement = Engagement::Idle;
        entity.tick.now = Instant::from_millis(33);
        m.update(&mut entity);
        assert!(
            entity.motor.head_pose.pan_deg < -5.0,
            "first beat looks left"
        );

        // Re-lock to B on right.
        entity.mind.engagement = locked((0.4, 0.0));
        entity.tick.now = Instant::from_millis(100);
        m.update(&mut entity);

        // Lose lock again → second beat must use the new (right) centroid.
        entity.mind.engagement = Engagement::Idle;
        entity.tick.now = Instant::from_millis(133);
        m.update(&mut entity);
        assert!(
            entity.motor.head_pose.pan_deg > 5.0,
            "second beat should follow the most recent centroid (right), got {}",
            entity.motor.head_pose.pan_deg,
        );
    }

    #[test]
    fn extreme_centroid_clamps_search_contribution() {
        // (0.99, 0.99) × 1.3 overshoot would otherwise push past
        // MAX_PAN_DEG / MIN_TILT_DEG. The clamp inside
        // `search_contribution` ensures the contribution stays in
        // the safe range, and the diff-and-undo state is honest
        // about how much actually landed.
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();
        entity.mind.engagement = locked((0.99, 0.99));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        entity.mind.engagement = Engagement::Idle;
        for ms in [33_u64, 600, 800, 1_000, 1_500, 1_800, 2_000] {
            entity.tick.now = Instant::from_millis(ms);
            m.update(&mut entity);
            assert!(entity.motor.head_pose.pan_deg <= crate::head::MAX_PAN_DEG);
            assert!(entity.motor.head_pose.pan_deg >= -crate::head::MAX_PAN_DEG);
            assert!(entity.motor.head_pose.tilt_deg <= crate::head::MAX_TILT_DEG);
            assert!(entity.motor.head_pose.tilt_deg >= crate::head::MIN_TILT_DEG);
        }
        // After the beat ends, head_pose is back to the zero upstream.
        entity.tick.now = Instant::from_millis(SEARCH_TOTAL_MS + 100);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose, Pose::default());
    }

    #[test]
    fn zero_centroid_makes_no_beat_contribution() {
        // Last-known centroid at frame center → search beat has
        // nowhere to look. Every phase produces a zero contribution.
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();
        entity.mind.engagement = locked((0.0, 0.0));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        entity.mind.engagement = Engagement::Idle;
        for ms in [33_u64, 200, 600, 800, 1_500] {
            entity.tick.now = Instant::from_millis(ms);
            m.update(&mut entity);
            assert_eq!(entity.motor.head_pose, Pose::default());
        }
    }

    #[test]
    fn search_direction_follows_centroid_sign() {
        // Negative (left) centroid → negative (left) pan during
        // hold + saccade.
        let mut m = LostTargetSearch::new();
        let mut entity = Entity::default();
        entity.mind.engagement = locked((-0.4, 0.0));
        entity.tick.now = Instant::from_millis(0);
        m.update(&mut entity);
        entity.mind.engagement = Engagement::Idle;
        entity.tick.now = Instant::from_millis(33);
        m.update(&mut entity);

        assert!(
            entity.motor.head_pose.pan_deg < -5.0,
            "left-exit lock loss should pan left, got {}",
            entity.motor.head_pose.pan_deg,
        );
    }
}
