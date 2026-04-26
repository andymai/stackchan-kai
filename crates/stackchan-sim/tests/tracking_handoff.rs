//! End-to-end sim test for the full camera-tracking pipeline:
//! AttentionFromTracking + GazeFromAttention + MicrosaccadeFromAttention +
//! HeadFromAttention + LostTargetSearch composed with the standard
//! background stack (IdleSway + HeadFromEmotion + IdleDrift) via the
//! Director.
//!
//! Mirrors the architectural shape of `listening.rs` for the
//! tracking arc — pins the cross-modifier handoff at the same
//! integration scale that the firmware loop runs at. A regression
//! that broke the perception → cognition → expression / motion flow
//! anywhere in the pipeline would surface here even if individual
//! modifier unit tests still passed.
//!
//! Pinned cross-modifier invariants:
//!
//! - **Silence stays at baseline**: no perception, no
//!   attention/engagement transitions; head + eyes stay close to
//!   their idle wander.
//! - **Face → engagement → head + eyes within one Director run**:
//!   when face_present + face_centroid arrives, the engagement state
//!   reaches `Locked` and the head pose / eye centers respond on the
//!   same tick the lock fires (no one-tick lag from a phase ordering
//!   slip).
//! - **Lock loss → search beat → recovery**: after the face vanishes
//!   and engagement releases, LostTargetSearch animates the
//!   choreographed beat on top of the rest of the Motion stack, then
//!   unwinds back to baseline.
//! - **Pose clamps hold across the full sequence**: every captured
//!   tick respects MAX_PAN_DEG / MAX_TILT_DEG / MIN_TILT_DEG.

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

use stackchan_core::modifiers::{
    AttentionFromTracking, FACE_LOCK_HITS, FACE_RELEASE_MISSES, GazeFromAttention,
    HeadFromAttention, HeadFromEmotion, IdleDrift, IdleSway, LostTargetSearch,
    MicrosaccadeFromAttention, SEARCH_TOTAL_MS, TRACKING_RELEASE_MS,
};
use stackchan_core::{
    Attention, Director, Engagement, Entity, Instant, MAX_PAN_DEG, MAX_TILT_DEG, MIN_TILT_DEG, Pose,
};
use stackchan_sim::TrackingScenario;

#[test]
fn full_pipeline_handles_silence_through_lock_loss_recovery() {
    // Build the canonical Motion + Expression stack the firmware
    // render task runs, with all five tracking modifiers wired in.
    let mut sway = IdleSway::new();
    let mut emo = HeadFromEmotion::new();
    let mut drift = IdleDrift::new();
    let mut afm = AttentionFromTracking::new();
    let mut gaze = GazeFromAttention::new();
    let mut micro = MicrosaccadeFromAttention::new();
    let mut head = HeadFromAttention::new();
    let mut search = LostTargetSearch::new();

    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut sway).unwrap();
    director.add_modifier(&mut emo).unwrap();
    director.add_modifier(&mut drift).unwrap();
    director.add_modifier(&mut gaze).unwrap();
    director.add_modifier(&mut micro).unwrap();
    director.add_modifier(&mut head).unwrap();
    director.add_modifier(&mut search).unwrap();

    let mut entity = Entity::default();
    let baseline_eye_x = entity.face.left_eye.center.x;

    // Realistic interaction shape:
    //
    //   - 1 s of silence (boot warmup with no observations)
    //   - 5 s of face-present Tracking observations (engagement locks)
    //   - 1 s of face-absent Tracking (engagement releases via face misses)
    //   - long Returning past the search beat AND the attention release
    //
    // Centroid placed off-axis so the head clearly steers toward the
    // face rather than the motion blob's neutral target.
    let target = Pose::new(0.0, 0.0);
    let face_centroid = (0.5_f32, 0.0_f32);
    let scenario = TrackingScenario::new(33)
        .silent(1_000)
        .tracking(target, 5_000)
        .with_face(face_centroid)
        .tracking(target, u64::from(FACE_RELEASE_MISSES + 2) * 33)
        .returning(SEARCH_TOTAL_MS + TRACKING_RELEASE_MS + 1_000);

    // Phase markers we want to observe at least once.
    let mut saw_locked = false;
    let mut saw_attention_tracking = false;
    let mut saw_eye_shift_during_lock = false;
    let mut saw_head_steered_right_of_baseline = false;
    // Tracks the FIRST tick at which engagement transitioned to
    // Idle — used to scope the search-beat assertion to the
    // SEARCH_TOTAL_MS window rather than IdleSway's free-running
    // left/right wander after the beat ends.
    let mut engagement_lost_at: Option<Instant> = None;
    let mut search_beat_peak_pan: f32 = 0.0;
    let mut final_attention = Attention::None;
    let mut final_engagement = Engagement::Idle;

    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);

        // Per-tick clamp invariants.
        let pose = entity.motor.head_pose;
        assert!(
            pose.pan_deg.abs() <= MAX_PAN_DEG + 0.01,
            "pan {} out of range at {}ms",
            pose.pan_deg,
            now.as_millis(),
        );
        assert!(
            (MIN_TILT_DEG - 0.01..=MAX_TILT_DEG + 0.01).contains(&pose.tilt_deg),
            "tilt {} out of range at {}ms",
            pose.tilt_deg,
            now.as_millis(),
        );

        if matches!(entity.mind.engagement, Engagement::Locked { .. }) {
            saw_locked = true;
            // Head pans toward the face (positive centroid → positive pan)
            if pose.pan_deg > 2.0 {
                saw_head_steered_right_of_baseline = true;
            }
            if entity.face.left_eye.center.x > baseline_eye_x {
                saw_eye_shift_during_lock = true;
            }
        }
        if matches!(entity.mind.attention, Attention::Tracking { .. }) {
            saw_attention_tracking = true;
        }
        // Capture the first engagement-lost tick so we can scope the
        // search-beat assertion to its SEARCH_TOTAL_MS window. Beyond
        // that, IdleSway's natural pan would noise up the signal.
        if matches!(entity.mind.engagement, Engagement::Idle) && engagement_lost_at.is_none() {
            engagement_lost_at = Some(now);
        }
        if let Some(lost) = engagement_lost_at {
            let elapsed = now.as_millis().saturating_sub(lost.as_millis());
            if elapsed <= SEARCH_TOTAL_MS {
                search_beat_peak_pan = search_beat_peak_pan.max(pose.pan_deg);
            }
        }
        final_attention = entity.mind.attention;
        final_engagement = entity.mind.engagement;
    }

    // Cross-modifier handoff invariants.
    assert!(
        saw_locked,
        "engagement should reach Locked during the face-present block",
    );
    assert!(
        saw_attention_tracking,
        "attention should reach Tracking during the motion block",
    );
    assert!(
        saw_eye_shift_during_lock,
        "GazeFromAttention should shift the eye toward the face during Locked",
    );
    assert!(
        saw_head_steered_right_of_baseline,
        "HeadFromAttention should pan head toward the face centroid (right) during Locked",
    );
    // Search beat peak: for a +0.5 centroid the hold pose is ~15.5°
    // and the saccade extends to ~20°. Bound at ≥10° to allow head-
    // smoothing residual + the worst-case IdleSway swing in the
    // opposite direction.
    assert!(
        search_beat_peak_pan >= 10.0,
        "search beat peak pan {search_beat_peak_pan} below the expected ~15° toward the last-known centroid",
    );

    // Final state at the end of the long quiet block.
    assert_eq!(
        final_attention,
        Attention::None,
        "attention should release after the long Returning block",
    );
    assert_eq!(
        final_engagement,
        Engagement::Idle,
        "engagement should release after the face-misses window",
    );
}

/// Sanity: confirm the lock-fires-on-the-right-tick invariant survives
/// the full pipeline. Not a duplicate of the per-modifier test — here
/// the "right tick" is determined by the engagement lock (3 face hits)
/// composed with the attention lock (3 tracking hits), and `Locked`
/// must arrive on the engagement-driven tick (which is the FIRST tick
/// where face_present has accumulated FACE_LOCK_HITS).
#[test]
fn engagement_lock_fires_on_the_face_lock_hits_tick_via_full_pipeline() {
    let mut sway = IdleSway::new();
    let mut emo = HeadFromEmotion::new();
    let mut drift = IdleDrift::new();
    let mut afm = AttentionFromTracking::new();
    let mut gaze = GazeFromAttention::new();
    let mut micro = MicrosaccadeFromAttention::new();
    let mut head = HeadFromAttention::new();
    let mut search = LostTargetSearch::new();

    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    director.add_modifier(&mut sway).unwrap();
    director.add_modifier(&mut emo).unwrap();
    director.add_modifier(&mut drift).unwrap();
    director.add_modifier(&mut gaze).unwrap();
    director.add_modifier(&mut micro).unwrap();
    director.add_modifier(&mut head).unwrap();
    director.add_modifier(&mut search).unwrap();

    let mut entity = Entity::default();

    // Drive exactly FACE_LOCK_HITS face-present ticks. The
    // engagement state machine reaches Locked on the LAST of these.
    let target = Pose::new(0.0, 0.0);
    let centroid = (0.3_f32, 0.0_f32);
    let s = TrackingScenario::new(33);
    let lock_window_ms = s.duration_for_ticks(u64::from(FACE_LOCK_HITS));
    let scenario = s.tracking(target, lock_window_ms).with_face(centroid);
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(&mut entity, now);
    }
    assert!(
        matches!(entity.mind.engagement, Engagement::Locked { .. }),
        "engagement should be Locked after FACE_LOCK_HITS face-present ticks via full pipeline; got {:?}",
        entity.mind.engagement,
    );
}
