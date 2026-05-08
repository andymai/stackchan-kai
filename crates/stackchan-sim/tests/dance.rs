//! End-to-end sim test for the dance pipeline.
//!
//! Drives the full firmware-style flow on the host:
//!
//! 1. Build a `DanceScript` (via the `stackchan_net::dance` parser
//!    on a JSON body, mimicking what `POST /dance` accepts).
//! 2. Hand the script to the modifier graph by writing
//!    `entity.input.dance_script`.
//! 3. Run the `Director` with `DancePlayer` registered, advancing
//!    the clock through the script.
//! 4. Assert pose / emotion / decorator / `led_override` at the
//!    expected timestamps.
//!
//! Routing through the parser exercises the JSON wire format too, so
//! a future schema or parser change that doesn't round-trip would
//! fail here before shipping.

#![allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    reason = "test-only: registry capacity + parser inputs are compile-time fixtures; \
              float assertions compare bit-exact outputs of our own keyframe sampler"
)]

use alloc::sync::Arc;

extern crate alloc;

use stackchan_core::modifiers::DancePlayer;
use stackchan_core::{Decorator, Director, Emotion, Entity, Instant};
use stackchan_net::dance::parse_dance;

/// 30 FPS render tick — matches the firmware.
const TICK_MS: u64 = 33;

/// Step the director from `start_ms` to `target_ms` inclusive at
/// 30 FPS so the test's expected behaviour is observed at the
/// exact tick boundary.
fn run_through(director: &mut Director<'_>, entity: &mut Entity, start_ms: u64, target_ms: u64) {
    let mut t = start_ms;
    while t <= target_ms {
        director.run(entity, Instant::from_millis(t));
        t += TICK_MS;
    }
}

#[test]
fn full_pipeline_motion_avatar_rgb() {
    let body = r#"{"keyframes":[
        {"at_ms":0,"pan_deg":15.0,"tilt_deg":10.0,"emotion":"happy","r":255,"g":200,"b":0},
        {"at_ms":500,"pan_deg":-15.0,"decorator":"heart"},
        {"at_ms":1000,"pan_deg":0.0,"tilt_deg":0.0,"r":0,"g":0,"b":255}
    ]}"#;
    let script = parse_dance(body).unwrap();

    let mut entity = Entity::default();
    let mut player = DancePlayer::new();
    let mut director = Director::new();
    director.add_modifier(&mut player).unwrap();

    // Hand the script in through the same pathway the firmware uses.
    entity.input.dance_script = Some(Arc::new(script));

    // Tick 0 — first keyframe lands.
    director.run(&mut entity, Instant::from_millis(0));
    assert_eq!(entity.motor.head_pose.pan_deg, 15.0);
    assert_eq!(entity.motor.head_pose.tilt_deg, 10.0);
    assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
    assert_eq!(entity.led_override, Some([255, 200, 0]));
    // The input slot is now drained (player consumed it).
    assert!(entity.input.dance_script.is_none());

    // Advance past t=500 — pan tracks new keyframe; emotion + RGB
    // hold over from t=0; decorator from t=500 fires.
    run_through(&mut director, &mut entity, TICK_MS, 600);
    assert!(
        (entity.motor.head_pose.pan_deg - (-15.0)).abs() < 0.5,
        "expected pan ≈ -15 after kf=500, got {}",
        entity.motor.head_pose.pan_deg
    );
    assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
    assert_eq!(entity.led_override, Some([255, 200, 0]));
    assert_eq!(
        entity.face.decorator.map(|d| d.kind),
        Some(Decorator::Heart)
    );

    // Advance past t=1000 — final keyframe: pose returns to zero;
    // RGB shifts to blue.
    run_through(&mut director, &mut entity, 600 + TICK_MS, 1_100);
    assert!(
        entity.motor.head_pose.pan_deg.abs() < 0.5,
        "expected pan ≈ 0 after kf=1000, got {}",
        entity.motor.head_pose.pan_deg
    );
    assert_eq!(entity.led_override, Some([0, 0, 255]));
}

#[test]
fn script_completion_clears_overrides() {
    let body = r#"{"keyframes":[
        {"at_ms":0,"pan_deg":10.0,"emotion":"happy","r":255,"g":0,"b":0}
    ]}"#;
    let script = parse_dance(body).unwrap();

    let mut entity = Entity::default();
    let mut player = DancePlayer::new();
    let mut director = Director::new();
    director.add_modifier(&mut player).unwrap();

    entity.input.dance_script = Some(Arc::new(script));
    director.run(&mut entity, Instant::from_millis(0));
    assert_eq!(entity.led_override, Some([255, 0, 0]));

    // Past the last keyframe + the script tail, overrides clear.
    run_through(&mut director, &mut entity, TICK_MS, 5_000);
    assert_eq!(entity.led_override, None);
    assert!(entity.face.decorator.is_none());
}

#[test]
fn partial_rgb_keyframe_is_parser_rejected() {
    // The parser refuses partial RGB triples, so the script never
    // reaches the player.
    let body = r#"{"keyframes":[{"at_ms":0,"r":10,"g":20}]}"#;
    assert!(parse_dance(body).is_err());
}
