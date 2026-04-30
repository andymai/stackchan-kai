//! End-to-end coverage from BLE wire payload to entity behaviour.
//!
//! Asserts that decoding a raw characteristic payload via
//! [`stackchan_net::ble_command`] and feeding the resulting
//! [`RemoteCommand`] into the same modifier stack the firmware runs
//! produces the entity transitions an operator would expect — the
//! tests are how we guard against silent drift between the BLE wire
//! contract and the host-side semantics.

#![allow(
    clippy::doc_markdown,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test-only: doc comments freely reference type names without backticks; \
              match-with-panic is the cleanest pattern for value extraction on enum \
              variants in tests"
)]

use stackchan_core::modifiers::{
    HeadFromAttention, HeadFromEmotion, IdleHeadDrift, RemoteCommandModifier,
};
use stackchan_core::{Attention, Director, Emotion, Entity, Instant, RemoteCommand};
use stackchan_net::ble_command::{
    self, EMOTION_HAPPY, EMOTION_WRITE_LEN, LOCALE_EN, LOOK_AT_LEN, PHRASE_GREETING, SPEAK_LEN,
};

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
fn ble_emotion_payload_holds_emotion_via_director() {
    // Build the payload as the central would: u8 emotion + u16 LE
    // hold_ms = 1500 ms.
    let mut payload = [0u8; EMOTION_WRITE_LEN];
    payload[0] = EMOTION_HAPPY;
    payload[1..3].copy_from_slice(&1_500u16.to_le_bytes());

    let cmd = ble_command::decode_emotion_write(&payload).unwrap();
    match cmd {
        RemoteCommand::SetEmotion { emotion, hold_ms } => {
            assert_eq!(emotion, Emotion::Happy);
            assert_eq!(hold_ms, 1_500);
        }
        other => panic!("expected SetEmotion, got {other:?}"),
    }

    let mut entity = Entity::default();
    let mut remote = RemoteCommandModifier::new();
    let mut director = Director::new();
    director.add_modifier(&mut remote).unwrap();

    entity.input.remote_command = Some(cmd);
    director.run(&mut entity, Instant::from_millis(0));

    // Even if an autonomous driver tries to flip emotion mid-hold, the
    // remote command's hold pins the BLE-requested value.
    entity.mind.affect.emotion = Emotion::Sleepy;
    director.run(&mut entity, Instant::from_millis(100));
    assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
}

#[test]
fn ble_look_at_payload_drives_motor_pose_via_head_from_attention() {
    // Wire payload: pan = 1500 centi-deg (15.0°), tilt = -300
    // centi-deg (-3.0°), hold = 2000 ms.
    let mut payload = [0u8; LOOK_AT_LEN];
    payload[0..2].copy_from_slice(&1_500i16.to_le_bytes());
    payload[2..4].copy_from_slice(&(-300i16).to_le_bytes());
    payload[4..6].copy_from_slice(&2_000u16.to_le_bytes());

    let cmd = ble_command::decode_look_at(&payload).unwrap();
    let target = match cmd {
        RemoteCommand::LookAt { target, hold_ms } => {
            assert!((target.pan_deg - 15.0).abs() < f32::EPSILON);
            assert!((target.tilt_deg + 3.0).abs() < f32::EPSILON);
            assert_eq!(hold_ms, 2_000);
            target
        }
        other => panic!("expected LookAt, got {other:?}"),
    };

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

    entity.input.remote_command = Some(cmd);
    run_for(&mut director, &mut entity, 0, 60);

    let pan = entity.motor.head_pose.pan_deg;
    assert!(
        pan > 5.0,
        "expected head pan to track toward BLE-requested target ({}°), got {pan}",
        target.pan_deg
    );
    assert!(
        matches!(entity.mind.attention, Attention::Tracking { target: t, .. } if t == target),
        "expected attention to lock onto BLE-requested target, got {:?}",
        entity.mind.attention
    );
}

#[test]
fn ble_speak_payload_routes_to_remote_command_speak() {
    let payload = [PHRASE_GREETING, LOCALE_EN];
    assert_eq!(payload.len(), SPEAK_LEN);
    let cmd = ble_command::decode_speak(&payload).unwrap();
    match cmd {
        RemoteCommand::Speak { phrase, locale, .. } => {
            assert_eq!(phrase, stackchan_core::voice::PhraseId::Greeting);
            assert_eq!(locale, stackchan_core::voice::Locale::En);
        }
        other => panic!("expected Speak, got {other:?}"),
    }
}

#[test]
fn ble_reset_payload_releases_attention_hold() {
    let mut entity = Entity::default();
    let mut remote = RemoteCommandModifier::new();
    let mut director = Director::new();
    director.add_modifier(&mut remote).unwrap();

    // Pin attention via a BLE look-at, then release via a BLE reset.
    let mut look = [0u8; LOOK_AT_LEN];
    look[0..2].copy_from_slice(&500i16.to_le_bytes());
    let look_cmd = ble_command::decode_look_at(&look).unwrap();
    entity.input.remote_command = Some(look_cmd);
    run_for(&mut director, &mut entity, 0, 5);
    assert!(matches!(entity.mind.attention, Attention::Tracking { .. }));

    // Reset is any 1-byte trigger.
    ble_command::decode_reset(&[0]).unwrap();
    entity.input.remote_command = Some(RemoteCommand::Reset);
    run_for(&mut director, &mut entity, 1_000, 2);
    assert_eq!(entity.mind.attention, Attention::None);
}
