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

/// Walk a DNS response packet for the leader's TXT record and
/// extract its commanded head pose.
///
/// Inverse of the [`PoseAnnouncer`] producer side: a follower
/// inspects inbound mDNS multicast packets and lifts the leader's
/// `yaw=` / `pitch=` keys straight off the TXT record without an
/// HTTP round-trip.
///
/// `leader_hostname` is the local-name first label (e.g.
/// `"kitchen-cat"`); the parser builds the expected fully-qualified
/// record name `<leader_hostname>._stackchan._tcp.local` and
/// matches it case-insensitively against each answer record's
/// owner.
///
/// Returns `Some(Pose)` only when:
///
/// - The message has the QR bit set (it's a response, not a query).
/// - At least one answer record's name matches the leader's TXT name.
/// - That record's RDATA contains both `yaw=<f32>` and `pitch=<f32>`.
///
/// Returns `None` otherwise. Malformed packets, compression loops,
/// and partial-key records all short-circuit safely — the follower
/// either gets a clean pose or no pose, never a half-applied one.
#[must_use]
pub fn parse_response_pose(msg: &[u8], leader_hostname: &str) -> Option<Pose> {
    if msg.len() < 12 {
        return None;
    }
    // QR bit lives in the high bit of byte 2.
    if msg[2] & 0x80 == 0 {
        return None;
    }
    let qdcount = u16::from_be_bytes([msg[4], msg[5]]) as usize;
    let ancount = u16::from_be_bytes([msg[6], msg[7]]) as usize;
    if ancount == 0 {
        return None;
    }
    let mut off = 12;
    // Skip the question section. Each question is `name + qtype +
    // qclass` (the name uses the same compression scheme as
    // answers, so reuse `read_name`).
    for _ in 0..qdcount {
        let (_name, after) = read_name(msg, off)?;
        off = after.checked_add(4)?;
        if off > msg.len() {
            return None;
        }
    }
    for _ in 0..ancount {
        let (name, after) = read_name(msg, off)?;
        if after.checked_add(10)? > msg.len() {
            return None;
        }
        let rtype = u16::from_be_bytes([msg[after], msg[after + 1]]);
        let rdlength = u16::from_be_bytes([msg[after + 8], msg[after + 9]]) as usize;
        let rdata_start = after + 10;
        let rdata_end = rdata_start.checked_add(rdlength)?;
        if rdata_end > msg.len() {
            return None;
        }
        if rtype == 16 && is_leader_txt_name(name.as_str(), leader_hostname) {
            return extract_pose_from_txt_rdata(&msg[rdata_start..rdata_end]);
        }
        off = rdata_end;
    }
    None
}

/// Pull `yaw=` and `pitch=` out of a TXT record's RDATA.
///
/// RDATA is one or more length-prefixed strings concatenated per
/// RFC 1035 § 3.3.14. Each string is `<len:u8><bytes>`. Strings
/// the follower doesn't recognise are skipped — kai's TXT also
/// carries `txtvers` / `version` / `path` / `mcp` / `kai`, and
/// upstream stackchan may add others, so a tolerant scan keeps
/// us forward-compatible.
fn extract_pose_from_txt_rdata(rdata: &[u8]) -> Option<Pose> {
    let mut yaw: Option<f32> = None;
    let mut pitch: Option<f32> = None;
    let mut i = 0;
    while i < rdata.len() {
        let len = rdata[i] as usize;
        i += 1;
        if i.checked_add(len)? > rdata.len() {
            return None;
        }
        if let Ok(s) = core::str::from_utf8(&rdata[i..i + len]) {
            // First-valid-wins on duplicate keys. A naive
            // `yaw = v.parse().ok()` would let a malformed
            // second `yaw=` entry clobber a valid first reading
            // and surface as a follower stall on otherwise-good
            // leader traffic.
            if let Some(v) = s.strip_prefix("yaw=")
                && let Ok(val) = v.parse()
            {
                yaw.get_or_insert(val);
            } else if let Some(v) = s.strip_prefix("pitch=")
                && let Ok(val) = v.parse()
            {
                pitch.get_or_insert(val);
            }
        }
        i += len;
    }
    Some(Pose::new(yaw?, pitch?))
}

/// Case-insensitive match for `<leader_hostname>._stackchan._tcp.local`.
fn is_leader_txt_name(name: &str, leader_hostname: &str) -> bool {
    let mut parts = name.splitn(2, '.');
    let host = parts.next().unwrap_or("");
    let suffix = parts.next().unwrap_or("");
    host.eq_ignore_ascii_case(leader_hostname)
        && suffix.eq_ignore_ascii_case("_stackchan._tcp.local")
}

/// Walk DNS labels starting at `start`, returning the dotted name
/// and the offset just past the last label byte in the original
/// stream (NOT past any followed compression pointers).
///
/// Supports the RFC 1035 § 4.1.4 compression scheme — a label byte
/// whose high two bits are `11` is a 14-bit pointer into the
/// message. We follow pointers (with a small loop budget so a
/// cyclic packet can't hang the parser) but only record the
/// "end offset in the original stream" up to the first pointer
/// follow, so the caller's record-walking arithmetic still lines
/// up.
fn read_name(msg: &[u8], start: usize) -> Option<(heapless::String<128>, usize)> {
    let mut out: heapless::String<128> = heapless::String::new();
    let mut off = start;
    let mut end_off: Option<usize> = None;
    let mut hops: u8 = 0;
    loop {
        if off >= msg.len() {
            return None;
        }
        let b = msg[off];
        if b == 0 {
            return Some((out, end_off.unwrap_or(off + 1)));
        }
        if b & 0xC0 == 0xC0 {
            if off + 1 >= msg.len() {
                return None;
            }
            hops = hops.checked_add(1)?;
            if hops > 8 {
                return None;
            }
            if end_off.is_none() {
                end_off = Some(off + 2);
            }
            off = (usize::from(b & 0x3F) << 8) | usize::from(msg[off + 1]);
            continue;
        }
        if b & 0xC0 != 0 {
            return None;
        }
        let len = b as usize;
        if off.checked_add(1 + len)? > msg.len() {
            return None;
        }
        if !out.is_empty() {
            out.push('.').ok()?;
        }
        for &ch in &msg[off + 1..off + 1 + len] {
            out.push(ch as char).ok()?;
        }
        off += 1 + len;
    }
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
        // `{:.1}` rounds to the nearest tenth, so `-59.95` lands at
        // either `-60.0` or `-59.9` depending on the IEEE 754
        // representation of the literal — both are acceptable for the
        // wire. Any other output would indicate a formatting bug.
        assert!(
            s.as_str() == "pitch=-60.0" || s.as_str() == "pitch=-59.9",
            "unexpected format: {s:?}"
        );
    }

    /// Build a synthetic mDNS response packet carrying exactly one
    /// answer: a TXT record at `<hostname>._stackchan._tcp.local`
    /// whose RDATA is the concatenation of the given key=value
    /// strings (each prefixed with its single-byte length).
    fn build_txt_response(hostname: &str, txt_strings: &[&str]) -> alloc::vec::Vec<u8> {
        let mut msg = alloc::vec::Vec::new();
        // Header: ID 0, flags QR=1 AA=1, qdcount=0, ancount=1.
        msg.extend_from_slice(&[
            0x00, 0x00, 0x84, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
        ]);
        // Answer NAME: <hostname>._stackchan._tcp.local.
        for label in [hostname, "_stackchan", "_tcp", "local"] {
            #[allow(clippy::cast_possible_truncation, reason = "test labels are short")]
            msg.push(label.len() as u8);
            msg.extend_from_slice(label.as_bytes());
        }
        msg.push(0x00); // root
        // TYPE 16 (TXT), CLASS 1 (IN).
        msg.extend_from_slice(&[0x00, 0x10, 0x00, 0x01]);
        // TTL 120.
        msg.extend_from_slice(&[0x00, 0x00, 0x00, 0x78]);
        // RDLENGTH + RDATA.
        let mut rdata = alloc::vec::Vec::new();
        for s in txt_strings {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "test TXT strings are short"
            )]
            rdata.push(s.len() as u8);
            rdata.extend_from_slice(s.as_bytes());
        }
        #[allow(
            clippy::cast_possible_truncation,
            reason = "test rdata length is bounded by short inputs"
        )]
        let rdlen = rdata.len() as u16;
        msg.extend_from_slice(&rdlen.to_be_bytes());
        msg.extend_from_slice(&rdata);
        msg
    }

    #[test]
    fn parse_response_pose_extracts_yaw_and_pitch() {
        let msg = build_txt_response(
            "kitchen-cat",
            &["txtvers=1", "kai=1", "yaw=12.3", "pitch=-4.5"],
        );
        let pose = parse_response_pose(&msg, "kitchen-cat").unwrap();
        assert!((pose.pan_deg - 12.3).abs() < 0.01);
        assert!((pose.tilt_deg - -4.5).abs() < 0.01);
    }

    #[test]
    fn parse_response_pose_is_case_insensitive_on_hostname() {
        let msg = build_txt_response("Kitchen-Cat", &["yaw=1.0", "pitch=2.0"]);
        assert!(parse_response_pose(&msg, "kitchen-cat").is_some());
        assert!(parse_response_pose(&msg, "KITCHEN-CAT").is_some());
    }

    #[test]
    fn parse_response_pose_rejects_wrong_hostname() {
        let msg = build_txt_response("kitchen-cat", &["yaw=1.0", "pitch=2.0"]);
        assert!(parse_response_pose(&msg, "living-room-cat").is_none());
    }

    #[test]
    fn parse_response_pose_rejects_query_with_qr_clear() {
        // Same packet shape but flags byte 2 = 0x00 (QR cleared).
        let mut msg = build_txt_response("kitchen-cat", &["yaw=1.0", "pitch=2.0"]);
        msg[2] = 0x00;
        assert!(parse_response_pose(&msg, "kitchen-cat").is_none());
    }

    #[test]
    fn parse_response_pose_returns_none_when_pose_keys_partial() {
        // Only yaw, no pitch — extractor returns None because both
        // are required to drive a useful follower update.
        let msg = build_txt_response("kitchen-cat", &["yaw=12.3"]);
        assert!(parse_response_pose(&msg, "kitchen-cat").is_none());
    }

    #[test]
    fn parse_response_pose_ignores_unrelated_keys() {
        let msg = build_txt_response(
            "kitchen-cat",
            &[
                "txtvers=1",
                "version=0.2.0",
                "path=/",
                "mcp=/mcp",
                "kai=1",
                "yaw=10.0",
                "pitch=0.0",
            ],
        );
        let pose = parse_response_pose(&msg, "kitchen-cat").unwrap();
        assert!((pose.pan_deg - 10.0).abs() < 0.01);
        assert!(pose.tilt_deg.abs() < 0.01);
    }

    #[test]
    fn parse_response_pose_rejects_too_short_msg() {
        assert!(parse_response_pose(&[0_u8; 4], "x").is_none());
        assert!(parse_response_pose(&[], "x").is_none());
    }

    #[test]
    fn parse_response_pose_rejects_no_answers() {
        let mut msg = build_txt_response("kitchen-cat", &["yaw=1.0", "pitch=2.0"]);
        // Zero out the answer count.
        msg[6] = 0;
        msg[7] = 0;
        assert!(parse_response_pose(&msg, "kitchen-cat").is_none());
    }

    #[test]
    fn read_name_handles_simple_qname() {
        // "kitchen-cat._stackchan._tcp.local."
        let mut msg = alloc::vec::Vec::new();
        for label in ["kitchen-cat", "_stackchan", "_tcp", "local"] {
            #[allow(clippy::cast_possible_truncation, reason = "test labels are short")]
            msg.push(label.len() as u8);
            msg.extend_from_slice(label.as_bytes());
        }
        msg.push(0x00);
        let (name, after) = read_name(&msg, 0).unwrap();
        assert_eq!(name.as_str(), "kitchen-cat._stackchan._tcp.local");
        assert_eq!(after, msg.len());
    }

    #[test]
    fn read_name_follows_compression_pointer() {
        // Build "_stackchan._tcp.local." at offset 0, then a
        // separate name "kitchen-cat" + compression pointer back
        // to offset 0.
        let mut msg = alloc::vec::Vec::new();
        // Offset 0: "_stackchan._tcp.local."
        for label in ["_stackchan", "_tcp", "local"] {
            #[allow(clippy::cast_possible_truncation, reason = "test labels are short")]
            msg.push(label.len() as u8);
            msg.extend_from_slice(label.as_bytes());
        }
        msg.push(0x00);
        let second_start = msg.len();
        // "kitchen-cat" + pointer to offset 0.
        msg.push(11);
        msg.extend_from_slice(b"kitchen-cat");
        msg.push(0xC0);
        msg.push(0x00);
        let (name, after) = read_name(&msg, second_start).unwrap();
        assert_eq!(name.as_str(), "kitchen-cat._stackchan._tcp.local");
        // `after` is the offset just past the 2-byte pointer in
        // the original stream — NOT past the followed labels.
        assert_eq!(after, msg.len());
    }

    #[test]
    fn read_name_rejects_pointer_loop() {
        // A 2-byte pointer at offset 0 that points back to offset
        // 0 — a one-cycle loop. Must terminate via the hop budget,
        // not hang.
        let msg = [0xC0_u8, 0x00];
        assert!(read_name(&msg, 0).is_none());
    }

    /// Build a response with a filler `A` record before the TXT
    /// to exercise the answer-walking loop's `off = rdata_end`
    /// advance. Mirrors the shape of a real kai announcement
    /// (PTR + SRV + TXT + A in order; TXT is the third answer)
    /// without reimplementing every record type — one preceding
    /// record is enough to catch a fixed-header miscount.
    fn build_txt_response_with_filler(hostname: &str, txt_strings: &[&str]) -> alloc::vec::Vec<u8> {
        let mut msg = alloc::vec::Vec::new();
        // Header: QR=1 AA=1, ancount=2.
        msg.extend_from_slice(&[
            0x00, 0x00, 0x84, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00,
        ]);
        // Filler answer #1: A `filler.local.` → 1.2.3.4.
        for label in ["filler", "local"] {
            #[allow(clippy::cast_possible_truncation, reason = "test labels are short")]
            msg.push(label.len() as u8);
            msg.extend_from_slice(label.as_bytes());
        }
        msg.push(0x00);
        msg.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // type A, class IN
        msg.extend_from_slice(&[0x00, 0x00, 0x00, 0x78]); // TTL
        msg.extend_from_slice(&[0x00, 0x04]); // rdlength = 4
        msg.extend_from_slice(&[1, 2, 3, 4]); // a.b.c.d
        // Target answer: TXT `<hostname>._stackchan._tcp.local.`.
        for label in [hostname, "_stackchan", "_tcp", "local"] {
            #[allow(clippy::cast_possible_truncation, reason = "test labels are short")]
            msg.push(label.len() as u8);
            msg.extend_from_slice(label.as_bytes());
        }
        msg.push(0x00);
        msg.extend_from_slice(&[0x00, 0x10, 0x00, 0x01]); // type TXT
        msg.extend_from_slice(&[0x00, 0x00, 0x00, 0x78]); // TTL
        let mut rdata = alloc::vec::Vec::new();
        for s in txt_strings {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "test TXT strings are short"
            )]
            rdata.push(s.len() as u8);
            rdata.extend_from_slice(s.as_bytes());
        }
        #[allow(
            clippy::cast_possible_truncation,
            reason = "test rdata length is bounded by short inputs"
        )]
        let rdlen = rdata.len() as u16;
        msg.extend_from_slice(&rdlen.to_be_bytes());
        msg.extend_from_slice(&rdata);
        msg
    }

    #[test]
    fn parse_response_pose_finds_txt_after_filler_answer() {
        // Real kai announcements emit four answers (PTR + SRV +
        // TXT + A) so the parser must skip past preceding records
        // before reaching the TXT. A miscount in the
        // `off = rdata_end` advance would pass every single-
        // answer test but fail on every live announcement.
        let msg = build_txt_response_with_filler(
            "kitchen-cat",
            &["txtvers=1", "kai=1", "yaw=7.5", "pitch=-2.5"],
        );
        let pose = parse_response_pose(&msg, "kitchen-cat").unwrap();
        assert!((pose.pan_deg - 7.5).abs() < 0.01);
        assert!((pose.tilt_deg + 2.5).abs() < 0.01);
    }

    #[test]
    fn parse_response_pose_keeps_first_valid_value_on_duplicate_key() {
        // A malformed second entry must not clobber a valid
        // first reading — `yaw = v.parse().ok()` would let
        // `yaw=bad` overwrite `Some(12.3)` with `None` and
        // surface as a follower stall on otherwise-good leader
        // traffic.
        let msg = build_txt_response("kitchen-cat", &["yaw=12.3", "yaw=bad", "pitch=4.5"]);
        let pose = parse_response_pose(&msg, "kitchen-cat").unwrap();
        assert!((pose.pan_deg - 12.3).abs() < 0.01);
        assert!((pose.tilt_deg - 4.5).abs() < 0.01);
    }
}
