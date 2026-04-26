//! Sim coverage for the dormancy → IdleSway gating handoff.
//!
//! `DormancyFromActivity` writes `mind.dormancy`; `IdleSway` reads
//! it. This test pins the architectural contract that solves the
//! office-noise problem: when no activity has happened for the
//! configured timeout, the head pose stops varying frame-to-frame
//! so the SCServo stays still.
//!
//! In-module unit tests cover the dormancy state machine in
//! isolation. The sim test here pins the cross-modifier composition
//! through the real Director:
//!
//! - Activity → IdleSway sweeps normally (the head actually moves).
//! - Quiet past timeout → dormancy flips to Asleep → IdleSway holds
//!   at zero contribution → captured pose trajectory becomes flat.
//! - Activity returns → wake → sway resumes.

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

use stackchan_core::modifiers::{DORMANCY_TIMEOUT_MS, DormancyFromActivity, IdleSway};
use stackchan_core::{Director, Dormancy, Entity, Instant, Intent};

const TICK_MS: u64 = 33;

/// Drive the director for `ticks` frames at `TICK_MS` cadence,
/// starting at `start_ms`. Optionally inject activity at every tick
/// via the `intent` callback.
fn run_for<F>(
    director: &mut Director<'_>,
    entity: &mut Entity,
    start_ms: u64,
    ticks: u64,
    intent: F,
) -> Instant
where
    F: Fn() -> Intent,
{
    let mut last = Instant::from_millis(start_ms);
    for t in 0..ticks {
        last = Instant::from_millis(start_ms + t * TICK_MS);
        entity.mind.intent = intent();
        director.run(entity, last);
    }
    last
}

#[test]
fn quiet_past_timeout_flattens_head_pose_trajectory() {
    let mut sway = IdleSway::new();
    let mut dormancy = DormancyFromActivity::new();
    let mut director = Director::new();
    director.add_modifier(&mut dormancy).unwrap();
    director.add_modifier(&mut sway).unwrap();
    let mut entity = Entity::default();

    // Phase 1: short window of no activity (well under the timeout).
    // Head must move during this period — IdleSway is sweeping.
    let phase1_ticks = 60; // ~2 s
    run_for(&mut director, &mut entity, 0, phase1_ticks, || Intent::Idle);
    assert_eq!(entity.mind.dormancy, Dormancy::Awake);
    let phase1_pose = entity.motor.head_pose;

    // Phase 2: cross the dormancy timeout still with no activity.
    let after_timeout_ms = DORMANCY_TIMEOUT_MS + 200;
    let after_timeout_ticks = after_timeout_ms / TICK_MS;
    run_for(
        &mut director,
        &mut entity,
        phase1_ticks * TICK_MS,
        after_timeout_ticks - phase1_ticks,
        || Intent::Idle,
    );
    assert!(
        entity.mind.dormancy.is_asleep(),
        "should be Asleep after >= DORMANCY_TIMEOUT_MS of quiet, got {:?}",
        entity.mind.dormancy,
    );

    // Capture the pose at the dormant boundary, then drive 30 more
    // ticks (~1 s) of asleep state. The head pose must not change
    // tick-to-tick — that's the SCServo-quietness contract.
    let dormant_pose = entity.motor.head_pose;
    for t in after_timeout_ticks..after_timeout_ticks + 30 {
        let now = Instant::from_millis(t * TICK_MS);
        entity.tick.now = now;
        entity.mind.intent = Intent::Idle;
        director.run(&mut entity, now);
        assert_eq!(
            entity.motor.head_pose,
            dormant_pose,
            "head pose must not vary tick-to-tick while Asleep at {}ms",
            (t * TICK_MS),
        );
    }

    // And the dormant pose itself must be different from the active
    // sway pose — proves IdleSway was actually sweeping during phase
    // 1 and is not just stuck at the same value.
    assert_ne!(
        phase1_pose, dormant_pose,
        "phase-1 sway pose should differ from the dormant pose — \
         otherwise the test isn't actually exercising sway motion",
    );
}

#[test]
fn activity_after_dormancy_resumes_sway() {
    let mut sway = IdleSway::new();
    let mut dormancy = DormancyFromActivity::new();
    let mut director = Director::new();
    director.add_modifier(&mut dormancy).unwrap();
    director.add_modifier(&mut sway).unwrap();
    let mut entity = Entity::default();

    // Sleep first.
    let after_timeout_ticks = (DORMANCY_TIMEOUT_MS + 200) / TICK_MS;
    run_for(&mut director, &mut entity, 0, after_timeout_ticks, || {
        Intent::Idle
    });
    assert!(entity.mind.dormancy.is_asleep());
    let dormant_pose = entity.motor.head_pose;

    // Now fire activity for 60 ticks (~2 s) and verify that:
    //  1. dormancy flips Awake immediately,
    //  2. the head pose changes from its dormant value at least
    //     once during the active window.
    let mut saw_motion = false;
    for t in after_timeout_ticks..after_timeout_ticks + 60 {
        let now = Instant::from_millis(t * TICK_MS);
        entity.tick.now = now;
        entity.mind.intent = Intent::Petted;
        director.run(&mut entity, now);
        if entity.motor.head_pose != dormant_pose {
            saw_motion = true;
        }
    }
    assert_eq!(entity.mind.dormancy, Dormancy::Awake);
    assert!(
        saw_motion,
        "head pose should vary from the dormant pose during activity",
    );
}
