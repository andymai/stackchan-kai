//! End-to-end sim test for the `Petting` skill.
//!
//! Verifies the parallel-driver design: `IntentFromBodyTouch` (modifier) and
//! `Petting` (skill) react to the same `perception.body_touch` input
//! without conflict — modifier writes emotion + autonomy on the
//! Press edge, skill writes intent after sustained contact.

#![allow(
    clippy::unwrap_used,
    reason = "test-only: registry capacity is a compile-time constant in this fixture"
)]

use stackchan_core::modifiers::{DEFAULT_CENTRE_PRESS, IntentFromBodyTouch};
use stackchan_core::skills::{PETTING_SUSTAIN_TICKS, Petting};
use stackchan_core::{BodyTouch, Director, Entity, Instant, Intent};

const TICK_MS: u64 = 50;

fn run_for(director: &mut Director<'_>, entity: &mut Entity, ticks: u64) {
    let mut now_ms = entity.tick.now.as_millis();
    for _ in 0..ticks {
        now_ms += TICK_MS;
        director.run(entity, Instant::from_millis(now_ms));
    }
}

#[test]
fn brief_touch_through_director_keeps_intent_idle() {
    let mut entity = Entity::default();
    let mut petting = Petting::new();
    let mut director = Director::new();
    director.add_skill(&mut petting).unwrap();

    entity.perception.body_touch = Some(BodyTouch {
        centre: 3,
        ..BodyTouch::default()
    });
    run_for(
        &mut director,
        &mut entity,
        u64::from(PETTING_SUSTAIN_TICKS) - 1,
    );
    assert_eq!(entity.mind.intent, Intent::Idle);
}

#[test]
fn sustained_touch_through_director_fires_being_pet() {
    let mut entity = Entity::default();
    let mut petting = Petting::new();
    let mut director = Director::new();
    director.add_skill(&mut petting).unwrap();

    entity.perception.body_touch = Some(BodyTouch {
        centre: 3,
        ..BodyTouch::default()
    });
    run_for(&mut director, &mut entity, u64::from(PETTING_SUSTAIN_TICKS));
    assert_eq!(entity.mind.intent, Intent::Petted);
}

#[test]
fn body_gesture_and_petting_coexist_on_same_input() {
    let mut entity = Entity::default();
    let mut gesture = IntentFromBodyTouch::new();
    let mut petting = Petting::new();
    let mut director = Director::new();
    director.add_modifier(&mut gesture).unwrap();
    director.add_skill(&mut petting).unwrap();

    // First frame: Press → IntentFromBodyTouch sets emotion.
    entity.perception.body_touch = Some(BodyTouch {
        centre: 3,
        ..BodyTouch::default()
    });
    director.run(&mut entity, Instant::from_millis(0));
    assert_eq!(entity.mind.affect.emotion, DEFAULT_CENTRE_PRESS);
    assert_eq!(entity.mind.intent, Intent::Idle);

    // Continue touching past the sustain — Petting fires intent
    // change while emotion stays pinned.
    run_for(&mut director, &mut entity, u64::from(PETTING_SUSTAIN_TICKS));
    assert_eq!(entity.mind.affect.emotion, DEFAULT_CENTRE_PRESS);
    assert_eq!(entity.mind.intent, Intent::Petted);
}

#[test]
fn release_clears_being_pet_through_director() {
    let mut entity = Entity::default();
    let mut petting = Petting::new();
    let mut director = Director::new();
    director.add_skill(&mut petting).unwrap();

    entity.perception.body_touch = Some(BodyTouch {
        centre: 3,
        ..BodyTouch::default()
    });
    run_for(&mut director, &mut entity, u64::from(PETTING_SUSTAIN_TICKS));
    assert_eq!(entity.mind.intent, Intent::Petted);

    entity.perception.body_touch = Some(BodyTouch::default());
    let next = entity.tick.now.as_millis() + TICK_MS;
    director.run(&mut entity, Instant::from_millis(next));
    assert_eq!(entity.mind.intent, Intent::Idle);
}
