//! Sim coverage for `MicrosaccadeFromAttention` in composition with
//! `GazeFromAttention` (the gross tracking offset it overlays) and
//! `IdleDrift` (the random eye-center wander) — driven through the
//! Director with realistic 30 FPS timing.
//!
//! The in-module unit tests cover single-modifier behaviour. The sim
//! tests here pin:
//!
//! - **Interval distribution under realistic cadence**: across 30 s
//!   of Tracking attention, the count of distinct microsaccade events
//!   falls inside the bounds the
//!   `MICROSACCADE_INTERVAL_{MIN,MAX}_MS` constants imply
//!   (`30s / max ≤ N ≤ 30s / min`). A drift in the interval
//!   distribution would surface here without anyone needing to time
//!   the avatar with a stopwatch.
//! - **Composition with GazeFromAttention preserves the gross offset**:
//!   while microsaccades fire, the eyes still sit near the gaze
//!   target — the additive jitter must not cancel the gross offset.
//! - **Reset on transition out of Tracking**: after the lock releases,
//!   the eyes settle back to a drift-only baseline within the drift
//!   bound. Pins the cleanup contract documented in the modifier.

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
    AttentionFromTracking, GazeFromAttention, IDLE_DRIFT_MAX_X, IDLE_DRIFT_MAX_Y, IdleDrift,
    MICROSACCADE_AMPLITUDE_PX, MICROSACCADE_DURATION_MS, MICROSACCADE_INTERVAL_MAX_MS,
    MICROSACCADE_INTERVAL_MIN_MS, MicrosaccadeFromAttention, TRACKING_LOCK_TICKS,
    TRACKING_RELEASE_MS,
};
use stackchan_core::{Director, Entity, Pose};
use stackchan_sim::TrackingScenario;

#[test]
fn microsaccade_count_over_30s_falls_inside_interval_bounds() {
    // The modifier schedules each microsaccade between
    // MICROSACCADE_INTERVAL_MIN_MS (500 ms) and
    // MICROSACCADE_INTERVAL_MAX_MS (1500 ms) of dwell time. So in a
    // 30 s tracking window:
    //
    //   min count = 30_000 / max_interval ≈ 20
    //   max count = 30_000 / (min_interval + duration) ≈ 53
    //
    // Add slack on both ends for the dwell that contains tick 0 and
    // the dwell that wraps past the 30 s boundary.
    let mut micro = MicrosaccadeFromAttention::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut micro).unwrap();

    let mut entity = Entity::default();
    let baseline_x = entity.face.left_eye.center.x;
    let baseline_y = entity.face.left_eye.center.y;

    let target = Pose::new(0.0, 0.0); // gaze isn't in this stack
    let scenario = TrackingScenario::new(33).tracking(target, 30_000);

    // Count rising edges into a non-zero offset (microsaccade fires).
    let mut events: u32 = 0;
    let mut prev_offset = (0_i32, 0_i32);
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
        let dx = entity.face.left_eye.center.x - baseline_x;
        let dy = entity.face.left_eye.center.y - baseline_y;
        if (dx, dy) != (0, 0) && prev_offset == (0, 0) {
            events += 1;
        }
        prev_offset = (dx, dy);
    }

    // Bounds: derived from the constants above. Each saccade consumes
    // MICROSACCADE_DURATION_MS of held offset before the *next*
    // interval rolls, so the lower bound uses (MAX + DURATION) for
    // honesty — otherwise a future bump to MICROSACCADE_DURATION_MS
    // would silently drift this assertion to flaky.
    let max_count = 30_000_u64 / MICROSACCADE_INTERVAL_MIN_MS + 5;
    let min_count = 30_000_u64 / (MICROSACCADE_INTERVAL_MAX_MS + MICROSACCADE_DURATION_MS);
    assert!(
        u64::from(events) >= min_count,
        "expected at least {min_count} microsaccades over 30 s, saw {events}",
    );
    assert!(
        u64::from(events) <= max_count,
        "expected at most {max_count} microsaccades over 30 s, saw {events}",
    );
}

#[test]
fn microsaccade_amplitude_stays_within_max_per_axis() {
    // Every observed jitter must be within ±MICROSACCADE_AMPLITUDE_PX
    // on each axis — proves the random-offset path can't blow past
    // the documented bound even across many events.
    let mut micro = MicrosaccadeFromAttention::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut micro).unwrap();

    let mut entity = Entity::default();
    let baseline_x = entity.face.left_eye.center.x;
    let baseline_y = entity.face.left_eye.center.y;

    let scenario = TrackingScenario::new(33).tracking(Pose::new(0.0, 0.0), 60_000);
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
        let dx = entity.face.left_eye.center.x - baseline_x;
        let dy = entity.face.left_eye.center.y - baseline_y;
        assert!(
            dx.abs() <= MICROSACCADE_AMPLITUDE_PX,
            "microsaccade dx {dx} exceeds amplitude bound at {}ms",
            now.as_millis(),
        );
        assert!(
            dy.abs() <= MICROSACCADE_AMPLITUDE_PX,
            "microsaccade dy {dy} exceeds amplitude bound at {}ms",
            now.as_millis(),
        );
    }
}

#[test]
fn composes_with_gaze_keeping_eyes_near_gross_offset() {
    // GazeFromAttention shifts eyes to the gross target; microsaccades
    // overlay ≤±MICROSACCADE_AMPLITUDE_PX jitter. The combined offset
    // from baseline must always sit between
    // (gross - MICROSACCADE_AMPLITUDE_PX) and
    // (gross + MICROSACCADE_AMPLITUDE_PX) — the additive jitter
    // can't accidentally cancel the gross offset.
    let mut gaze = GazeFromAttention::new();
    let mut micro = MicrosaccadeFromAttention::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut gaze).unwrap();
    director.add_modifier(&mut micro).unwrap();

    let mut entity = Entity::default();
    let baseline_x = entity.face.left_eye.center.x;

    // Pan target chosen so gaze offset = +5 px (well inside the
    // GAZE_MAX_OFFSET_PX clamp, leaves headroom for microsaccades).
    let target = Pose::new(10.0, 0.0);
    let scenario = TrackingScenario::new(33).tracking(target, 5_000);
    let gross_dx = 5_i32;
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
        // Skip the very first lock-tick: gaze + microsaccade may
        // both transition on the same tick, allow a single-tick
        // settling window.
        if now.as_millis() < (u64::from(TRACKING_LOCK_TICKS) + 1) * 33 {
            continue;
        }
        let dx = entity.face.left_eye.center.x - baseline_x;
        let lo = gross_dx - MICROSACCADE_AMPLITUDE_PX;
        let hi = gross_dx + MICROSACCADE_AMPLITUDE_PX;
        assert!(
            (lo..=hi).contains(&dx),
            "combined dx={dx} outside [{lo}, {hi}] at {}ms — additive composition broke",
            now.as_millis(),
        );
    }
}

#[test]
fn reset_on_release_returns_to_drift_only_baseline() {
    // After tracking releases, the microsaccade contribution must
    // fully unwind. With IdleDrift in the stack the post-release
    // position should sit within drift's bound of baseline.
    let mut drift = IdleDrift::new();
    let mut micro = MicrosaccadeFromAttention::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut drift).unwrap();
    director.add_modifier(&mut micro).unwrap();

    let mut entity = Entity::default();
    let baseline_x = entity.face.left_eye.center.x;
    let baseline_y = entity.face.left_eye.center.y;

    let target = Pose::new(8.0, 4.0);
    let scenario = TrackingScenario::new(33)
        .tracking(target, u64::from(TRACKING_LOCK_TICKS) * 33 + 5_000)
        .returning(TRACKING_RELEASE_MS + 1_000);
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
    }

    let dx = (entity.face.left_eye.center.x - baseline_x).abs();
    let dy = (entity.face.left_eye.center.y - baseline_y).abs();
    assert!(
        dx <= IDLE_DRIFT_MAX_X + 1,
        "post-release dx={dx} exceeds drift bound {} — microsaccade contribution may not have unwound",
        IDLE_DRIFT_MAX_X + 1,
    );
    assert!(
        dy <= IDLE_DRIFT_MAX_Y + 1,
        "post-release dy={dy} exceeds drift bound {} — microsaccade contribution may not have unwound",
        IDLE_DRIFT_MAX_Y + 1,
    );
}
