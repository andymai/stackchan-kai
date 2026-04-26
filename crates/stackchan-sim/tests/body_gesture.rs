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
    DEFAULT_SWIPE_FORWARD, IntentFromBodyTouch,
};
use stackchan_core::{BodyTouch, Director, Emotion, Entity, Instant, OverrideSource};

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
