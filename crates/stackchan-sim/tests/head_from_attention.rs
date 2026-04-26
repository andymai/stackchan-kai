//! Sim coverage for `HeadFromAttention` composed with the full
//! Motion-phase head-pose stack via the Director, captured through
//! `RecordingHead` for firmware-port verification.
//!
//! The in-module unit tests exercise the modifier in isolation. The
//! sim tests here pin the *firmware-shaped* contracts that only show
//! up when:
//!
//! - The full Motion stack runs (`IdleSway` priority 0, `HeadFromEmotion`
//!   priority 10, `HeadFromAttention` priority 20) — proving the
//!   diff-and-undo bookkeeping composes with the two earlier
//!   contributors.
//! - The pose flows through `RecordingHead` (the same async
//!   `HeadDriver` shape `SCServo` implements on the firmware), so a
//!   future regression that violates the `set_pose` contract surfaces
//!   here as well as on hardware.
//! - Realistic 30 FPS time sequences are driven via `FakeClock` —
//!   long enough that the smoother has time to converge and the
//!   asymmetric clamp (`MIN_TILT_DEG = 0`) gets stress-tested.

#![allow(
    clippy::float_cmp,
    reason = "tests compare bit-exact pass-through values through RecordingHead, \
              not results of accumulated FP arithmetic"
)]
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
    AttentionFromTracking, HeadFromAttention, HeadFromEmotion, IDLE_SWAY_PAN_AMPLITUDE_DEG,
    IdleSway, TRACKING_LOCK_TICKS,
};
use stackchan_core::{
    Clock, Director, Entity, HeadDriver, MAX_PAN_DEG, MAX_TILT_DEG, MIN_TILT_DEG, Pose,
};
use stackchan_sim::{FakeClock, RecordingHead, TrackingScenario, block_on};

#[test]
fn full_motion_stack_pose_stays_within_clamps_through_recording_head() {
    // Run a 30 s tracking burst with the full Motion stack
    // (IdleSway + HeadFromEmotion + HeadFromAttention) plus
    // AttentionFromTracking driving attention. Capture every pose
    // through RecordingHead and assert the trajectory NEVER escapes
    // the asymmetric clamps. The `Pose::clamped` invariant must hold
    // for every one of the ~900 captured frames; even a one-frame
    // violation indicates the diff-and-undo bookkeeping leaked an
    // out-of-range pose past the modifier composition.
    let mut sway = IdleSway::new();
    let mut emo = HeadFromEmotion::new();
    let mut head = HeadFromAttention::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut sway).unwrap();
    director.add_modifier(&mut emo).unwrap();
    director.add_modifier(&mut head).unwrap();

    let mut entity = Entity::default();
    let mut recorder = RecordingHead::new();
    let clock = FakeClock::new();

    // Push the target into one of the worst regions for clamping:
    // tilt = -50° would drive the smoother below MIN_TILT_DEG (0)
    // unless the per-tick clamp catches it.
    let target = Pose::new(35.0, -20.0);
    let scenario = TrackingScenario::new(33).tracking(target, 30_000);

    for (now, obs) in scenario.iter() {
        clock.set(now);
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
        block_on(recorder.set_pose(entity.motor.head_pose, clock.now()))
            .expect("RecordingHead is infallible");
    }

    let records = recorder.records();
    assert!(
        records.len() > 800,
        "expected ~909 records over 30 s at 30 FPS, got {}",
        records.len(),
    );

    for (ts, pose) in records {
        assert!(
            pose.pan_deg.abs() <= MAX_PAN_DEG + 0.01,
            "pan {} out of range at {}ms",
            pose.pan_deg,
            ts.as_millis(),
        );
        assert!(
            (MIN_TILT_DEG - 0.01..=MAX_TILT_DEG + 0.01).contains(&pose.tilt_deg),
            "tilt {} out of range at {}ms",
            pose.tilt_deg,
            ts.as_millis(),
        );
    }
}

#[test]
fn smoother_converges_toward_target_across_realistic_burst() {
    // After enough Tracking ticks at 30 FPS, the head's pan should
    // be within ~0.5° of the target's pan (modulo IdleSway's ±2.5°
    // baseline). 30 ticks ≈ 1 s — well past the smoother's
    // ~4-frame time constant.
    let mut sway = IdleSway::new();
    let mut head = HeadFromAttention::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut sway).unwrap();
    director.add_modifier(&mut head).unwrap();

    let mut entity = Entity::default();

    // Modest target that's well inside the clamps for both pan and
    // tilt, so we can bound convergence tightly without the clamp
    // doing the test's job for it.
    let target = Pose::new(15.0, 8.0);
    let scenario = TrackingScenario::new(33).tracking(target, 2_000);
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
    }

    // Convergence: pan must be within ~3° of target (sway + smoother
    // residual). Tighter would catch single-pole low-pass alpha
    // changes, but sway dominates the residual at 1 s.
    let pan_err = (entity.motor.head_pose.pan_deg - target.pan_deg).abs();
    assert!(
        pan_err < 3.0,
        "pan should converge to within 3° of target after 2 s, got error {pan_err}",
    );
}

#[test]
fn engagement_lock_drives_head_through_full_pipeline() {
    // Drive a Tracking observation that ALSO carries face_present +
    // face_centroid, so AttentionFromTracking sets engagement →
    // HeadFromAttention reads engagement.centroid → head pans toward
    // the FACE direction (which is opposite to the motion target's
    // direction in this scenario).
    let mut sway = IdleSway::new();
    let mut head = HeadFromAttention::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut sway).unwrap();
    director.add_modifier(&mut head).unwrap();

    let mut entity = Entity::default();

    // Motion target says "look right". Face centroid says "the face
    // is on the LEFT" (-0.6 of HALF_FOV_H_DEG). The head must follow
    // the face, not the motion blob — that's the engagement-driven
    // behavior promised by the `engaged_face_centroid_overrides_motion_target`
    // unit test, but exercised here via the FULL pipeline:
    // AttentionFromTracking writes the engagement state from
    // face_centroid; HeadFromAttention reads it and steers.
    let target = Pose::new(20.0, 0.0); // motion says right
    let face_centroid = (-0.6_f32, 0.0_f32); // face says left
    let scenario = TrackingScenario::new(33)
        .tracking(target, 2_000)
        .with_face(face_centroid);
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
    }

    assert!(
        entity.motor.head_pose.pan_deg < 0.0,
        "head should pan LEFT toward the face, not RIGHT toward the motion blob; got pan={}",
        entity.motor.head_pose.pan_deg,
    );
}

#[test]
fn pose_returns_to_baseline_after_release() {
    // After a tracking burst ends and attention releases, the head
    // pose should converge back to a baseline dominated by IdleSway
    // — the HeadFromAttention contribution must unwind via diff-
    // and-undo. Bound: pan within IDLE_SWAY_PAN_AMPLITUDE_DEG plus
    // 1° slack.
    let mut sway = IdleSway::new();
    let mut head = HeadFromAttention::new();
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut sway).unwrap();
    director.add_modifier(&mut head).unwrap();

    let mut entity = Entity::default();

    // Burst, then long Returning past the release window.
    let target = Pose::new(25.0, 12.0);
    let scenario = TrackingScenario::new(33)
        .tracking(target, u64::from(TRACKING_LOCK_TICKS) * 33 + 1_000)
        .returning(3_000);
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
    }

    let bound = IDLE_SWAY_PAN_AMPLITUDE_DEG + 1.0; // sway amplitude + 1° slack
    assert!(
        entity.motor.head_pose.pan_deg.abs() <= bound,
        "post-release pan {} should be within ±{bound}° (idle sway baseline)",
        entity.motor.head_pose.pan_deg,
    );
}
