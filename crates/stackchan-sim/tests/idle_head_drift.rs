//! Golden test for `IdleHeadDrift`: drives the modifier + a
//! [`RecordingHead`] over a long simulated window at 30 FPS, then
//! asserts the captured pan/tilt trajectory respects the per-axis
//! glance amplitude AND that the head spends most of its time at
//! rest (the contract that distinguishes the new event-driven
//! behaviour from the old continuous triangle wave).
//!
//! The test exercises the full shape of the firmware port: the
//! modifier writes `avatar.motor.head_pose`, a consumer pulls the
//! pose and calls [`HeadDriver::set_pose`] on a `RecordingHead`
//! (sim) — the same code path the firmware uses against the SCServo
//! head driver.

#![allow(
    clippy::cast_precision_loss,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::unwrap_used,
    reason = "test-only: doc references type names without backticks; \
              compares bit-exact pose values flowing through RecordingHead; \
              counters fit well inside f32 mantissa for trajectory-fraction math; \
              RecordingHead::set_pose is infallible so unwrap/expect can't fire"
)]

use stackchan_core::modifiers::{
    GLANCE_EASE_IN_MS, GLANCE_EASE_OUT_MS, GLANCE_HOLD_MS, GLANCE_INTERVAL_MAX_MS,
    GLANCE_INTERVAL_MIN_MS, GLANCE_PAN_MAX_DEG, GLANCE_TILT_MAX_DEG, IdleHeadDrift,
};
use stackchan_core::{Clock, Entity, HeadDriver, Modifier};
use stackchan_sim::{FakeClock, RecordingHead, block_on};

// 90 s captures multiple full glance cycles (avg interval ~10 s, plus
// a ~2.1 s glance window).
const DURATION_MS: u64 = 90_000;
const TICK_MS: u64 = 33;

#[test]
fn trajectory_respects_per_axis_glance_amplitude() {
    let clock = FakeClock::new();
    let mut entity = Entity::default();
    let mut head_drift = IdleHeadDrift::new();
    let mut recorder = RecordingHead::new();

    let mut t_ms = 0;
    while t_ms <= DURATION_MS {
        clock.set(stackchan_core::Instant::from_millis(t_ms));
        entity.tick.now = clock.now();
        head_drift.update(&mut entity);
        block_on(recorder.set_pose(entity.motor.head_pose, clock.now()))
            .expect("RecordingHead is infallible");
        t_ms += TICK_MS;
    }

    let records = recorder.records();
    assert!(
        records.len() > 2_500,
        "expected ~2_727 records over 90 s at 30 FPS, got {}",
        records.len(),
    );

    for (ts, pose) in records {
        assert!(
            pose.pan_deg.abs() <= GLANCE_PAN_MAX_DEG + 0.01,
            "pan {} at {}ms exceeds GLANCE_PAN_MAX_DEG",
            pose.pan_deg,
            ts.as_millis(),
        );
        // Tilt is asymmetric (Pose::clamped pins negatives to MIN_TILT_DEG = 0).
        assert!(
            pose.tilt_deg <= GLANCE_TILT_MAX_DEG + 0.01,
            "tilt {} at {}ms exceeds GLANCE_TILT_MAX_DEG",
            pose.tilt_deg,
            ts.as_millis(),
        );
    }
}

#[test]
fn head_spends_most_time_at_rest() {
    // The defining behaviour of the new event-driven pattern (vs. the
    // old continuous triangle wave) is that the head spends the
    // *majority* of its time at zero pose, glancing only briefly. We
    // expect "active" frames (non-zero pose) to be a minority of the
    // recorded trajectory.
    //
    // Math: each glance is `EASE_IN + HOLD + EASE_OUT` ≈ 2.1 s of
    // motion, separated by `[MIN, MAX]` ms intervals (~10 s avg).
    // Active fraction ≈ 2.1 / (10 + 2.1) ≈ 17 %. We assert <= 35 %
    // to leave headroom for the random-interval distribution.
    let glance_window_ms = GLANCE_EASE_IN_MS + GLANCE_HOLD_MS + GLANCE_EASE_OUT_MS;
    let avg_interval_ms = u64::midpoint(GLANCE_INTERVAL_MIN_MS, GLANCE_INTERVAL_MAX_MS);
    let expected_active_fraction =
        glance_window_ms as f32 / (glance_window_ms + avg_interval_ms) as f32;
    assert!(
        expected_active_fraction < 0.35,
        "expected_active_fraction sanity-check: {expected_active_fraction}",
    );

    let mut entity = Entity::default();
    let mut head_drift = IdleHeadDrift::new();
    let mut active_frames = 0_u32;
    let mut total_frames = 0_u32;
    let mut t_ms = 0;
    while t_ms <= DURATION_MS {
        entity.tick.now = stackchan_core::Instant::from_millis(t_ms);
        head_drift.update(&mut entity);
        if entity.motor.head_pose.pan_deg != 0.0 || entity.motor.head_pose.tilt_deg != 0.0 {
            active_frames += 1;
        }
        total_frames += 1;
        t_ms += TICK_MS;
    }

    let observed = active_frames as f32 / total_frames.max(1) as f32;
    let observed_pct = observed * 100.0;
    // Two-sided bound. The upper bound is the office-quiet contract.
    // The lower bound catches a regression that would silently
    // suppress all glances (e.g. a scheduler bug that never sets
    // `next_glance_at`); without it, this test passes vacuously at
    // `observed = 0` and we lose the "must actually fire" signal.
    // Monte Carlo across 1000 seeds: 11.6%–21.1%, mean 16.4%.
    assert!(
        (0.05..0.35).contains(&observed),
        "head was active {observed_pct:.0}% of the trajectory — expected 5%–35%",
    );
}

#[test]
fn at_least_one_glance_fires_within_max_interval_plus_window() {
    // Lower bound: within MAX_INTERVAL + glance window we must see at
    // least one non-zero pose. Otherwise the scheduler isn't actually
    // running.
    let mut entity = Entity::default();
    let mut head_drift = IdleHeadDrift::new();
    let mut saw_motion = false;
    let bound_ms =
        GLANCE_INTERVAL_MAX_MS + GLANCE_EASE_IN_MS + GLANCE_HOLD_MS + GLANCE_EASE_OUT_MS + 200;
    let mut t_ms = 0;
    while t_ms <= bound_ms {
        entity.tick.now = stackchan_core::Instant::from_millis(t_ms);
        head_drift.update(&mut entity);
        if entity.motor.head_pose.pan_deg != 0.0 || entity.motor.head_pose.tilt_deg != 0.0 {
            saw_motion = true;
            break;
        }
        t_ms += TICK_MS;
    }
    assert!(saw_motion, "no glance fired within {bound_ms} ms");
}

#[test]
fn recording_head_preserves_call_order() {
    // Contract test for RecordingHead: order of (ts, pose) matches call order.
    let mut head = RecordingHead::new();
    block_on(head.set_pose(
        stackchan_core::Pose::new(1.0, 2.0),
        stackchan_core::Instant::from_millis(10),
    ))
    .unwrap();
    block_on(head.set_pose(
        stackchan_core::Pose::new(-3.0, 4.0),
        stackchan_core::Instant::from_millis(20),
    ))
    .unwrap();
    let recs = head.records();
    assert_eq!(recs.len(), 2);
    assert_eq!(recs[0].0.as_millis(), 10);
    assert_eq!(recs[1].0.as_millis(), 20);
    assert_eq!(recs[1].1.pan_deg, -3.0);
}
