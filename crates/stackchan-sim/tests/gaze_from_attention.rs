//! Sim coverage for `GazeFromAttention` composed with the rest of the
//! Expression-phase eye-center stack via the Director.
//!
//! The in-module unit tests cover the modifier in isolation. The sim
//! tests here pin the *composition* contracts that only show up when
//! GazeFromAttention runs alongside IdleDrift (also writes
//! `face.{left,right}_eye.center`), in canonical priority order
//! (IdleDrift = 0 → GazeFromAttention = 5).
//!
//! Pinned contracts:
//!
//! - **Diff-and-undo composes with IdleDrift over time**: across a
//!   30 s realistic Tracking → release sequence, eye centers stay
//!   within `±(GAZE_MAX_OFFSET_PX + drift_max + safety)` per axis.
//!   The diff-and-undo bookkeeping (storing only the prior gaze
//!   delta, not the absolute eye position) means IdleDrift's
//!   independent random offsets accumulate cleanly without being
//!   clobbered.
//! - **Release returns eyes to a drift-only baseline**: when the
//!   tracker observation stops and attention releases, the gaze
//!   contribution unwinds and the eye positions converge back to
//!   IdleDrift's neutral wander (close to baseline).
//! - **Full Cognition → Expression pipeline**: AttentionFromTracking
//!   writes `mind.attention`, GazeFromAttention reads it the same
//!   tick — via the Director's phase ordering rather than direct
//!   modifier calls.

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
    AttentionFromTracking, GAZE_MAX_OFFSET_PX, GazeFromAttention, IDLE_DRIFT_MAX_X,
    IDLE_DRIFT_MAX_Y, IdleDrift, TRACKING_LOCK_TICKS, TRACKING_RELEASE_MS,
};
use stackchan_core::{Director, Entity, Pose};
use stackchan_sim::TrackingScenario;

#[test]
fn gaze_plus_drift_stays_within_combined_bound() {
    // Compose GazeFromAttention with IdleDrift in Expression phase
    // and run 30 s with active Tracking attention. The combined eye-
    // center offset from baseline must not exceed the sum of
    // GAZE_MAX_OFFSET_PX and the drift's per-axis max — proves the
    // diff-and-undo bookkeeping doesn't accumulate stale offsets.
    let mut gaze = GazeFromAttention::new();
    let mut drift = IdleDrift::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut drift).unwrap();
    director.add_modifier(&mut gaze).unwrap();

    let mut entity = Entity::default();
    let baseline_left_x = entity.face.left_eye.center.x;
    let baseline_left_y = entity.face.left_eye.center.y;
    let baseline_right_x = entity.face.right_eye.center.x;

    let target = Pose::new(20.0, 10.0); // beyond the gaze clamp
    let scenario = TrackingScenario::new(33).tracking(target, 30_000);

    // Combined bound: gaze (clamped to GAZE_MAX_OFFSET_PX) + drift
    // (DEFAULT_MAX_X) + 1 px slack for the integer arithmetic.
    let max_dx = GAZE_MAX_OFFSET_PX + IDLE_DRIFT_MAX_X + 1;
    let max_dy = GAZE_MAX_OFFSET_PX + IDLE_DRIFT_MAX_Y + 1;

    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);

        let dx = (entity.face.left_eye.center.x - baseline_left_x).abs();
        let dy = (entity.face.left_eye.center.y - baseline_left_y).abs();
        assert!(
            dx <= max_dx,
            "left-eye dx={dx} exceeds combined bound {max_dx} at {}ms",
            now.as_millis(),
        );
        assert!(
            dy <= max_dy,
            "left-eye dy={dy} exceeds combined bound {max_dy} at {}ms",
            now.as_millis(),
        );
        // Both eyes share the same gaze + drift contribution per the
        // modifier contracts, so the right-eye delta must mirror the
        // left's.
        let right_dx = (entity.face.right_eye.center.x - baseline_right_x).abs();
        assert_eq!(
            dx,
            right_dx,
            "right-eye delta should mirror left-eye delta at {}ms",
            now.as_millis(),
        );
    }
}

#[test]
fn release_returns_eyes_to_drift_only_baseline() {
    // After the tracking burst ends and the release window expires,
    // the gaze contribution unwinds. The eye position should be
    // within the drift's max excursion of baseline — proving the
    // gaze diff-and-undo cleared cleanly even though drift kept
    // mutating the same field across the same window.
    let mut gaze = GazeFromAttention::new();
    let mut drift = IdleDrift::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut drift).unwrap();
    director.add_modifier(&mut gaze).unwrap();

    let mut entity = Entity::default();
    let baseline_x = entity.face.left_eye.center.x;
    let baseline_y = entity.face.left_eye.center.y;

    let target = Pose::new(15.0, 8.0);
    let scenario = TrackingScenario::new(33)
        .tracking(target, u64::from(TRACKING_LOCK_TICKS) * 33 + 1_000)
        // Long Returning past the release window — silent doesn't
        // drive release in AttentionFromTracking.
        .returning(TRACKING_RELEASE_MS + 1_000);

    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
    }

    // Post-release: gaze contribution is gone; only drift remains.
    // Drift wanders up to ±DRIFT_MAX per axis from baseline — allow
    // 1 px slack for the diff-and-undo arithmetic.
    let dx = (entity.face.left_eye.center.x - baseline_x).abs();
    let dy = (entity.face.left_eye.center.y - baseline_y).abs();
    assert!(
        dx <= IDLE_DRIFT_MAX_X + 1,
        "post-release dx={dx} exceeds drift bound {} — gaze contribution may not have unwound",
        IDLE_DRIFT_MAX_X + 1,
    );
    assert!(
        dy <= IDLE_DRIFT_MAX_Y + 1,
        "post-release dy={dy} exceeds drift bound {} — gaze contribution may not have unwound",
        IDLE_DRIFT_MAX_Y + 1,
    );
}

#[test]
fn cognition_to_expression_pipeline_drives_eyes_within_one_director_run() {
    // Critical phase-ordering pin: AttentionFromTracking (Cognition,
    // priority 0) runs BEFORE GazeFromAttention (Expression, priority
    // 5) in the same Director::run, so the gaze modifier can read
    // the freshly-set `mind.attention` without a one-tick delay.
    //
    // A future refactor that put GazeFromAttention in an earlier
    // phase, or AttentionFromTracking in a later one, would surface
    // a one-tick lag that this test catches: after exactly
    // TRACKING_LOCK_TICKS Director runs, eyes must already be shifted.
    let mut gaze = GazeFromAttention::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut gaze).unwrap();

    let mut entity = Entity::default();
    let baseline_x = entity.face.left_eye.center.x;

    // Drive exactly TRACKING_LOCK_TICKS of Tracking observations.
    let target = Pose::new(10.0, 0.0); // +5 px after gaze mapping
    let scenario = TrackingScenario::new(33).tracking(target, u64::from(TRACKING_LOCK_TICKS) * 33);
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
    }

    // On the lock tick, gaze sees Attention::Tracking and shifts
    // eyes the same Director run. No one-tick lag.
    assert_ne!(
        entity.face.left_eye.center.x, baseline_x,
        "eye should shift on the lock tick, not one tick later",
    );
    assert!(
        entity.face.left_eye.center.x > baseline_x,
        "positive pan target should shift eyes to the right",
    );
}
