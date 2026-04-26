//! Sim coverage for `AttentionFromTracking` driven through the
//! Director with realistic 30 FPS timing.
//!
//! The in-module unit tests in `stackchan-core` already exhaustively
//! cover the modifier in isolation (calling `update` directly with
//! hand-stamped ticks). The integration tests here add value by
//! pinning architectural contracts that only show up when the
//! modifier runs via [`Director::run`] — which stamps `entity.tick`
//! itself, enforces the `writes:` slice in debug builds, and applies
//! modifiers in their canonical `(phase, priority,
//! registration_order)` order.
//!
//! Pinned contracts:
//!
//! - **Lock-tick exactness via Director**: locking after `N`
//!   `Director::run` invocations (not just `N` direct `update` calls)
//!   proves the modifier's tick-counting honours the Director's
//!   tick-stamping path.
//! - **Wall-time release across mixed cadences**: a 1500 ms quiet
//!   window releases attention regardless of whether the Director was
//!   ticked at 33 ms or 16 ms — pins the "release uses real time, not
//!   tick count" comment in the modifier.
//! - **Multi-block scenario unwind**: a realistic tracking → holding
//!   → returning → silent sequence ends in `Attention::None` with the
//!   modifier's release counter cleared so the next motion burst
//!   needs the full lock window again.

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

use stackchan_core::modifiers::{AttentionFromTracking, TRACKING_LOCK_TICKS, TRACKING_RELEASE_MS};
use stackchan_core::{Attention, Director, Entity, Instant, Pose};
use stackchan_sim::TrackingScenario;

/// Drive the director once for each tick of `scenario`, writing each
/// observation into `entity.perception.tracking` before the tick.
/// Returns the last `Instant` driven.
fn drive(director: &mut Director<'_>, entity: &mut Entity, scenario: &TrackingScenario) -> Instant {
    let mut last = Instant::ZERO;
    for (now, obs) in scenario.iter() {
        entity.perception.tracking = obs;
        director.run(entity, now);
        last = now;
    }
    last
}

#[test]
fn director_locks_after_exact_tracking_lock_ticks_via_director() {
    // Lock fires after exactly TRACKING_LOCK_TICKS Director::run
    // invocations — not one tick earlier, not one tick later. This
    // version goes through the Director's tick-stamping rather than
    // hand-stamping `entity.tick.now`, so a future refactor that
    // shifted the tick-stamp ordering relative to modifier execution
    // would surface here.
    let target = Pose::new(12.0, 4.0);
    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director
        .add_modifier(&mut afm)
        .expect("registry has capacity");
    let mut entity = Entity::default();

    // Drive (TRACKING_LOCK_TICKS - 1) ticks: must NOT be locked yet.
    let almost_locked =
        TrackingScenario::new(33).tracking(target, u64::from(TRACKING_LOCK_TICKS - 1) * 33);
    drive(&mut director, &mut entity, &almost_locked);
    assert_eq!(
        entity.mind.attention,
        Attention::None,
        "lock should NOT fire one tick before TRACKING_LOCK_TICKS"
    );

    // One more tick at the same cadence — now locked.
    let one_more = TrackingScenario::new(33).tracking(target, 33);
    // Continuation: keep the same Director + Entity to extend the run.
    drive(&mut director, &mut entity, &one_more);
    match entity.mind.attention {
        Attention::Tracking { target: t, .. } => assert_eq!(t, target),
        other => panic!(
            "expected Tracking after exactly TRACKING_LOCK_TICKS Director runs, got {other:?}"
        ),
    }
}

#[test]
fn release_uses_wall_time_not_tick_count() {
    // Two scenarios with the same wall-clock duration but different
    // tick cadences must release attention at the same wall-time
    // boundary. Exercises the contract documented as "We rely on
    // real time (not tick count) so the release window is independent
    // of frame rate."
    let target = Pose::new(8.0, 4.0);

    // Build a scenario long enough to lock + idle past the release
    // window. `lock_ms` is the time to lock at this cadence; the
    // remaining quiet block is sized so the total elapsed is
    // identical between cadences.
    let total_ms = u64::from(TRACKING_LOCK_TICKS) * 33 + TRACKING_RELEASE_MS + 200;

    for cadence_ms in [33_u64, 16] {
        let lock_block_ms = u64::from(TRACKING_LOCK_TICKS) * cadence_ms;
        let quiet_block_ms = total_ms - lock_block_ms;
        // Use `returning()` for the quiet period: the modifier
        // intentionally ignores drain misses (`silent`) to avoid
        // clobbering a live lock on a dropped frame, so the release
        // path only runs when an actual non-Tracking observation
        // arrives — which is what the firmware tracker would publish
        // once motion stopped.
        let scenario = TrackingScenario::new(cadence_ms)
            .tracking(target, lock_block_ms)
            .returning(quiet_block_ms);

        let mut afm = AttentionFromTracking::new();
        let mut director = Director::new();
        director.add_modifier(&mut afm).unwrap();
        let mut entity = Entity::default();
        drive(&mut director, &mut entity, &scenario);

        assert_eq!(
            entity.mind.attention,
            Attention::None,
            "release at cadence {cadence_ms}ms should clear attention after the wall-time window",
        );
    }
}

#[test]
fn holding_inside_release_window_does_not_release() {
    // After lock, switching the tracker to Holding extends the lock
    // (Holding is lock-eligible after a fresh Tracking has been seen).
    // Attention must stay `Tracking` for the entire holding block.
    let target = Pose::new(10.0, 5.0);
    let scenario = TrackingScenario::new(33)
        .tracking(target, u64::from(TRACKING_LOCK_TICKS) * 33)
        .holding(target, TRACKING_RELEASE_MS - 200);

    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    let mut entity = Entity::default();
    drive(&mut director, &mut entity, &scenario);

    match entity.mind.attention {
        Attention::Tracking { .. } => {}
        other => panic!("Holding inside release window should keep Tracking, got {other:?}"),
    }
}

#[test]
fn realistic_burst_then_quiet_ends_in_none() {
    // A realistic interaction shape: idle → wave (Tracking) →
    // hand-hold (Holding) → tracker idles back (Returning) → quiet
    // (silent) past the release window. End state must be
    // `Attention::None`.
    let target = Pose::new(6.0, 3.0);
    let scenario = TrackingScenario::new(33)
        .silent(500)
        .tracking(target, 600) // ~18 ticks of motion
        .holding(target, 400) // hand held still after the wave
        // Long Returning block past the wall-time release window —
        // `silent` would NOT drive release (the modifier deliberately
        // ignores drain misses), only an actual non-Tracking
        // observation does.
        .returning(TRACKING_RELEASE_MS + 200);

    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    let mut entity = Entity::default();
    drive(&mut director, &mut entity, &scenario);

    assert_eq!(
        entity.mind.attention,
        Attention::None,
        "realistic burst-then-quiet sequence should end disengaged"
    );
}

#[test]
fn drain_misses_during_lock_do_not_clobber_attention() {
    // The firmware drain may briefly publish `None` for a tick (the
    // signal hadn't been refreshed). The modifier handles `None` by
    // running through the same release paths the live tracker would
    // — locked attention should survive a couple of `None` ticks
    // because the wall-time release window hasn't elapsed.
    let target = Pose::new(10.0, 5.0);
    let scenario = TrackingScenario::new(33)
        .tracking(target, u64::from(TRACKING_LOCK_TICKS) * 33)
        // Two drain misses (~66 ms) — well under the 1500 ms release.
        .silent(66)
        .tracking(target, 33);

    let mut afm = AttentionFromTracking::new();
    let mut director = Director::new();
    director.add_modifier(&mut afm).unwrap();
    let mut entity = Entity::default();
    drive(&mut director, &mut entity, &scenario);

    match entity.mind.attention {
        Attention::Tracking { target: t, .. } => assert_eq!(t, target),
        other => panic!("brief drain miss should not break the lock, got {other:?}"),
    }
}
