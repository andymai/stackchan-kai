//! End-to-end sim test for the `IntentFromBodyTouch` modifier.
//!
//! Drives a Director with `IntentFromBodyTouch` + `StyleFromEmotion` (so emotion
//! changes propagate into the face style fields), varies
//! `perception.body_touch` across simulated time, and asserts the
//! Press / Swipe / Release state machine matches the documented
//! contract.
//!
//! Routing through `Director` exercises the debug-mode `writes:`
//! enforcement on every frame, so a future modifier change that
//! silently writes a field outside its declared `writes:` slice would
//! panic here.

#![allow(
    clippy::unwrap_used,
    reason = "test-only: registry capacity is a compile-time constant in this fixture"
)]

use stackchan_core::modifiers::{
    DEFAULT_CENTRE_PRESS, DEFAULT_LEFT_PRESS, DEFAULT_RIGHT_PRESS, DEFAULT_SWIPE_BACKWARD,
    DEFAULT_SWIPE_FORWARD, HEADPET_REACTION_ATTACK_MS, HEADPET_REACTION_TOTAL_MS,
    HeadFromBodyGesture, IntentFromBodyTouch,
};
use stackchan_core::{BodyGesture, BodyTouch, Director, Emotion, Entity, Instant, OverrideSource};

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
fn press_centre_through_director_yields_happy() {
    let mut entity = Entity::default();
    let mut gesture = IntentFromBodyTouch::new();
    let mut director = Director::new();
    director.add_modifier(&mut gesture).unwrap();

    entity.perception.body_touch = Some(BodyTouch {
        centre: 3,
        ..BodyTouch::default()
    });
    director.run(&mut entity, Instant::from_millis(0));

    assert_eq!(entity.mind.affect.emotion, DEFAULT_CENTRE_PRESS);
    assert_eq!(entity.mind.autonomy.source, Some(OverrideSource::BodyTouch));
    assert!(entity.mind.autonomy.manual_until.is_some());
}

#[test]
fn left_press_then_release_then_right_press_fires_twice() {
    let mut entity = Entity::default();
    let mut gesture = IntentFromBodyTouch::new();
    let mut director = Director::new();
    director.add_modifier(&mut gesture).unwrap();

    entity.perception.body_touch = Some(BodyTouch {
        left: 3,
        ..BodyTouch::default()
    });
    director.run(&mut entity, Instant::from_millis(0));
    assert_eq!(entity.mind.affect.emotion, DEFAULT_LEFT_PRESS);

    entity.perception.body_touch = Some(BodyTouch::default());
    run_for(&mut director, &mut entity, TICK_MS, 5);

    entity.perception.body_touch = Some(BodyTouch {
        right: 3,
        ..BodyTouch::default()
    });
    director.run(&mut entity, Instant::from_millis(10_000));
    assert_eq!(entity.mind.affect.emotion, DEFAULT_RIGHT_PRESS);
}

#[test]
fn left_to_right_slide_through_director_fires_swipe_forward() {
    let mut entity = Entity::default();
    let mut gesture = IntentFromBodyTouch::new();
    let mut director = Director::new();
    director.add_modifier(&mut gesture).unwrap();

    // Press on the left.
    entity.perception.body_touch = Some(BodyTouch {
        left: 3,
        ..BodyTouch::default()
    });
    director.run(&mut entity, Instant::from_millis(0));
    assert_eq!(entity.mind.affect.emotion, DEFAULT_LEFT_PRESS);

    // Slide finger right; centroid moves well past +SWIPE_DELTA.
    entity.perception.body_touch = Some(BodyTouch {
        left: 0,
        centre: 0,
        right: 3,
    });
    director.run(&mut entity, Instant::from_millis(100));
    assert_eq!(entity.mind.affect.emotion, DEFAULT_SWIPE_FORWARD);
}

#[test]
fn right_to_left_slide_through_director_fires_swipe_backward() {
    let mut entity = Entity::default();
    let mut gesture = IntentFromBodyTouch::new();
    let mut director = Director::new();
    director.add_modifier(&mut gesture).unwrap();

    entity.perception.body_touch = Some(BodyTouch {
        right: 3,
        ..BodyTouch::default()
    });
    director.run(&mut entity, Instant::from_millis(0));
    assert_eq!(entity.mind.affect.emotion, DEFAULT_RIGHT_PRESS);

    entity.perception.body_touch = Some(BodyTouch {
        left: 3,
        ..BodyTouch::default()
    });
    director.run(&mut entity, Instant::from_millis(100));
    assert_eq!(entity.mind.affect.emotion, DEFAULT_SWIPE_BACKWARD);
}

#[test]
fn no_perception_keeps_neutral() {
    let mut entity = Entity::default();
    let mut gesture = IntentFromBodyTouch::new();
    let mut director = Director::new();
    director.add_modifier(&mut gesture).unwrap();

    // perception.body_touch defaults to None.
    run_for(&mut director, &mut entity, 0, 30);
    assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
    assert!(entity.mind.autonomy.manual_until.is_none());
}

#[test]
fn intent_from_body_touch_stamps_last_gesture_on_press_swipe_release() {
    let mut entity = Entity::default();
    let mut gesture = IntentFromBodyTouch::new();
    let mut director = Director::new();
    director.add_modifier(&mut gesture).unwrap();

    // Press fires Press(zones).
    entity.perception.body_touch = Some(BodyTouch {
        centre: 3,
        ..BodyTouch::default()
    });
    director.run(&mut entity, Instant::from_millis(0));
    assert!(matches!(
        entity.mind.last_gesture,
        Some((BodyGesture::Press { centre: 3, .. }, _))
    ));

    // Slide right → SwipeForward.
    entity.perception.body_touch = Some(BodyTouch {
        right: 3,
        ..BodyTouch::default()
    });
    director.run(&mut entity, Instant::from_millis(100));
    assert!(matches!(
        entity.mind.last_gesture,
        Some((BodyGesture::SwipeForward, _))
    ));

    // Release → Release.
    entity.perception.body_touch = Some(BodyTouch::default());
    director.run(&mut entity, Instant::from_millis(200));
    assert!(matches!(
        entity.mind.last_gesture,
        Some((BodyGesture::Release, _))
    ));
}

#[test]
fn swipe_drives_head_pet_reaction_through_director() {
    // Drive the full Affect→Motion chain: IntentFromBodyTouch sets
    // emotion + stamps last_gesture; HeadFromBodyGesture reads
    // last_gesture and applies a randomized head-pose nudge.
    let mut entity = Entity::default();
    let mut gesture = IntentFromBodyTouch::new();
    let mut head_pet = HeadFromBodyGesture::new();
    let mut director = Director::new();
    director.add_modifier(&mut gesture).unwrap();
    director.add_modifier(&mut head_pet).unwrap();

    // Press → Swipe.
    entity.perception.body_touch = Some(BodyTouch {
        left: 3,
        ..BodyTouch::default()
    });
    director.run(&mut entity, Instant::from_millis(0));
    entity.perception.body_touch = Some(BodyTouch {
        right: 3,
        ..BodyTouch::default()
    });
    director.run(&mut entity, Instant::from_millis(50));

    // Advance to the envelope peak. The pose should now reflect the
    // head-pet nudge.
    director.run(
        &mut entity,
        Instant::from_millis(50 + HEADPET_REACTION_ATTACK_MS),
    );
    assert!(
        entity.motor.head_pose.pan_deg.abs() > 0.5 || entity.motor.head_pose.tilt_deg.abs() > 0.5,
        "expected head pose to reflect the swipe-driven pet reaction at peak, got {:?}",
        entity.motor.head_pose,
    );

    // After total reaction window, the pose should return to the
    // upstream baseline (zero, since no other Motion modifier wrote).
    director.run(
        &mut entity,
        Instant::from_millis(50 + HEADPET_REACTION_TOTAL_MS + 200),
    );
    assert!(
        entity.motor.head_pose.pan_deg.abs() < 0.01 && entity.motor.head_pose.tilt_deg.abs() < 0.01,
        "expected head pose to settle back to zero after reaction window, got {:?}",
        entity.motor.head_pose,
    );
}
