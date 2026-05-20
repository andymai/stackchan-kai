//! Mimic-follower task — apply a leader's head pose locally.
//!
//! `serve_loop` in [`super::mdns`] inspects every inbound mDNS
//! multicast packet; when one carries a TXT record from the
//! configured `behavior.follower_leader_hostname`, the leader's
//! commanded `yaw` / `pitch` are signalled on
//! [`super::mdns::LEADER_POSE_SIGNAL`]. This task drains the
//! signal and converts each pose into a
//! [`RemoteCommand::LookAt`] hold long enough to bridge a missed
//! leader update.
//!
//! ## Hold duration
//!
//! The leader's `PoseAnnouncer` beats at most every
//! [`POSE_HEARTBEAT_INTERVAL_MS`] (1 s) even when stationary, so
//! a 1.5 s hold guarantees the follower never falls back to
//! autonomy mid-stream from a single dropped multicast packet.
//! A longer hold would stomp on operator-driven HTTP
//! `POST /look-at` commands; shorter would let one missed packet
//! return the head to autonomy.
//!
//! ## Empty leader → idle
//!
//! Empty `behavior.follower_leader_hostname` parks the task. No
//! signal source ever fires (the producer in `serve_loop` also
//! gates on a non-empty hostname), but the task still spawns —
//! this matches the pattern other opt-in tasks use
//! (`audio_debug`, `agent_sidecar`), so the boot-time spawn list
//! stays consistent regardless of config.

use alloc::string::String;

use embassy_time::{Duration, Timer};
use stackchan_core::input::RemoteCommand;
use stackchan_net::mdns_pose::POSE_HEARTBEAT_INTERVAL_MS;

use super::http::enqueue_remote_command;
use super::mdns::LEADER_POSE_SIGNAL;

/// `LookAt` hold per signalled leader update. 1.5 × the leader's
/// 1 s heartbeat so a single dropped multicast packet doesn't
/// drop the follower back to autonomy mid-stream. The
/// `POSE_HEARTBEAT_INTERVAL_MS` source constant is a `u64` for
/// arithmetic in milliseconds, but `RemoteCommand::LookAt`'s
/// `hold_ms` is `u32`; the constant fits in `u32` comfortably
/// (~12 days at 1 ms resolution before overflow).
#[allow(
    clippy::cast_possible_truncation,
    reason = "POSE_HEARTBEAT_INTERVAL_MS = 1000 is well below u32::MAX; the truncation is unreachable"
)]
const FOLLOWER_HOLD_MS: u32 = (POSE_HEARTBEAT_INTERVAL_MS as u32) * 3 / 2;

/// Follower task entry point.
///
/// Empty `leader_hostname` parks the task — there's no producer
/// for the signal in that case, so the loop would never advance.
#[embassy_executor::task]
pub async fn follower_task(leader_hostname: String) -> ! {
    if leader_hostname.is_empty() {
        defmt::info!("mdns-follower: no leader configured, idle");
        park_forever().await;
    }
    defmt::info!(
        "mdns-follower: tracking leader '{=str}' (hold {=u32}ms per update)",
        leader_hostname.as_str(),
        FOLLOWER_HOLD_MS,
    );
    loop {
        let pose = LEADER_POSE_SIGNAL.wait().await;
        enqueue_remote_command(RemoteCommand::LookAt {
            target: pose,
            hold_ms: FOLLOWER_HOLD_MS,
        });
    }
}

/// Spin forever, parking the task. Used when no leader is
/// configured.
async fn park_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}
