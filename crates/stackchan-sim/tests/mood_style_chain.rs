//! End-to-end sim test: a non-Neutral [`Mood`] visibly modulates
//! cadence on top of [`StyleFromEmotion`]'s per-emotion baseline.
//!
//! Drives the full Affect → Expression chain through the
//! [`Director`] for a single emotion (Happy) under each [`Mood`]
//! variant and asserts the resulting `blink_rate_scale` /
//! `breath_depth_scale` actually differ — pinning that
//! [`StyleFromMood`] integrates correctly with the rest of the
//! Director sort order.

#![allow(
    clippy::expect_used,
    reason = "test-only: registry capacity is a compile-time constant in this fixture"
)]

use stackchan_core::modifiers::{StyleFromEmotion, StyleFromMood};
use stackchan_core::{Director, Emotion, Entity, Instant, Mood};

fn settled_style_under(mood: Mood) -> (u8, u8) {
    let mut entity = Entity::default();
    entity.mind.affect.emotion = Emotion::Happy;
    entity.mind.mood = mood;
    let mut style_from_emotion = StyleFromEmotion::new();
    let mut style_from_mood = StyleFromMood::new();
    let mut director = Director::new();
    director
        .add_modifier(&mut style_from_emotion)
        .expect("registry full");
    director
        .add_modifier(&mut style_from_mood)
        .expect("registry full");
    // Two ticks past the transition window so the easing settles.
    director.run(&mut entity, Instant::from_millis(0));
    director.run(
        &mut entity,
        Instant::from_millis(StyleFromEmotion::TRANSITION_MS + 1),
    );
    (
        entity.face.style.blink_rate_scale,
        entity.face.style.breath_depth_scale,
    )
}

#[test]
fn mood_modulates_blink_rate_through_director() {
    let neutral = settled_style_under(Mood::Neutral);
    let playful = settled_style_under(Mood::Playful);
    let calm = settled_style_under(Mood::Calm);

    assert!(
        playful.0 > neutral.0,
        "Playful should blink faster than Neutral on the same Happy face ({} vs {})",
        playful.0,
        neutral.0,
    );
    assert!(
        calm.0 < neutral.0,
        "Calm should blink slower than Neutral on the same Happy face ({} vs {})",
        calm.0,
        neutral.0,
    );
}

#[test]
fn mood_modulates_breath_depth_through_director() {
    let neutral = settled_style_under(Mood::Neutral);
    let calm = settled_style_under(Mood::Calm);
    let playful = settled_style_under(Mood::Playful);

    assert!(
        calm.1 > neutral.1,
        "Calm should deepen breath above Neutral ({} vs {})",
        calm.1,
        neutral.1,
    );
    assert!(
        playful.1 < neutral.1,
        "Playful should shallow breath below Neutral ({} vs {})",
        playful.1,
        neutral.1,
    );
}

#[test]
fn surprised_blink_zero_resists_mood_amplification() {
    // `Surprised` emotion sets blink_rate_scale = 0 (eyes held
    // wide). A Playful mood multiplier mustn't quietly start the
    // eyes blinking — the zero must propagate.
    let mut entity = Entity::default();
    entity.mind.affect.emotion = Emotion::Surprised;
    entity.mind.mood = Mood::Playful;
    let mut style_from_emotion = StyleFromEmotion::new();
    let mut style_from_mood = StyleFromMood::new();
    let mut director = Director::new();
    director
        .add_modifier(&mut style_from_emotion)
        .expect("registry full");
    director
        .add_modifier(&mut style_from_mood)
        .expect("registry full");
    director.run(&mut entity, Instant::from_millis(0));
    director.run(
        &mut entity,
        Instant::from_millis(StyleFromEmotion::TRANSITION_MS + 1),
    );
    assert_eq!(entity.face.style.blink_rate_scale, 0);
}
