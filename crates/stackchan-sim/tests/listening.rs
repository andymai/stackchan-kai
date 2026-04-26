//! End-to-end sim test for the `Listening` skill + `HeadFromAttention`
//! motion modifier handoff.
//!
//! Drives a Director with the full Motion-phase modifier stack
//! (`IdleHeadDrift` + `HeadFromEmotion` + `HeadFromAttention`) plus the `Listening`
//! skill, varies `perception.audio_rms` across simulated time, and
//! asserts:
//!
//! - Silence keeps `mind.attention == None` and head tilt at the
//!   head-drift+emotion baseline.
//! - Sustained loud audio causes `mind.attention` to enter
//!   `Listening` and adds an upward tilt bias.
//! - Quieting past `LISTEN_RELEASE_MS` returns attention to None and
//!   tilt to baseline.
//!
//! This is the host-side mirror of the firmware loop: it pins the
//! Skill→Modifier handoff (the architectural point of the v0.10 split)
//! against accidental regressions in either layer.

#![allow(
    clippy::unwrap_used,
    reason = "test-only: registry capacity is a compile-time constant in this \
              fixture, the unwraps can't fire"
)]

use stackchan_core::modifiers::GLANCE_TILT_MAX_DEG;
use stackchan_core::modifiers::{HeadFromAttention, HeadFromEmotion, IdleHeadDrift};
use stackchan_core::skills::{LISTEN_RELEASE_MS, LISTEN_SUSTAIN_TICKS, Listening};
use stackchan_core::{Attention, Director, Entity, Instant};

const TICK_MS: u64 = 33;

/// Baseline tilt ceiling used by silence + post-release assertions:
/// `IdleHeadDrift`'s peak per-glance tilt (`GLANCE_TILT_MAX_DEG`) +
/// `HeadFromEmotion` Neutral bias (0°) + a small margin. Tight
/// enough that an 8° `HeadFromAttention` leak would blow past it.
/// Pose clamps the lower bound at 0 so we only upper-bound here.
const BASELINE_TILT_CEILING_DEG: f32 = GLANCE_TILT_MAX_DEG + 1.0;

/// Drive the director for `ticks` frames at `TICK_MS` cadence,
/// starting at `start_ms`. Returns the final tick's `now`.
fn run_for(director: &mut Director<'_>, entity: &mut Entity, start_ms: u64, ticks: u64) -> Instant {
    let mut last = Instant::from_millis(start_ms);
    for t in 0..ticks {
        last = Instant::from_millis(start_ms + t * TICK_MS);
        director.run(entity, last);
    }
    last
}

#[test]
fn silence_holds_attention_none_and_baseline_tilt() {
    let mut entity = Entity::default();
    let mut head_drift = IdleHeadDrift::new();
    let mut emo = HeadFromEmotion::new();
    let mut head_from_attention = HeadFromAttention::new();
    let mut listening = Listening::new();
    let mut director = Director::new();
    director.add_modifier(&mut head_drift).unwrap();
    director.add_modifier(&mut emo).unwrap();
    director.add_modifier(&mut head_from_attention).unwrap();
    director.add_skill(&mut listening).unwrap();

    // Default perception.audio_rms = None ⇒ silence.
    run_for(&mut director, &mut entity, 0, 60);

    assert_eq!(entity.mind.attention, Attention::None);
    assert!(
        entity.motor.head_pose.tilt_deg < BASELINE_TILT_CEILING_DEG,
        "silence should leave tilt at baseline (<{BASELINE_TILT_CEILING_DEG}°), got {}",
        entity.motor.head_pose.tilt_deg
    );
}

#[test]
fn sustained_loud_enters_listening_and_lifts_tilt() {
    let mut entity = Entity::default();
    let mut head_drift = IdleHeadDrift::new();
    let mut emo = HeadFromEmotion::new();
    let mut head_from_attention = HeadFromAttention::new();
    let mut listening = Listening::new();
    let mut director = Director::new();
    director.add_modifier(&mut head_drift).unwrap();
    director.add_modifier(&mut emo).unwrap();
    director.add_modifier(&mut head_from_attention).unwrap();
    director.add_skill(&mut listening).unwrap();

    // Sample baseline tilt across a couple of seconds of silence so
    // we know the max head-drift+emotion contribution at rest.
    let mut baseline_max = 0.0_f32;
    for t in 0..60 {
        director.run(&mut entity, Instant::from_millis(t * TICK_MS));
        baseline_max = baseline_max.max(entity.motor.head_pose.tilt_deg);
    }
    assert_eq!(entity.mind.attention, Attention::None);

    // Drive long enough for both the skill's sustain count to fire
    // AND HeadFromAttention's ease window to fully ramp up.
    entity.perception.audio_rms = Some(0.3);
    run_for(
        &mut director,
        &mut entity,
        60 * TICK_MS,
        u64::from(LISTEN_SUSTAIN_TICKS) + 30,
    );

    assert!(
        matches!(entity.mind.attention, Attention::Listening { .. }),
        "expected Listening after sustained audio, got {:?}",
        entity.mind.attention
    );
    // HeadFromAttention adds 8° on top of baseline. Even with the head-drift glance at its
    // valley, listening tilt must clear baseline + a few degrees.
    let listen_tilt = entity.motor.head_pose.tilt_deg;
    assert!(
        listen_tilt > baseline_max + 3.0,
        "listening tilt {listen_tilt} did not lift meaningfully above baseline max {baseline_max}",
    );
}

#[test]
fn quieting_past_release_window_returns_to_baseline() {
    let mut entity = Entity::default();
    let mut head_drift = IdleHeadDrift::new();
    let mut emo = HeadFromEmotion::new();
    let mut head_from_attention = HeadFromAttention::new();
    let mut listening = Listening::new();
    let mut director = Director::new();
    director.add_modifier(&mut head_drift).unwrap();
    director.add_modifier(&mut emo).unwrap();
    director.add_modifier(&mut head_from_attention).unwrap();
    director.add_skill(&mut listening).unwrap();

    entity.perception.audio_rms = Some(0.3);
    let now = run_for(
        &mut director,
        &mut entity,
        0,
        u64::from(LISTEN_SUSTAIN_TICKS) + 30,
    );
    assert!(matches!(entity.mind.attention, Attention::Listening { .. }));

    // Step well past both the skill's release window AND HeadFromAttention's
    // ease-out window so the bias has fully decayed back to 0.
    entity.perception.audio_rms = Some(0.001);
    run_for(
        &mut director,
        &mut entity,
        now.as_millis() + TICK_MS,
        (LISTEN_RELEASE_MS / TICK_MS) + 30,
    );

    assert_eq!(
        entity.mind.attention,
        Attention::None,
        "expected attention cleared after release window"
    );
    assert!(
        entity.motor.head_pose.tilt_deg < BASELINE_TILT_CEILING_DEG,
        "tilt did not return to baseline (<{BASELINE_TILT_CEILING_DEG}°) after release, got {}",
        entity.motor.head_pose.tilt_deg
    );
}
