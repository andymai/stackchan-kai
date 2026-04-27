//! End-to-end sim coverage for the `RemoteCommandModifier` — the
//! firmware HTTP control plane's host-side translator.
//!
//! Pinned contracts:
//!
//! - **LookAt → motor pose**: a `RemoteCommand::LookAt` with a held
//!   target, run through the `RemoteCommandModifier` + `HeadFromAttention`
//!   stack, drives `motor.head_pose` toward the requested pose.
//! - **Hold beats tracking**: while a LookAt hold is alive, fresh
//!   `TrackingObservation`s feeding `AttentionFromTracking` cannot
//!   stomp the operator's target. After the hold expires, tracking
//!   resumes ownership of attention.
//! - **Reset returns to autonomous**: after `RemoteCommand::Reset`,
//!   subsequent ticks let normal modifiers run unimpeded.

#![allow(
    clippy::doc_markdown,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test-only: doc comments freely reference type names without backticks; \
              match-with-panic is the cleanest pattern for value extraction on enum \
              variants in tests; registry capacity is a compile-time constant"
)]

use stackchan_core::modifiers::{
    AttentionFromTracking, HeadFromAttention, HeadFromEmotion, IdleHeadDrift, RemoteCommandModifier,
};
use stackchan_core::{Attention, Director, Emotion, Entity, Instant, Pose, RemoteCommand};
use stackchan_sim::TrackingScenario;

const TICK_MS: u64 = 33;

fn run_for(director: &mut Director<'_>, entity: &mut Entity, start_ms: u64, ticks: u64) -> Instant {
    let mut last = Instant::from_millis(start_ms);
    for t in 0..ticks {
        last = Instant::from_millis(start_ms + t * TICK_MS);
        director.run(entity, last);
    }
    last
}

#[test]
fn lookat_drives_motor_head_pose_through_head_from_attention() {
    let target = Pose::new(20.0, -5.0);
    let mut entity = Entity::default();
    let mut head_drift = IdleHeadDrift::new();
    let mut emo = HeadFromEmotion::new();
    let mut head_from_attention = HeadFromAttention::new();
    let mut remote = RemoteCommandModifier::new();
    let mut director = Director::new();
    director.add_modifier(&mut head_drift).unwrap();
    director.add_modifier(&mut emo).unwrap();
    director.add_modifier(&mut head_from_attention).unwrap();
    director.add_modifier(&mut remote).unwrap();

    entity.input.remote_command = Some(RemoteCommand::LookAt {
        target,
        hold_ms: 2_000,
    });

    // Drive long enough for HeadFromAttention's ease window to ramp
    // toward the target.
    run_for(&mut director, &mut entity, 0, 60);

    let pan = entity.motor.head_pose.pan_deg;
    assert!(
        pan > 5.0,
        "expected head pan to track toward target (20°), got {pan}"
    );
    assert!(
        matches!(entity.mind.attention, Attention::Tracking { target: t, .. } if t == target),
        "expected attention to hold operator's target, got {:?}",
        entity.mind.attention
    );
}

#[test]
fn lookat_hold_beats_tracking_observation() {
    // Operator-supplied target wins against fresh tracking input
    // while the hold is active. After release, tracking takes over.
    let operator_target = Pose::new(25.0, 0.0);
    let mut entity = Entity::default();
    let mut afm = AttentionFromTracking::new();
    let mut remote = RemoteCommandModifier::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut remote).unwrap();

    entity.input.remote_command = Some(RemoteCommand::LookAt {
        target: operator_target,
        hold_ms: 1_000,
    });

    // While the hold is alive, feed AttentionFromTracking a different
    // target. RemoteCommandModifier runs after AttentionFromTracking
    // in Phase::Cognition (priority 100 > 0) and re-asserts.
    let tracker_target = Pose::new(-30.0, 8.0);
    let mut last = Instant::ZERO;
    let scenario = TrackingScenario::new(TICK_MS).tracking(tracker_target, 20 * TICK_MS);
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
        last = now;
    }

    match entity.mind.attention {
        Attention::Tracking { target, .. } => {
            assert_eq!(
                target, operator_target,
                "operator target must hold against tracking observations"
            );
        }
        other => panic!("expected Tracking, got {other:?}"),
    }

    // Step past the hold. Now AttentionFromTracking should be free to
    // write its own target on the next observation that meets its lock
    // criteria.
    let after_hold = last.as_millis() + 2_000;
    entity.perception.tracking = None;
    run_for(&mut director, &mut entity, after_hold, 60);

    // After release, attention is no longer pinned to operator_target.
    // (Either None, or AttentionFromTracking re-locked on the silent
    // observation — both prove our hold stopped re-asserting.)
    if let Attention::Tracking { target, .. } = entity.mind.attention {
        assert_ne!(
            target, operator_target,
            "after release, operator target should not be re-asserted"
        );
    }
}

#[test]
fn reset_returns_attention_to_default() {
    let mut entity = Entity::default();
    let mut remote = RemoteCommandModifier::new();
    let mut director = Director::new();
    director.add_modifier(&mut remote).unwrap();

    entity.input.remote_command = Some(RemoteCommand::LookAt {
        target: Pose::new(15.0, 0.0),
        hold_ms: 10_000,
    });
    run_for(&mut director, &mut entity, 0, 5);
    assert!(matches!(entity.mind.attention, Attention::Tracking { .. }));

    entity.input.remote_command = Some(RemoteCommand::Reset);
    run_for(&mut director, &mut entity, 1_000, 2);
    assert_eq!(entity.mind.attention, Attention::None);
    assert!(entity.mind.autonomy.manual_until.is_none());
}

#[test]
fn set_emotion_holds_against_autonomous_overwrite() {
    // The hold timer keeps emotion + autonomy pinned even if the test
    // poke-mutates emotion mid-hold (proxy for an autonomous emotion
    // driver writing during the same frame stack).
    let mut entity = Entity::default();
    let mut remote = RemoteCommandModifier::new();
    let mut director = Director::new();
    director.add_modifier(&mut remote).unwrap();

    entity.input.remote_command = Some(RemoteCommand::SetEmotion {
        emotion: Emotion::Happy,
        hold_ms: 1_000,
    });
    director.run(&mut entity, Instant::from_millis(0));

    // Autonomous driver picks Sleepy.
    entity.mind.affect.emotion = Emotion::Sleepy;
    director.run(&mut entity, Instant::from_millis(100));

    assert_eq!(
        entity.mind.affect.emotion,
        Emotion::Happy,
        "hold must re-assert emotion across ticks while hold is active"
    );
}
