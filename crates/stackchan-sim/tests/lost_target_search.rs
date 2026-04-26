//! Sim coverage for `LostTargetSearch` driven through the FULL
//! pipeline: AttentionFromTracking owns engagement, LostTargetSearch
//! reacts to the engagement falling-edge, all running through the
//! Director with realistic 30 FPS timing.
//!
//! The in-module unit tests directly mutate `entity.mind.engagement`
//! to set up the lock-loss edge. The sim tests here exercise the
//! *real* engagement transition that the firmware would see: a face
//! present for `FACE_LOCK_HITS` consecutive ticks → engaged →
//! face vanishes for `FACE_RELEASE_MISSES` consecutive ticks →
//! disengaged → search beat fires.
//!
//! Pinned contracts:
//!
//! - **Engagement falling-edge fires the search beat**: the search
//!   contribution actually shows up on the head pose after the
//!   AttentionFromTracking-driven engagement transition, not just
//!   after a hand-set engagement value.
//! - **Composition with the Motion stack**: the search beat rides on
//!   top of `IdleHeadDrift` + `HeadFromAttention` without breaking the
//!   pose clamps or causing any modifier to leak a stale offset.
//! - **Beat fully unwinds via diff-and-undo**: after `SEARCH_TOTAL_MS`,
//!   the modifier's contribution returns to zero — the head pose is
//!   back to the upstream baseline (head drift only).

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::unwrap_used,
    reason = "test-only relaxations: doc comments freely reference type names \
              without backticks; assertions use unwrap/expect/panic; long setup \
              blocks; baseline_*_x/y bindings share a common shape"
)]

use stackchan_core::modifiers::{
    AttentionFromTracking, FACE_LOCK_HITS, FACE_RELEASE_MISSES, GLANCE_PAN_MAX_DEG,
    HeadFromAttention, IdleHeadDrift, LostTargetSearch, SEARCH_HOLD_MS, SEARCH_TOTAL_MS,
};
use stackchan_core::{Director, Engagement, Entity, MAX_PAN_DEG, MAX_TILT_DEG, MIN_TILT_DEG, Pose};
use stackchan_sim::TrackingScenario;

/// Build a scenario that takes engagement Idle → Locked (face for
/// `FACE_LOCK_HITS+1` ticks) → Releasing → Idle (no face for
/// `FACE_RELEASE_MISSES` ticks), then idles long enough to walk the
/// full search beat. `centroid` is the face's normalised location for
/// the engaged window.
fn engaged_then_lost(target: Pose, centroid: (f32, f32)) -> TrackingScenario {
    let tick_ms = 33_u64;
    let s = TrackingScenario::new(tick_ms);
    let lock_window_ms = s.duration_for_ticks(u64::from(FACE_LOCK_HITS + 1));
    let release_window_ms = s.duration_for_ticks(u64::from(FACE_RELEASE_MISSES + 1));
    s.tracking(target, lock_window_ms)
        .with_face(centroid)
        // Tracking continues but face vanishes — drives the
        // engagement release path without releasing motion attention.
        .tracking(target, release_window_ms)
        // Then enough silence/returning to walk the entire search beat.
        .returning(SEARCH_TOTAL_MS + 500)
}

#[test]
fn engagement_falling_edge_fires_search_beat() {
    let mut afm = AttentionFromTracking::new();
    let mut search = LostTargetSearch::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut search).unwrap();

    let mut entity = Entity::default();

    // Face on the right of frame; lose it; expect search beat to
    // pan head right.
    let target = Pose::new(0.0, 0.0);
    let centroid = (0.4_f32, 0.0_f32);
    let scenario = engaged_then_lost(target, centroid);

    // Find the moment engagement transitions to Idle, then sample
    // shortly after — the head pose must deviate from baseline
    // toward the last-known centroid (positive pan).
    let mut peak_pan: f32 = 0.0;
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
        if matches!(entity.mind.engagement, Engagement::Idle) {
            peak_pan = peak_pan.max(entity.motor.head_pose.pan_deg);
        }
    }

    assert!(
        peak_pan > 5.0,
        "search beat should pan head toward last-known centroid (right); peak pan was {peak_pan}",
    );
}

#[test]
fn search_pose_stays_within_clamps_under_extreme_centroid() {
    // Centroid at (-0.99, +0.99) × the SEARCH_SACCADE_OVERSHOOT (1.3)
    // would otherwise push past MAX_PAN_DEG / MAX_TILT_DEG. The
    // per-tick clamp inside the search must keep the head safe.
    // Driven through the FULL pipeline so we cover both the
    // search clamp AND the Director's reclamp on motor.head_pose.
    let mut head_drift = IdleHeadDrift::new();
    let mut head = HeadFromAttention::new();
    let mut search = LostTargetSearch::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut head_drift).unwrap();
    director.add_modifier(&mut head).unwrap();
    director.add_modifier(&mut search).unwrap();

    let mut entity = Entity::default();

    // Face at extreme bottom-left of the frame.
    let target = Pose::new(-30.0, -15.0);
    let centroid = (-0.99_f32, 0.99_f32);
    let scenario = engaged_then_lost(target, centroid);
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
        assert!(
            entity.motor.head_pose.pan_deg.abs() <= MAX_PAN_DEG + 0.01,
            "pan {} out of range at {}ms",
            entity.motor.head_pose.pan_deg,
            now.as_millis(),
        );
        assert!(
            (MIN_TILT_DEG - 0.01..=MAX_TILT_DEG + 0.01).contains(&entity.motor.head_pose.tilt_deg),
            "tilt {} out of range at {}ms",
            entity.motor.head_pose.tilt_deg,
            now.as_millis(),
        );
    }
}

#[test]
fn beat_unwinds_to_head_drift_baseline_after_total_window() {
    // After SEARCH_TOTAL_MS past the lock-loss edge, the search
    // contribution must be fully unwound. Run with IdleHeadDrift + the
    // search modifier, drive a face → no-face → long quiet
    // sequence, then sample at the end. The pan must be within
    // IdleHeadDrift's amplitude.
    let mut head_drift = IdleHeadDrift::new();
    let mut search = LostTargetSearch::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut head_drift).unwrap();
    director.add_modifier(&mut search).unwrap();

    let mut entity = Entity::default();

    let target = Pose::new(0.0, 0.0);
    let centroid = (0.5_f32, 0.0_f32);
    let scenario = engaged_then_lost(target, centroid);
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
    }

    // Post-beat the head should be at the IdleHeadDrift baseline:
    // either at 0 (between glances) or up to ±GLANCE_PAN_MAX_DEG
    // (mid-glance). Bound covers both with 1° slack.
    let bound = GLANCE_PAN_MAX_DEG + 1.0;
    assert!(
        entity.motor.head_pose.pan_deg.abs() <= bound,
        "post-beat pan {} should be at head-drift baseline (\u{2264}\u{00b1}{bound}\u{00b0})",
        entity.motor.head_pose.pan_deg,
    );
}

#[test]
fn hold_phase_pans_toward_last_centroid_not_past_it() {
    // During the SEARCH_HOLD_MS hold phase, the head should be at
    // ~1.0× the centroid mapping (no overshoot yet). Sample shortly
    // after engagement drops, well inside the hold window.
    let mut search = LostTargetSearch::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut search).unwrap();

    let mut entity = Entity::default();

    let target = Pose::new(0.0, 0.0);
    let centroid = (0.4_f32, 0.0_f32);
    // 0.4 × HALF_FOV_H_DEG (31°) ≈ 12.4° expected during hold.
    let scenario = engaged_then_lost(target, centroid);

    let mut hold_pan_seen = None;
    let mut engagement_lost_at = None;
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
        if matches!(entity.mind.engagement, Engagement::Idle) && engagement_lost_at.is_none() {
            engagement_lost_at = Some(now);
        }
        if let Some(lost) = engagement_lost_at {
            let elapsed = now.as_millis().saturating_sub(lost.as_millis());
            // Sample mid-hold (between the lost edge and SEARCH_HOLD_MS).
            if elapsed > 100 && elapsed < SEARCH_HOLD_MS && hold_pan_seen.is_none() {
                hold_pan_seen = Some(entity.motor.head_pose.pan_deg);
            }
        }
    }

    let hold_pan = hold_pan_seen.expect("should have observed a tick mid-hold");
    // 12.4° ± 2° tolerance (search beat ~12.4°, no overshoot in hold).
    assert!(
        (10.0..=15.0).contains(&hold_pan),
        "hold-phase pan {hold_pan} should be at ~12.4° (1.0× centroid mapping)",
    );
}
