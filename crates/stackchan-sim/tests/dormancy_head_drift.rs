//! Sim coverage for the dormancy → IdleHeadDrift gating handoff.
//!
//! `DormancyFromActivity` writes `mind.dormancy`; `IdleHeadDrift` reads
//! it. This test pins the architectural contract that solves the
//! office-noise problem: once dormancy fires, no new glance starts,
//! and any in-flight glance gets cancelled — so the SCServo stays
//! still.
//!
//! In-module unit tests cover both modifiers in isolation. The sim
//! test here pins the cross-modifier composition through the real
//! Director:
//!
//! - Quiet past dormancy timeout → IdleHeadDrift produces zero pose
//!   for the entire post-timeout window, regardless of where the
//!   glance scheduler would otherwise fire.
//! - Activity reawakens → glances resume within the next
//!   `GLANCE_INTERVAL_MAX_MS + window`.

#![allow(
    clippy::doc_markdown,
    clippy::unwrap_used,
    reason = "test-only relaxations: doc comments reference type names without \
              backticks; Director::add_modifier is unwrapped on a fresh registry"
)]

use stackchan_core::modifiers::{
    DORMANCY_TIMEOUT_MS, DormancyFromActivity, GLANCE_EASE_IN_MS, GLANCE_EASE_OUT_MS,
    GLANCE_HOLD_MS, GLANCE_INTERVAL_MAX_MS, IdleHeadDrift,
};
use stackchan_core::{Director, Dormancy, Entity, Instant, Intent, Pose};

const TICK_MS: u64 = 33;

/// Full glance window — useful for sizing observation windows that
/// must outlast a single in-flight glance.
const GLANCE_WINDOW_MS: u64 = GLANCE_EASE_IN_MS + GLANCE_HOLD_MS + GLANCE_EASE_OUT_MS;

#[test]
fn asleep_holds_zero_pose_across_long_observation_window() {
    let mut head_drift = IdleHeadDrift::new();
    let mut dormancy = DormancyFromActivity::new();
    let mut director = Director::new();
    director.add_modifier(&mut dormancy).unwrap();
    director.add_modifier(&mut head_drift).unwrap();
    let mut entity = Entity::default();

    // Walk past the dormancy timeout with no activity. The head may
    // glance once or twice before timeout fires; that's expected and
    // doesn't violate the office-quiet contract.
    let mut t_ms = 0;
    while t_ms <= DORMANCY_TIMEOUT_MS + 200 {
        let now = Instant::from_millis(t_ms);
        entity.tick.now = now;
        entity.mind.intent = Intent::Idle;
        director.run(&mut entity, now);
        t_ms += TICK_MS;
    }
    assert!(
        entity.mind.dormancy.is_asleep(),
        "should be Asleep after >= DORMANCY_TIMEOUT_MS of quiet, got {:?}",
        entity.mind.dormancy,
    );

    // Drive a long observation window AFTER dormancy has fired.
    // Must outlast any worst-case in-flight glance. Every captured
    // pose must be exactly the neutral pose — that's the
    // SCServo-quietness contract.
    let observation_ms = GLANCE_INTERVAL_MAX_MS + 2 * GLANCE_WINDOW_MS;
    let stop_ms = t_ms + observation_ms;
    while t_ms <= stop_ms {
        let now = Instant::from_millis(t_ms);
        entity.tick.now = now;
        entity.mind.intent = Intent::Idle;
        director.run(&mut entity, now);
        assert_eq!(
            entity.motor.head_pose,
            Pose::default(),
            "head must stay at neutral pose while Asleep at {t_ms}ms",
        );
        t_ms += TICK_MS;
    }
}

#[test]
fn activity_after_dormancy_resumes_glances() {
    let mut head_drift = IdleHeadDrift::new();
    let mut dormancy = DormancyFromActivity::new();
    let mut director = Director::new();
    director.add_modifier(&mut dormancy).unwrap();
    director.add_modifier(&mut head_drift).unwrap();
    let mut entity = Entity::default();

    // Sleep first.
    let mut t_ms = 0;
    while t_ms <= DORMANCY_TIMEOUT_MS + 200 {
        let now = Instant::from_millis(t_ms);
        entity.tick.now = now;
        entity.mind.intent = Intent::Idle;
        director.run(&mut entity, now);
        t_ms += TICK_MS;
    }
    assert!(entity.mind.dormancy.is_asleep());

    // Now hold activity (Petting) for the full max-interval-plus-glance
    // window. The dormancy state flips Awake immediately on the next
    // tick; a glance must fire within `GLANCE_INTERVAL_MAX_MS + glance window`.
    let bound_ms = GLANCE_INTERVAL_MAX_MS + GLANCE_WINDOW_MS + 200;
    let stop_ms = t_ms + bound_ms;
    let mut saw_motion = false;
    while t_ms <= stop_ms {
        let now = Instant::from_millis(t_ms);
        entity.tick.now = now;
        entity.mind.intent = Intent::Petted;
        director.run(&mut entity, now);
        if entity.motor.head_pose != Pose::default() {
            saw_motion = true;
            break;
        }
        t_ms += TICK_MS;
    }

    assert_eq!(entity.mind.dormancy, Dormancy::Awake);
    assert!(
        saw_motion,
        "no glance fired within {bound_ms} ms of waking — scheduler may not have re-anchored",
    );
}
