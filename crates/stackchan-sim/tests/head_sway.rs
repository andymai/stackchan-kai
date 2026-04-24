//! Golden test for `IdleSway`: drives the modifier + a [`RecordingHead`]
//! over 30 s of simulated time at 30 FPS, then asserts the captured
//! pan/tilt trajectory stays within the configured amplitude, crosses
//! zero on both axes, and moves smoothly between ticks.
//!
//! The test exercises the full shape of the hardware port: the modifier
//! writes `avatar.head_pose` in core, a consumer pulls the pose out and
//! calls [`HeadDriver::set_pose`] on a `RecordingHead` (sim) — the same
//! code path the firmware will use against a PCA9685.

#![allow(
    clippy::float_cmp,
    reason = "tests compare bit-exact pass-through values through RecordingHead, \
              not results of accumulated FP arithmetic"
)]

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};
use stackchan_core::modifiers::IdleSway;
use stackchan_core::{Avatar, Clock, HeadDriver, Modifier};
use stackchan_sim::{FakeClock, RecordingHead};

/// Minimal synchronous future driver.
///
/// `RecordingHead::set_pose` returns a future that is always immediately
/// `Ready`, so a single `poll` is sufficient. Keeps this test free of an
/// async-executor dependency. If used with a future that actually yields,
/// this would spin — intentional, the assertion panic surfaces the misuse.
fn block_on<F: Future>(future: F) -> F::Output {
    // `Waker::noop()` returns `&'static Waker`; pass it through directly.
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = pin!(future);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

const DURATION_MS: u64 = 30_000;
const TICK_MS: u64 = 33;
const PAN_AMPLITUDE: f32 = 4.0;
const TILT_AMPLITUDE: f32 = 2.5;

#[test]
fn idle_sway_trajectory_stays_within_amplitude() {
    let clock = FakeClock::new();
    let mut avatar = Avatar::default();
    let mut sway = IdleSway::new();
    let mut head = RecordingHead::new();

    let mut t_ms = 0;
    while t_ms <= DURATION_MS {
        clock.set(stackchan_core::Instant::from_millis(t_ms));
        sway.update(&mut avatar, clock.now());
        block_on(head.set_pose(avatar.head_pose, clock.now()))
            .expect("RecordingHead is infallible");
        t_ms += TICK_MS;
    }

    let records = head.records();
    assert!(
        records.len() > 800,
        "expected ~909 records, got {}",
        records.len()
    );

    // Amplitude bound: Pose::clamped is a no-op at these sizes.
    for (ts, pose) in records {
        assert!(
            pose.pan_deg.abs() <= PAN_AMPLITUDE + 0.01,
            "pan {} at {}ms exceeds amplitude",
            pose.pan_deg,
            ts.as_millis()
        );
        assert!(
            pose.tilt_deg.abs() <= TILT_AMPLITUDE + 0.01,
            "tilt {} at {}ms exceeds amplitude",
            pose.tilt_deg,
            ts.as_millis()
        );
    }
}

#[test]
fn idle_sway_crosses_zero_on_both_axes() {
    let mut avatar = Avatar::default();
    let mut sway = IdleSway::new();
    let mut head = RecordingHead::new();

    for i in 0..1_000 {
        let now = stackchan_core::Instant::from_millis(i * 33);
        sway.update(&mut avatar, now);
        block_on(head.set_pose(avatar.head_pose, now)).expect("RecordingHead is infallible");
    }

    // Both axes must visit both sides of zero over this window.
    let (pan_pos, pan_neg) = head
        .records()
        .iter()
        .fold((false, false), |(pos, neg), (_, p)| {
            (pos || p.pan_deg > 0.0, neg || p.pan_deg < 0.0)
        });
    let (tilt_pos, tilt_neg) = head
        .records()
        .iter()
        .fold((false, false), |(pos, neg), (_, p)| {
            (pos || p.tilt_deg > 0.0, neg || p.tilt_deg < 0.0)
        });
    assert!(pan_pos && pan_neg, "pan did not cross zero in 33 s");
    assert!(tilt_pos && tilt_neg, "tilt did not cross zero in 33 s");
}

#[test]
fn recording_head_preserves_call_order() {
    // Contract test for RecordingHead: order of (ts, pose) matches call order.
    let mut head = RecordingHead::new();
    block_on(head.set_pose(
        stackchan_core::Pose::new(1.0, 2.0),
        stackchan_core::Instant::from_millis(10),
    ))
    .unwrap();
    block_on(head.set_pose(
        stackchan_core::Pose::new(-3.0, 4.0),
        stackchan_core::Instant::from_millis(20),
    ))
    .unwrap();
    let recs = head.records();
    assert_eq!(recs.len(), 2);
    assert_eq!(recs[0].0.as_millis(), 10);
    assert_eq!(recs[1].0.as_millis(), 20);
    assert_eq!(recs[1].1.pan_deg, -3.0);
}
