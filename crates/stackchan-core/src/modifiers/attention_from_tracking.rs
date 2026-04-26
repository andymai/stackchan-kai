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
use crate::mind::Attention;
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
    /// Running counter of consecutive `Tracking` ticks. Saturates at
    /// `u8::MAX` so a very long sustained run doesn't wrap.
    consecutive_tracking: u8,
    /// Instant of the most recent `Tracking` tick. Drives the release
    /// window. `None` between motion bursts.
    last_tracking_at: Option<Instant>,
}

impl AttentionFromTracking {
    /// Construct with default tuning ([`TRACKING_LOCK_TICKS`] /
    /// [`TRACKING_RELEASE_MS`]).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            lock_ticks: TRACKING_LOCK_TICKS,
            release_ms: TRACKING_RELEASE_MS,
            consecutive_tracking: 0,
            last_tracking_at: None,
        }
    }

    /// Construct with custom lock + release tuning.
    #[must_use]
    pub const fn with_config(lock_ticks: u8, release_ms: u64) -> Self {
        Self {
            lock_ticks,
            release_ms,
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
                          stable ease-in animation.",
            phase: Phase::Cognition,
            priority: 0,
            reads: &[Field::Tracking, Field::Attention],
            writes: &[Field::Attention],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let Some(obs) = entity.perception.tracking.as_ref() else {
            // No observation yet — leave attention alone, reset
            // counter so the next non-None obs starts fresh.
            self.consecutive_tracking = 0;
            return;
        };

        let active = matches!(obs.motion, TrackingMotion::Tracking);
        if active {
            self.consecutive_tracking = self.consecutive_tracking.saturating_add(1);
            self.last_tracking_at = Some(now);
        } else {
            self.consecutive_tracking = 0;
        }

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
                target: obs.target_pose,
                since,
            };
            return;
        }

        // Not enough sustained motion to lock. If we're already
        // tracking, hold until the release window expires.
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
        }
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
    fn single_frame_motion_below_threshold_does_not_lock() {
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        // One frame of Tracking, then immediately quiet.
        entity.perception.tracking =
            Some(observation(TrackingMotion::Tracking, Pose::new(10.0, 5.0)));
        m.update(&mut entity);
        entity.perception.tracking =
            Some(observation(TrackingMotion::Holding, Pose::new(10.0, 5.0)));
        for t in 1..10 {
            entity.tick.now = Instant::from_millis(t * 33);
            m.update(&mut entity);
        }
        assert_eq!(entity.mind.attention, Attention::None);
    }

    #[test]
    fn quiet_tick_resets_counter_mid_burst() {
        // LOCK_TICKS - 1 Tracking, one Holding (resets), then more
        // Tracking — should NOT lock because each run is below the
        // threshold.
        let mut m = AttentionFromTracking::new();
        let mut entity = at(0);
        let target = Pose::new(10.0, 5.0);

        for t in 0..(u64::from(TRACKING_LOCK_TICKS) - 1) {
            entity.tick.now = Instant::from_millis(t * 33);
            entity.perception.tracking = Some(observation(TrackingMotion::Tracking, target));
            m.update(&mut entity);
        }
        // Quiet tick.
        entity.tick.now = Instant::from_millis((u64::from(TRACKING_LOCK_TICKS)) * 33);
        entity.perception.tracking = Some(observation(TrackingMotion::Holding, target));
        m.update(&mut entity);
        // Another partial run, still below threshold.
        for t in 0..(u64::from(TRACKING_LOCK_TICKS) - 1) {
            entity.tick.now = Instant::from_millis((u64::from(TRACKING_LOCK_TICKS) + 1 + t) * 33);
            entity.perception.tracking = Some(observation(TrackingMotion::Tracking, target));
            m.update(&mut entity);
        }
        assert_eq!(entity.mind.attention, Attention::None);
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
