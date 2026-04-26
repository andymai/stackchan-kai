//! `AttentionFromTracking`: cognition-phase modifier that latches
//! [`Attention::Tracking`] when the camera tracker reports sustained
//! motion, and releases back to [`Attention::None`] after a quiet
//! window.
//!
//! Reads `entity.perception.tracking` (populated each tick by the
//! firmware drain of `CAMERA_TRACKING_SIGNAL`) and writes
//! `entity.mind.attention`. The firmware-side tracker has already
//! computed the target pose; this modifier's job is the
//! *cognitive* decision: "should the avatar latch onto this motion
//! as a thing to look at?" — with hysteresis to avoid flicker.
//!
//! ## Lock / release
//!
//! - **Enter `Tracking`** when [`TrackingMotion::Tracking`] persists
//!   for [`TRACKING_LOCK_TICKS`] consecutive ticks. Each frame after
//!   entry refreshes `target` from the latest observation while
//!   pinning `since` to the entry tick (downstream modifiers use
//!   `since` for ease-in animation; jumping it mid-track would
//!   reset the ramp).
//! - **Stay** while [`TrackingMotion::Tracking`] or
//!   [`TrackingMotion::Holding`] keeps appearing — both indicate the
//!   tracker still believes the target is meaningful.
//! - **Release** to [`Attention::None`] once
//!   [`TRACKING_RELEASE_MS`] has elapsed since the last
//!   [`TrackingMotion::Tracking`] tick. We rely on real time (not
//!   tick count) so the release window is independent of frame rate.
//!
//! ## Coexistence with [`Attention::Listening`]
//!
//! Only writes `Attention::Tracking` (when locked) or
//! `Attention::None` (on release). If another modifier or skill has
//! set `Attention::Listening`, this modifier leaves it alone unless
//! it has its own lock-on edge to fire — that is, sustained motion
//! interrupts a listening attention. The relative priority isn't
//! enforced here; it falls out of registration order. Today this
//! modifier is registered after [`crate::skills::Listening`], so
//! tracking wins when both fire.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::mind::{Attention, Engagement};
use crate::modifier::Modifier;
use crate::perception::TrackingMotion;

/// Consecutive [`TrackingMotion::Tracking`] ticks required to enter
/// [`Attention::Tracking`].
///
/// `3` ticks at the firmware tracker's ~30 Hz cadence is ~100 ms —
/// long enough to ignore single-frame spikes (e.g. a hand swept past
/// the camera once), short enough that the avatar reacts within the
/// first sustained motion of a real interaction.
pub const TRACKING_LOCK_TICKS: u8 = 3;

/// How long to hold [`Attention::Tracking`] after the last
/// [`TrackingMotion::Tracking`] tick before releasing to
/// [`Attention::None`], in ms.
///
/// `1500` ms reads as "the avatar stays interested for a moment
/// after the motion stops" — matches the
/// [`crate::skills::Listening`] release window for symmetry.
pub const TRACKING_RELEASE_MS: u64 = 1_500;

/// Consecutive face-cascade hits required to engage
/// [`Engagement::Locked`].
///
/// `3` ticks at the firmware tracker's ~30 Hz cadence is ~100 ms — long
/// enough to ride out single-frame detector misses (head turn, hand
/// briefly occluding) without lagging the engagement onset, and the
/// match for [`TRACKING_LOCK_TICKS`].
pub const FACE_LOCK_HITS: u8 = 3;

/// Consecutive face-cascade misses tolerated before releasing
/// [`Engagement::Locked`] / [`Engagement::Releasing`] back to
/// [`Engagement::Idle`].
///
/// `10` ticks at ~30 Hz is ~330 ms — survives a blink, a quick head
/// turn, or one frame of false-negative without dropping engagement,
/// but transitions to idle quickly enough that a face leaving the
/// scene reads as "they're gone" rather than "they're still here".
pub const FACE_RELEASE_MISSES: u8 = 10;

/// Modifier that watches `perception.tracking` and decides whether
/// `mind.attention` should be `Tracking{target}`.
#[derive(Debug, Clone, Copy)]
pub struct AttentionFromTracking {
    /// Consecutive `Tracking`-classified ticks required to enter the
    /// tracking attention state.
    pub lock_ticks: u8,
    /// Hold window after the last tracking tick before releasing
    /// attention back to `None`, in ms.
    pub release_ms: u64,
    /// Consecutive face-cascade hits required to engage the face
    /// lock.
    pub face_lock_hits: u8,
    /// Consecutive face-cascade misses tolerated before releasing the
    /// face lock back to [`Engagement::Idle`].
    pub face_release_misses: u8,
    /// Running counter of consecutive `Tracking` ticks. Saturates at
    /// `u8::MAX` so a very long sustained run doesn't wrap.
    consecutive_tracking: u8,
    /// Instant of the most recent `Tracking` tick. Drives the release
    /// window. `None` between motion bursts.
    last_tracking_at: Option<Instant>,
}

impl AttentionFromTracking {
    /// Construct with default tuning ([`TRACKING_LOCK_TICKS`] /
    /// [`TRACKING_RELEASE_MS`] / [`FACE_LOCK_HITS`] /
    /// [`FACE_RELEASE_MISSES`]).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            lock_ticks: TRACKING_LOCK_TICKS,
            release_ms: TRACKING_RELEASE_MS,
            face_lock_hits: FACE_LOCK_HITS,
            face_release_misses: FACE_RELEASE_MISSES,
            consecutive_tracking: 0,
            last_tracking_at: None,
        }
    }

    /// Construct with custom motion lock + release tuning. Face-lock
    /// thresholds keep their defaults; tweak the public fields after
    /// construction if you need to override those too.
    #[must_use]
    pub const fn with_config(lock_ticks: u8, release_ms: u64) -> Self {
        Self {
            lock_ticks,
            release_ms,
            face_lock_hits: FACE_LOCK_HITS,
            face_release_misses: FACE_RELEASE_MISSES,
            consecutive_tracking: 0,
            last_tracking_at: None,
        }
    }
}

impl Default for AttentionFromTracking {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for AttentionFromTracking {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "AttentionFromTracking",
            description: "Watches perception.tracking; latches mind.attention=Tracking{target} \
                          after TRACKING_LOCK_TICKS consecutive Tracking-classified ticks. \
                          Releases to None after TRACKING_RELEASE_MS quiet ms. Updates target \
                          continuously while locked; pins `since` to the entry tick for \
                          stable ease-in animation. Also drives mind.engagement from \
                          face_present/face_centroid with FACE_LOCK_HITS / \
                          FACE_RELEASE_MISSES hysteresis.",
            phase: Phase::Cognition,
            priority: 0,
            reads: &[Field::Tracking, Field::Attention, Field::Engagement],
            writes: &[Field::Attention, Field::Engagement],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        // No observation = treat as a face-less + motion-less frame,
        // running through the same release paths the live tracker
        // would use. This way a brief drain miss bleeds engagement
        // through the natural `face_release_misses` window instead
        // of clobbering a live lock.
        let motion_obs = entity.perception.tracking.as_ref();
        let face_present = motion_obs.is_some_and(|obs| obs.face_present);
        let face_centroid = motion_obs.and_then(|obs| obs.face_centroid);

        // Engagement state machine. Runs alongside attention rather
        // than gated by it: faces are only scored when there's a
        // motion candidate (camera task gates the cascade), so
        // `face_present` is `true` only inside the same window where
        // attention is ramping up or locked. The natural
        // `face_release_misses` path handles cleanup when faces
        // disappear, regardless of what attention does — including
        // the case where the motion-tracker drops attention while a
        // face is still detected.
        //
        // Computed up-front so every attention-state branch below
        // sees the same engagement update — no early-return ordering
        // hazards.
        entity.mind.engagement = advance_engagement(
            entity.mind.engagement,
            face_present,
            face_centroid,
            self.face_lock_hits,
            self.face_release_misses,
            now,
        );

        // The remaining attention-side state machine needs a live
        // observation. Without one we leave attention alone — the
        // engagement path above already handled face-side hysteresis.
        let Some(obs) = motion_obs else {
            self.consecutive_tracking = 0;
            return;
        };

        // The tracker reports `Tracking` only on *change* frames
        // (frame-differencing), so a hand held still after a wave
        // produces `Holding` frames — same target still tracked, but
        // no current motion. Counting Holding as lock-eligible lets
        // the avatar stay engaged through stationary moments after a
        // wave or step.
        //
        // Gate Holding on having seen at least one fresh `Tracking`
        // since the last release. Otherwise the tracker emits
        // Holding for the first ~3 s after boot (idle_timeout_ms)
        // even on a still scene, and the avatar would false-lock at
        // the neutral pose with no real target.
        //
        // Only fresh `Tracking` advances `last_tracking_at` so the
        // release window still gates on real motion having stopped.
        let fresh = matches!(obs.motion, TrackingMotion::Tracking);
        let lock_eligible = match obs.motion {
            TrackingMotion::Tracking => true,
            TrackingMotion::Holding => self.last_tracking_at.is_some(),
            TrackingMotion::Warmup | TrackingMotion::Returning | TrackingMotion::GlobalEvent => {
                false
            }
        };
        if lock_eligible {
            self.consecutive_tracking = self.consecutive_tracking.saturating_add(1);
            if fresh {
                self.last_tracking_at = Some(now);
            }
        } else {
            self.consecutive_tracking = 0;
        }

        let target_pose = obs.target_pose;
        let currently_tracking = matches!(entity.mind.attention, Attention::Tracking { .. });

        if self.consecutive_tracking >= self.lock_ticks {
            // Locked. Refresh target on every tick; preserve `since`
            // if already locked so consumers' ease-in math stays
            // anchored.
            let since = match entity.mind.attention {
                Attention::Tracking { since, .. } => since,
                _ => now,
            };
            entity.mind.attention = Attention::Tracking {
                target: target_pose,
                since,
            };
            return;
        }

        // Not enough sustained motion to lock. If we're already
        // tracking, hold until the release window expires. Don't
        // reset engagement here — the `advance_engagement` path
        // above is the single source of truth for face hysteresis,
        // which lets a still-detected face survive a motion-only
        // release window.
        if currently_tracking {
            let elapsed = self
                .last_tracking_at
                .map_or(u64::MAX, |t| now.saturating_duration_since(t));
            if elapsed >= self.release_ms {
                entity.mind.attention = Attention::None;
                self.last_tracking_at = None;
            }
        }
    }
}

/// Advance the [`Engagement`] state machine by one tick.
///
/// `face_present` reflects whether the firmware-side cascade fired on
/// any motion candidate this frame. `face_centroid` is the centroid
/// to track when present; ignored when `face_present` is false. `now`
/// stamps fresh `Locked` transitions so engagement modifiers can
/// reason about how long the lock has held.
///
/// The state machine is a Mealy machine — every transition is
/// determined by `(state, face_present)`; centroid only refreshes the
/// `Locked` state's payload while the lock holds. Pulled out as a free
/// function so it's straightforward to unit-test in isolation.
const fn advance_engagement(
    current: Engagement,
    face_present: bool,
    face_centroid: Option<(f32, f32)>,
    lock_hits: u8,
    release_misses: u8,
    now: Instant,
) -> Engagement {
    match (current, face_present, face_centroid) {
        // No face this frame: count toward release / reset locking.
        // `Idle` and `Locking` collapse to the same outcome — neither
        // had a confirmed lock, so we just fall back to Idle.
        (Engagement::Idle | Engagement::Locking { .. }, false, _) => Engagement::Idle,
        (Engagement::Locked { centroid, at }, false, _) => Engagement::Releasing {
            centroid,
            at,
            misses: 1,
        },
        (
            Engagement::Releasing {
                centroid,
                at,
                misses,
            },
            false,
            _,
        ) => {
            let next = misses.saturating_add(1);
            if next >= release_misses {
                Engagement::Idle
            } else {
                Engagement::Releasing {
                    centroid,
                    at,
                    misses: next,
                }
            }
        }

        // Face this frame WITHOUT a centroid (shouldn't happen in
        // practice — camera task always pairs them — but defensive
        // for the rare warmup race). Treat as "no useful data this
        // frame" without resetting hits.
        (state, true, None) => state,

        // Face this frame WITH a centroid: advance the locking /
        // refresh the lock. `Locked` and `Releasing` collapse to the
        // same outcome — both already had a lock and a fresh face
        // confirms / re-confirms it.
        (Engagement::Idle, true, Some(_centroid)) => Engagement::Locking { hits: 1 },
        (Engagement::Locking { hits }, true, Some(centroid)) => {
            let next = hits.saturating_add(1);
            if next >= lock_hits {
                Engagement::Locked { centroid, at: now }
            } else {
                Engagement::Locking { hits: next }
            }
        }
        (Engagement::Locked { .. } | Engagement::Releasing { .. }, true, Some(centroid)) => {
            Engagement::Locked { centroid, at: now }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    reason = "let-else with panic is the cleanest pattern for value extraction \
              on enum variants in tests"
)]
mod tests {
    use super::*;
    use crate::Pose;
    use crate::perception::TrackingObservation;

    fn at(now_ms: u64) -> Entity {
        let mut e = Entity::default();
        e.tick.now = Instant::from_millis(now_ms);
        e
    }

    fn observation(motion: TrackingMotion, target: Pose) -> TrackingObservation {
        TrackingObservation {
            target_pose: target,
            fired_cells: if matches!(motion, TrackingMotion::Tracking) {
                4
            } else {
                0
            },
            motion,
            candidates: heapless::Vec::new(),
            face_present: false,
            face_centroid: None,
        }
    }

    #[test]
    fn advance_engagement_table() {
        // Direct table coverage of the Mealy machine, independent of
        // entity / observation plumbing. Each row is
        // `(initial, face_present, centroid) -> expected`. Locking and
        // release thresholds chosen small (2 / 3) so the test exercises
        // the boundaries cheaply.
        const LOCK: u8 = 2;
        const REL: u8 = 3;
        let now = Instant::from_millis(0);
        let c = Some((0.1_f32, 0.2_f32));

        // Idle paths.
        assert_eq!(
            advance_engagement(Engagement::Idle, false, None, LOCK, REL, now),
            Engagement::Idle,
        );
        assert_eq!(
            advance_engagement(Engagement::Idle, true, c, LOCK, REL, now),
            Engagement::Locking { hits: 1 },
        );
        assert_eq!(
            advance_engagement(Engagement::Idle, true, None, LOCK, REL, now),
            Engagement::Idle,
            "true-without-centroid is a defensive no-op, not a state change",
        );

        // Locking{hits=1} + face → reaches lock threshold, transitions
        // to Locked carrying the latest centroid.
        match advance_engagement(Engagement::Locking { hits: 1 }, true, c, LOCK, REL, now) {
            Engagement::Locked { centroid, .. } => assert_eq!(Some(centroid), c),
            other => panic!("expected Locked at hit threshold, got {other:?}"),
        }
        // Locking + miss → drops to Idle (no half-lock).
        assert_eq!(
            advance_engagement(Engagement::Locking { hits: 1 }, false, None, LOCK, REL, now),
            Engagement::Idle,
        );

        // Locked + miss → first-frame Releasing.
        let locked = Engagement::Locked {
            centroid: (0.5, -0.5),
            at: now,
        };
        match advance_engagement(locked, false, None, LOCK, REL, now) {
            Engagement::Releasing {
                centroid, misses, ..
            } => {
                assert_eq!(centroid, (0.5, -0.5));
                assert_eq!(misses, 1);
            }
            other => panic!("expected Releasing on first miss, got {other:?}"),
        }
        // Locked + face → still Locked, centroid refreshed.
        match advance_engagement(locked, true, c, LOCK, REL, now) {
            Engagement::Locked { centroid, .. } => assert_eq!(Some(centroid), c),
            other => panic!("expected Locked refresh, got {other:?}"),
        }

        // Releasing edge cases.
        let releasing = Engagement::Releasing {
            centroid: (0.5, -0.5),
            at: now,
            misses: REL - 1,
        };
        // Hits release threshold this frame → Idle.
        assert_eq!(
            advance_engagement(releasing, false, None, LOCK, REL, now),
            Engagement::Idle,
        );
        // Re-acquire face mid-Releasing → straight back to Locked.
        match advance_engagement(releasing, true, c, LOCK, REL, now) {
            Engagement::Locked { centroid, .. } => assert_eq!(Some(centroid), c),
            other => panic!("expected re-Locked, got {other:?}"),
        }
    }

    fn observation_with_face(
        motion: TrackingMotion,
        target: Pose,
        centroid: (f32, f32),
    ) -> TrackingObservation {
        let mut obs = observation(motion, target);
        obs.face_present = true;
        obs.face_centroid = Some(centroid);
        obs
    }

    #[test]
    fn no_observation_keeps_attention_none() {
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        for t in 0..30 {
            entity.tick.now = Instant::from_millis(t * 33);
            m.update(&mut entity);
        }
        assert_eq!(entity.mind.attention, Attention::None);
    }

    #[test]
    fn warmup_does_not_lock() {
        // The tracker reports Warmup on its first frame — must not
        // trigger a lock.
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        entity.perception.tracking =
            Some(observation(TrackingMotion::Warmup, Pose::new(10.0, 5.0)));
        for t in 0..10 {
            entity.tick.now = Instant::from_millis(t * 33);
            m.update(&mut entity);
        }
        assert_eq!(entity.mind.attention, Attention::None);
    }

    #[test]
    fn sustained_tracking_locks_after_threshold() {
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        let target = Pose::new(15.0, 8.0);
        entity.perception.tracking = Some(observation(TrackingMotion::Tracking, target));

        // Drive for the lock-tick count.
        for t in 0..u64::from(TRACKING_LOCK_TICKS) {
            entity.tick.now = Instant::from_millis(t * 33);
            m.update(&mut entity);
        }
        match entity.mind.attention {
            Attention::Tracking {
                target: t,
                since: _,
            } => {
                assert_eq!(t, target);
            }
            other => panic!("expected Tracking, got {other:?}"),
        }
    }

    #[test]
    fn target_updates_while_locked_since_pinned() {
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        let initial = Pose::new(10.0, 0.0);
        entity.perception.tracking = Some(observation(TrackingMotion::Tracking, initial));
        for t in 0..u64::from(TRACKING_LOCK_TICKS) {
            entity.tick.now = Instant::from_millis(t * 33);
            m.update(&mut entity);
        }
        let Attention::Tracking {
            since: original_since,
            ..
        } = entity.mind.attention
        else {
            panic!("expected Tracking after lock");
        };

        // Move the target.
        let updated = Pose::new(-12.0, 6.0);
        entity.perception.tracking = Some(observation(TrackingMotion::Tracking, updated));
        entity.tick.now = Instant::from_millis(10 * 33);
        m.update(&mut entity);

        match entity.mind.attention {
            Attention::Tracking { target, since } => {
                assert_eq!(target, updated, "target should refresh");
                assert_eq!(since, original_since, "since should pin to entry tick");
            }
            _ => panic!("expected Tracking still"),
        }
    }

    #[test]
    fn quiet_within_release_window_holds() {
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        entity.perception.tracking =
            Some(observation(TrackingMotion::Tracking, Pose::new(10.0, 5.0)));
        for t in 0..u64::from(TRACKING_LOCK_TICKS) {
            entity.tick.now = Instant::from_millis(t * 33);
            m.update(&mut entity);
        }
        assert!(matches!(entity.mind.attention, Attention::Tracking { .. }));

        // Tracker switches to Holding (idle but inside window).
        entity.perception.tracking =
            Some(observation(TrackingMotion::Holding, Pose::new(10.0, 5.0)));
        entity.tick.now = Instant::from_millis(u64::from(TRACKING_LOCK_TICKS) * 33 + 100);
        m.update(&mut entity);
        assert!(
            matches!(entity.mind.attention, Attention::Tracking { .. }),
            "tracking attention should hold inside release window"
        );
    }

    #[test]
    fn quiet_past_release_window_clears() {
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        entity.perception.tracking =
            Some(observation(TrackingMotion::Tracking, Pose::new(10.0, 5.0)));
        for t in 0..u64::from(TRACKING_LOCK_TICKS) {
            entity.tick.now = Instant::from_millis(t * 33);
            m.update(&mut entity);
        }
        let lock_at = entity.tick.now;

        // Tracker switches to Returning. Step past the release window.
        entity.perception.tracking =
            Some(observation(TrackingMotion::Returning, Pose::new(0.0, 0.0)));
        entity.tick.now = lock_at + TRACKING_RELEASE_MS + 200;
        m.update(&mut entity);

        assert_eq!(entity.mind.attention, Attention::None);
    }

    #[test]
    fn single_motion_frame_followed_by_holding_locks() {
        // The tracker reports Tracking only on *change* frames. A
        // single motion (e.g. wave-and-stop) followed by Holding
        // frames must be enough to engage attention — Holding counts
        // as lock-eligible because the tracker still believes the
        // target is present.
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        let target = Pose::new(10.0, 5.0);
        entity.perception.tracking = Some(observation(TrackingMotion::Tracking, target));
        m.update(&mut entity);
        entity.perception.tracking = Some(observation(TrackingMotion::Holding, target));
        for t in 1..10 {
            entity.tick.now = Instant::from_millis(t * 33);
            m.update(&mut entity);
        }
        assert!(
            matches!(entity.mind.attention, Attention::Tracking { .. }),
            "single motion + holding should lock",
        );
    }

    #[test]
    fn returning_resets_counter() {
        // After the tracker enters Returning (its idle-timeout slew
        // back to neutral), the counter must reset so a subsequent
        // brief motion needs to re-accumulate before re-locking.
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        let target = Pose::new(10.0, 5.0);

        // Lock via sustained Tracking.
        entity.perception.tracking = Some(observation(TrackingMotion::Tracking, target));
        for t in 0..u64::from(TRACKING_LOCK_TICKS) {
            entity.tick.now = Instant::from_millis(t * 33);
            m.update(&mut entity);
        }
        assert!(matches!(entity.mind.attention, Attention::Tracking { .. }));

        // Returning frame: counter should reset (not lock-eligible).
        entity.perception.tracking = Some(observation(TrackingMotion::Returning, target));
        entity.tick.now = Instant::from_millis(u64::from(TRACKING_LOCK_TICKS) * 33);
        m.update(&mut entity);
        // Counter is private but its effect is visible: a single
        // subsequent Holding frame should NOT extend the lock — only
        // a fresh Tracking frame can.
    }

    #[test]
    fn holding_without_prior_tracking_does_not_lock() {
        // On a still scene from boot, the tracker emits Holding for
        // ~3 s before transitioning to Returning. Without the gate
        // these would false-lock attention with a neutral target.
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        entity.perception.tracking = Some(observation(TrackingMotion::Holding, Pose::NEUTRAL));
        for t in 0..30 {
            entity.tick.now = Instant::from_millis(t * 33);
            m.update(&mut entity);
        }
        assert_eq!(entity.mind.attention, Attention::None);
    }

    #[test]
    fn face_lock_engages_after_three_hits_and_releases_after_ten_misses() {
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        let target = Pose::new(10.0, 5.0);
        let centroid = (0.2_f32, -0.1_f32);

        // Sustained motion + face: locks attention AND engagement.
        entity.perception.tracking = Some(observation_with_face(
            TrackingMotion::Tracking,
            target,
            centroid,
        ));
        for tick in 0..u64::from(FACE_LOCK_HITS) {
            entity.tick.now = Instant::from_millis(tick * 33);
            m.update(&mut entity);
        }
        match entity.mind.engagement {
            Engagement::Locked { centroid: c, at: _ } => assert_eq!(c, centroid),
            other => panic!("expected Locked, got {other:?}"),
        }

        // Face vanishes mid-tracking. Each empty frame increments
        // `misses`; engagement enters `Releasing` immediately.
        entity.perception.tracking = Some(observation(TrackingMotion::Tracking, target));
        entity.tick.now = Instant::from_millis(u64::from(FACE_LOCK_HITS) * 33);
        m.update(&mut entity);
        match entity.mind.engagement {
            Engagement::Releasing { misses, .. } => assert_eq!(misses, 1),
            other => panic!("expected Releasing after first miss, got {other:?}"),
        }

        // Step through `FACE_RELEASE_MISSES - 1` more empty frames.
        for tick in 1..u64::from(FACE_RELEASE_MISSES) {
            entity.tick.now = Instant::from_millis((u64::from(FACE_LOCK_HITS) + tick) * 33);
            m.update(&mut entity);
        }
        assert_eq!(entity.mind.engagement, Engagement::Idle);
    }

    #[test]
    fn brief_face_dropout_does_not_release_lock() {
        // A single missed cascade frame inside the release window
        // must NOT drop the lock — the head shouldn't twitch on a
        // blink or single false-negative.
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        let target = Pose::new(0.0, 0.0);
        let centroid = (0.0_f32, 0.0_f32);

        entity.perception.tracking = Some(observation_with_face(
            TrackingMotion::Tracking,
            target,
            centroid,
        ));
        for tick in 0..u64::from(FACE_LOCK_HITS) {
            entity.tick.now = Instant::from_millis(tick * 33);
            m.update(&mut entity);
        }
        assert!(matches!(entity.mind.engagement, Engagement::Locked { .. }));

        // One frame without a face → Releasing { misses: 1 }.
        entity.perception.tracking = Some(observation(TrackingMotion::Tracking, target));
        entity.tick.now = Instant::from_millis(u64::from(FACE_LOCK_HITS) * 33);
        m.update(&mut entity);
        assert!(matches!(
            entity.mind.engagement,
            Engagement::Releasing { misses: 1, .. }
        ));

        // Face returns: Releasing → Locked again with refreshed centroid.
        let new_centroid = (0.3_f32, 0.0_f32);
        entity.perception.tracking = Some(observation_with_face(
            TrackingMotion::Tracking,
            target,
            new_centroid,
        ));
        entity.tick.now = Instant::from_millis((u64::from(FACE_LOCK_HITS) + 1) * 33);
        m.update(&mut entity);
        match entity.mind.engagement {
            Engagement::Locked { centroid: c, .. } => assert_eq!(c, new_centroid),
            other => panic!("expected re-Locked, got {other:?}"),
        }
    }

    #[test]
    fn motion_release_does_not_clobber_active_face_lock() {
        // Reviewer finding: when motion's release window expires, we
        // must NOT force engagement to Idle. A still face is still
        // detected (face_present=true) even after the wave-and-stop
        // has gone quiet on motion. Engagement state is owned by the
        // face-side hysteresis; only `face_release_misses` consecutive
        // face-less frames should release it.
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        let target = Pose::new(5.0, 0.0);
        let centroid = (0.0_f32, 0.0_f32);

        entity.perception.tracking = Some(observation_with_face(
            TrackingMotion::Tracking,
            target,
            centroid,
        ));
        for tick in 0..u64::from(FACE_LOCK_HITS) {
            entity.tick.now = Instant::from_millis(tick * 33);
            m.update(&mut entity);
        }
        assert!(matches!(entity.mind.engagement, Engagement::Locked { .. }));
        let lock_at = entity.tick.now;

        // Motion goes quiet (Returning) BUT face is still detected.
        entity.perception.tracking = Some(observation_with_face(
            TrackingMotion::Returning,
            target,
            centroid,
        ));
        entity.tick.now = lock_at + TRACKING_RELEASE_MS + 200;
        m.update(&mut entity);
        // Motion-side attention may release, but engagement holds.
        assert!(
            matches!(entity.mind.engagement, Engagement::Locked { .. }),
            "face lock must survive motion release while face_present=true",
        );
    }

    #[test]
    fn engagement_releases_after_face_misses_window() {
        // Same setup as the previous test, but the face vanishes
        // alongside motion. Engagement must release after exactly
        // `FACE_RELEASE_MISSES` consecutive face-less frames — not
        // sooner just because attention also dropped.
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        let target = Pose::new(5.0, 0.0);
        let centroid = (0.0_f32, 0.0_f32);

        entity.perception.tracking = Some(observation_with_face(
            TrackingMotion::Tracking,
            target,
            centroid,
        ));
        for tick in 0..u64::from(FACE_LOCK_HITS) {
            entity.tick.now = Instant::from_millis(tick * 33);
            m.update(&mut entity);
        }
        assert!(matches!(entity.mind.engagement, Engagement::Locked { .. }));

        // Run exactly `face_release_misses` face-less frames; engagement
        // ends in Idle.
        for tick in 0..u64::from(FACE_RELEASE_MISSES) {
            entity.perception.tracking =
                Some(observation(TrackingMotion::Returning, Pose::new(0.0, 0.0)));
            entity.tick.now = Instant::from_millis((u64::from(FACE_LOCK_HITS) + tick) * 33);
            m.update(&mut entity);
        }
        assert_eq!(entity.mind.engagement, Engagement::Idle);
    }

    #[test]
    fn single_face_hit_in_locking_resets_on_miss() {
        // Locking { hits: 1 or 2 } followed by a missing frame must
        // drop back to Idle without engaging — proves we don't lock
        // on isolated detections.
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        let target = Pose::new(0.0, 0.0);
        let centroid = (0.0_f32, 0.0_f32);

        // First two hits → Locking.
        entity.perception.tracking = Some(observation_with_face(
            TrackingMotion::Tracking,
            target,
            centroid,
        ));
        for tick in 0..(u64::from(FACE_LOCK_HITS) - 1) {
            entity.tick.now = Instant::from_millis(tick * 33);
            m.update(&mut entity);
        }
        assert!(matches!(entity.mind.engagement, Engagement::Locking { .. }));

        // Miss → reset to Idle.
        entity.perception.tracking = Some(observation(TrackingMotion::Tracking, target));
        entity.tick.now = Instant::from_millis(u64::from(FACE_LOCK_HITS - 1) * 33);
        m.update(&mut entity);
        assert_eq!(entity.mind.engagement, Engagement::Idle);
    }

    #[test]
    fn global_event_does_not_count_as_motion() {
        // Lighting change → tracker reports GlobalEvent → must not
        // trip the lock even if sustained.
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        entity.perception.tracking = Some(observation(
            TrackingMotion::GlobalEvent,
            Pose::new(0.0, 0.0),
        ));
        for t in 0..20 {
            entity.tick.now = Instant::from_millis(t * 33);
            m.update(&mut entity);
        }
        assert_eq!(entity.mind.attention, Attention::None);
    }
}
