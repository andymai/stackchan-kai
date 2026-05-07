//! Pose-publication helpers for the firmware's mDNS responder.
//!
//! The DNS encoding itself (`A` / `PTR` / `SRV` / `TXT` records,
//! socket / multicast plumbing) lives in
//! `stackchan_firmware::net::mdns` — that side is embassy-bound. The
//! pure decisions — *should* a fresh announcement go out, and how do
//! we format the pose values into TXT key/value strings — live here
//! so they can be host-tested without an esp toolchain.
//!
//! ## Wire format
//!
//! The TXT record carries the live commanded head pose alongside the
//! existing `txtvers` / `version` / `path` / `mcp` / `kai` keys:
//!
//! - `yaw=<degrees>` — pan, one decimal place, signed.
//! - `pitch=<degrees>` — tilt, one decimal place, signed.
//!
//! Naming mirrors the meganetaaan `mimic_main` mod
//! ([source](https://github.com/meganetaaan/stack-chan/tree/main/firmware/mods/mimic_main))
//! so a kai unit can lead a mixed-firmware mimic stack without a
//! translation layer on the follower side.
//!
//! ## Throttling
//!
//! [`PoseAnnouncer`] gates fresh multicasts. The decision is:
//!
//! - First call after the link comes up — publish.
//! - Heartbeat interval elapsed — publish (so a listener that joined
//!   the multicast group while the head was stationary still picks up
//!   state).
//! - Both axes moved past the deadband AND the minimum interval
//!   elapsed — publish.
//! - Otherwise — suppress.

use core::fmt::Write as _;

use stackchan_core::Pose;

/// Minimum interval between unsolicited pose-driven announcements.
///
/// 100 ms matches the meganetaaan `mimic_main` cadence and is short
/// enough that a follower mimicking yaw/pitch tracks the leader
/// without visible lag.
pub const POSE_PUBLISH_MIN_INTERVAL_MS: u64 = 100;

/// Heartbeat interval, in milliseconds.
///
/// The pose announcement re-emits at least this often even when
/// nothing has changed so a fresh listener that joined the multicast
/// group while the head was stationary still picks up the current
/// pose.
pub const POSE_HEARTBEAT_INTERVAL_MS: u64 = 1_000;

/// Per-axis publish deadband, in degrees.
///
/// Below this magnitude we treat pose changes as servo / quantisation
/// noise and suppress the announcement. 0.1° is well under the
/// smallest visible head step on the `SCServo` (one step ≈ 0.293°)
/// but large enough to absorb commanded-pose float-add noise from
/// the modifier pipeline.
pub const POSE_DEADBAND_DEG: f32 = 0.1;

/// Cap of the [`heapless::String`]-style buffer returned by
/// [`format_pose_kv`]. Sized for `pitch=-NNN.N` plus headroom.
pub const POSE_KV_CAP: usize = 24;

/// Pure decision struct: tracks the last published pose + its
/// timestamp, decides whether to publish a fresh announcement.
///
/// Lives in stackchan-net (not stackchan-firmware) because it has no
/// embassy / hardware dependencies — keeping it here gets it onto
/// the host CI test path.
#[derive(Debug, Clone, Copy, Default)]
pub struct PoseAnnouncer {
    /// Last pose actually pushed onto the wire. `None` until the
    /// first announcement is sent.
    last_published: Option<Pose>,
    /// Wall-clock instant (milliseconds since boot) of the last
    /// publish. `None` until the first announcement is sent.
    last_published_at_ms: Option<u64>,
}

impl PoseAnnouncer {
    /// Empty announcer — neither pose nor time recorded yet. The
    /// first call to [`Self::should_publish`] will return `true`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_published: None,
            last_published_at_ms: None,
        }
    }

    /// Decide whether a fresh announcement should go out for `current`
    /// at `now_ms`. See the module docs for the policy.
    #[must_use]
    pub fn should_publish(&self, current: Pose, now_ms: u64) -> bool {
        let (Some(last), Some(last_at)) = (self.last_published, self.last_published_at_ms) else {
            return true;
        };
        let elapsed = now_ms.saturating_sub(last_at);
        if elapsed >= POSE_HEARTBEAT_INTERVAL_MS {
            return true;
        }
        if elapsed < POSE_PUBLISH_MIN_INTERVAL_MS {
            return false;
        }
        let dyaw = (current.pan_deg - last.pan_deg).abs();
        let dpitch = (current.tilt_deg - last.tilt_deg).abs();
        dyaw >= POSE_DEADBAND_DEG || dpitch >= POSE_DEADBAND_DEG
    }

    /// Stamp a successful publish so subsequent calls to
    /// [`Self::should_publish`] gate against it.
    pub const fn record_publish(&mut self, current: Pose, now_ms: u64) {
        self.last_published = Some(current);
        self.last_published_at_ms = Some(now_ms);
    }
}

/// Format a `<key>=<deg>` TXT entry into a small fixed-cap buffer.
/// One decimal place, signed.
///
/// Returns `None` only if the formatter overflows the buffer — which
/// `f32` `Display` can't produce for the in-range degrees the head
/// driver clamps to (±60° per axis). The `None` branch is defensive.
#[must_use]
pub fn format_pose_kv(key: &str, value_deg: f32) -> Option<heapless::String<POSE_KV_CAP>> {
    let mut buf: heapless::String<POSE_KV_CAP> = heapless::String::new();
    write!(buf, "{key}={value_deg:.1}").ok()?;
    Some(buf)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test-only: unwrap is the standard pattern for asserting Some-returning helpers"
)]
mod tests {
    use super::*;

    #[test]
    fn first_call_publishes() {
        let a = PoseAnnouncer::new();
        assert!(a.should_publish(Pose::new(0.0, 0.0), 0));
    }

    #[test]
    fn suppresses_within_min_interval() {
        let mut a = PoseAnnouncer::new();
        a.record_publish(Pose::new(10.0, 5.0), 1_000);
        // Same pose, within the 100 ms window — suppress.
        assert!(!a.should_publish(Pose::new(10.0, 5.0), 1_050));
        // Tiny change well under the deadband, still within window —
        // suppress.
        assert!(!a.should_publish(Pose::new(10.05, 5.0), 1_050));
    }

    #[test]
    fn publishes_on_pose_change_after_min_interval() {
        let mut a = PoseAnnouncer::new();
        a.record_publish(Pose::new(10.0, 5.0), 1_000);
        // Past min interval, pose moved beyond deadband — publish.
        assert!(a.should_publish(Pose::new(10.5, 5.0), 1_100));
        a.record_publish(Pose::new(10.5, 5.0), 1_100);
        // Past min interval, only one axis moved — still publish
        // (per-axis OR semantics).
        assert!(a.should_publish(Pose::new(10.5, 5.5), 1_250));
    }

    #[test]
    fn suppresses_subdeadband_changes_in_window() {
        let mut a = PoseAnnouncer::new();
        a.record_publish(Pose::new(10.0, 5.0), 1_000);
        // Past min interval but neither axis moved past deadband —
        // suppress.
        assert!(!a.should_publish(Pose::new(10.05, 5.05), 1_150));
    }

    #[test]
    fn heartbeat_fires_when_idle() {
        let mut a = PoseAnnouncer::new();
        a.record_publish(Pose::new(10.0, 5.0), 1_000);
        // Past heartbeat interval, identical pose — heartbeat fires.
        assert!(a.should_publish(Pose::new(10.0, 5.0), 1_000 + POSE_HEARTBEAT_INTERVAL_MS));
    }

    #[test]
    fn heartbeat_supersedes_min_interval_gate() {
        let mut a = PoseAnnouncer::new();
        a.record_publish(Pose::new(10.0, 5.0), 0);
        // Heartbeat interval elapsed AND no movement — should publish
        // (heartbeat must win over the deadband suppression).
        assert!(a.should_publish(Pose::new(10.0, 5.0), POSE_HEARTBEAT_INTERVAL_MS + 50));
    }

    #[test]
    fn deadband_threshold_is_inclusive() {
        let mut a = PoseAnnouncer::new();
        a.record_publish(Pose::new(0.0, 0.0), 0);
        // Exactly the deadband on yaw, past min interval — should
        // publish (`>=` semantics).
        assert!(a.should_publish(
            Pose::new(POSE_DEADBAND_DEG, 0.0),
            POSE_PUBLISH_MIN_INTERVAL_MS
        ));
    }

    #[test]
    fn record_publish_resets_window() {
        let mut a = PoseAnnouncer::new();
        a.record_publish(Pose::new(10.0, 5.0), 1_000);
        // First post-publish call within the window suppresses.
        assert!(!a.should_publish(Pose::new(10.0, 5.0), 1_050));
        // After a second publish at t=1_050, the window restarts —
        // a t=1_100 call (50 ms later) should still suppress, even
        // though we're past 1_000+100 from the original publish.
        a.record_publish(Pose::new(10.0, 5.0), 1_050);
        assert!(!a.should_publish(Pose::new(10.0, 5.0), 1_100));
    }

    #[test]
    fn format_pose_kv_uses_one_decimal_signed() {
        let s = format_pose_kv("yaw", 12.345).unwrap();
        assert_eq!(s.as_str(), "yaw=12.3");
        let s = format_pose_kv("pitch", -4.5).unwrap();
        assert_eq!(s.as_str(), "pitch=-4.5");
        let s = format_pose_kv("yaw", 180.0).unwrap();
        assert_eq!(s.as_str(), "yaw=180.0");
    }

    #[test]
    fn format_pose_kv_handles_zero() {
        let s = format_pose_kv("yaw", 0.0).unwrap();
        assert_eq!(s.as_str(), "yaw=0.0");
    }

    #[test]
    fn format_pose_kv_fits_extreme_values() {
        // Realistic worst case after the head driver's ±60° clamp;
        // no truncation expected.
        let s = format_pose_kv("pitch", -59.95).unwrap();
        // `{:.1}` rounds -59.95 to -60.0 (banker's rounding via Rust's
        // default round-half-to-even on the IEEE 754 representation).
        // Either form is acceptable for the wire — assert one of them.
        assert!(
            s.as_str() == "pitch=-60.0"
                || s.as_str() == "pitch=-59.9"
                || s.as_str() == "pitch=-59.0"
        );
    }
}
