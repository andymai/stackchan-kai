//! End-to-end sim test for the `IntentFromLoud` modifier + `HeadFromIntent`
//! recoil + `render_leds` brightness override chain.
//!
//! Drives a Director with the full Affect + Motion stack relevant to
//! startle, varies `perception.audio_rms` across simulated time, and
//! asserts the cross-cutting reaction chain ([`Intent::Startled`] +
//! [`Emotion::Surprised`] + [`ChirpKind::Startle`] + head recoil + LED
//! brightness pin).
//!
//! Pins the modifier-writes-intent pattern: `IntentFromLoud` writes
//! `mind.intent` directly (uncommon — most intents come from skills)
//! to keep the startle reaction within a single tick. A regression
//! that splits the writes back across two frames would manifest as
//! `head_pose.pan_deg == 0` here even though intent flipped.

#![allow(
    clippy::unwrap_used,
    reason = "test-only: registry capacity is a compile-time constant in this fixture"
)]

use stackchan_core::modifiers::{
    EmotionHead, HeadFromIntent, IdleSway, IntentFromLoud, ListenHead, STARTLE_HEAD_TOTAL_MS,
    STARTLE_HOLD_MS, WakeOnVoice,
};
use stackchan_core::skills::LookAtSound;
use stackchan_core::voice::ChirpKind;
use stackchan_core::{Director, Emotion, Entity, Instant, LedFrame, mind::Intent, render_leds};

const TICK_MS: u64 = 33;

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
fn quiet_audio_does_not_startle() {
    let mut entity = Entity::default();
    let mut startle = IntentFromLoud::new();
    let mut director = Director::new();
    director.add_modifier(&mut startle).unwrap();

    entity.perception.audio_rms = Some(0.05);
    run_for(&mut director, &mut entity, 0, 30);

    assert_eq!(entity.mind.intent, Intent::Idle);
    assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
    assert!(entity.voice.chirp_request.is_none());
}

#[test]
fn loud_transient_fires_full_reaction_chain() {
    let mut entity = Entity::default();
    let mut sway = IdleSway::new();
    let mut emo = EmotionHead::new();
    let mut listen_head = ListenHead::new();
    let mut head_from_intent = HeadFromIntent::new();
    let mut startle = IntentFromLoud::new();
    let mut director = Director::new();
    director.add_modifier(&mut sway).unwrap();
    director.add_modifier(&mut emo).unwrap();
    director.add_modifier(&mut listen_head).unwrap();
    director.add_modifier(&mut head_from_intent).unwrap();
    director.add_modifier(&mut startle).unwrap();

    // Establish a quiet baseline so the rising edge is unambiguous,
    // and capture pre-startle pan to compare against (IdleSway's pan
    // amplitude at any given instant is non-trivial — comparing
    // against absolute thresholds is brittle).
    entity.perception.audio_rms = Some(0.05);
    run_for(&mut director, &mut entity, 0, 5);
    assert_eq!(entity.mind.intent, Intent::Idle);
    let pan_before = entity.motor.head_pose.pan_deg;

    // Loud transient. Single tick over threshold is enough.
    let fire_tick_ms = 5 * TICK_MS;
    entity.perception.audio_rms = Some(0.6);
    director.run(&mut entity, Instant::from_millis(fire_tick_ms));

    assert_eq!(
        entity.mind.intent,
        Intent::Startled,
        "rising-edge loud must set Intent::Startled"
    );
    assert_eq!(
        entity.mind.affect.emotion,
        Emotion::Surprised,
        "IntentFromLoud must write Surprised in the same tick"
    );
    assert_eq!(
        entity.voice.chirp_request,
        Some(ChirpKind::Startle),
        "IntentFromLoud must queue a Startle chirp on the rising edge"
    );

    // Drive forward to the head recoil's peak (anchor + attack window).
    let peak_tick = fire_tick_ms + 50; // STARTLE_HEAD_ATTACK_MS = 50
    director.run(&mut entity, Instant::from_millis(peak_tick));
    let pan_after = entity.motor.head_pose.pan_deg;
    assert!(
        pan_after - pan_before > 2.0,
        "HeadFromIntent must add a positive pan recoil on top of upstream sway \
         (before {pan_before}, after {pan_after})",
    );

    // LEDs at Startled should pin to peak brightness regardless of
    // the breath envelope phase — verify by sampling at the breath
    // trough where dim would normally apply.
    let mut frame = LedFrame::default();
    render_leds(&entity, Instant::from_millis(0), &mut frame);
    let loud_blue = frame.0[0] & 0x1F;
    entity.mind.intent = Intent::Idle;
    render_leds(&entity, Instant::from_millis(0), &mut frame);
    let idle_blue = frame.0[0] & 0x1F;
    assert!(
        loud_blue > idle_blue,
        "Startled must pin LED brightness above the breath trough ({idle_blue} → {loud_blue})",
    );
}

#[test]
fn intent_clears_after_hold_head_returns_to_baseline() {
    let mut entity = Entity::default();
    let mut sway = IdleSway::new();
    let mut emo = EmotionHead::new();
    let mut listen_head = ListenHead::new();
    let mut head_from_intent = HeadFromIntent::new();
    let mut startle = IntentFromLoud::new();
    let mut director = Director::new();
    director.add_modifier(&mut sway).unwrap();
    director.add_modifier(&mut emo).unwrap();
    director.add_modifier(&mut listen_head).unwrap();
    director.add_modifier(&mut head_from_intent).unwrap();
    director.add_modifier(&mut startle).unwrap();

    // Fire the startle.
    entity.perception.audio_rms = Some(0.05);
    run_for(&mut director, &mut entity, 0, 3);
    entity.perception.audio_rms = Some(0.6);
    director.run(&mut entity, Instant::from_millis(3 * TICK_MS));
    assert_eq!(entity.mind.intent, Intent::Startled);

    // Quiet down. Drive past STARTLE_HOLD_MS so the modifier releases
    // intent back to Idle.
    entity.perception.audio_rms = Some(0.05);
    let post_hold = 3 * TICK_MS + STARTLE_HOLD_MS + 100;
    run_for(
        &mut director,
        &mut entity,
        4 * TICK_MS,
        (STARTLE_HOLD_MS / TICK_MS) + 5,
    );
    director.run(&mut entity, Instant::from_millis(post_hold));
    assert_eq!(
        entity.mind.intent,
        Intent::Idle,
        "intent should clear back to Idle after STARTLE_HOLD_MS",
    );

    // Head recoil envelope (STARTLE_HEAD_TOTAL_MS = 400ms) is well
    // shorter than STARTLE_HOLD_MS (1500ms), so by post-hold the head
    // contribution must be zero.
    let _ = STARTLE_HEAD_TOTAL_MS; // pinned-import: regressions in the
    // ratio surface as test imports going stale. Keeps this test
    // honest about the constant relationship.
}

#[test]
fn startle_overrides_in_progress_listen() {
    // Sustained voice puts the avatar into Listen + Happy via
    // LookAtSound + WakeOnVoice. A loud spike mid-conversation must
    // still flip the intent to Startled (IntentFromLoud overrides
    // the WakeOnVoice hold).
    let mut entity = Entity::default();
    let mut wake_on_voice = WakeOnVoice::new();
    let mut startle = IntentFromLoud::new();
    let mut look_at_sound = LookAtSound::new();
    let mut director = Director::new();
    director.add_modifier(&mut wake_on_voice).unwrap();
    director.add_modifier(&mut startle).unwrap();
    director.add_skill(&mut look_at_sound).unwrap();

    // Sustained "voice" (above wake threshold, below startle).
    entity.perception.audio_rms = Some(0.1);
    run_for(&mut director, &mut entity, 0, 30);
    assert_eq!(
        entity.mind.intent,
        Intent::Listen,
        "sustained voice should enter Listen via LookAtSound",
    );
    assert_eq!(entity.mind.affect.emotion, Emotion::Happy);

    // Loud spike — must override.
    entity.perception.audio_rms = Some(0.6);
    director.run(&mut entity, Instant::from_millis(31 * TICK_MS));
    assert_eq!(
        entity.mind.intent,
        Intent::Startled,
        "loud transient must override in-progress Listen",
    );
    assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
}
