//! mDNS responder — hostname A records plus full DNS-SD service
//! advertising for `_stackchan._tcp.local.`.
//!
//! Joins the IPv4 mDNS multicast group `224.0.0.251:5353` and emits an
//! unsolicited announcement on every `WIFI_LINK_WATCH::Connected`
//! transition so phones / laptops / Bonjour browsers pick up the
//! device without an explicit query. Inbound A queries for
//! `<hostname>.local` and PTR queries for the service type are
//! answered on demand.
//!
//! ## Records advertised
//!
//! - `A` `<hostname>.local.` → station IPv4 lease
//! - `PTR` `_stackchan._tcp.local.` → `<hostname>._stackchan._tcp.local.`
//! - `SRV` `<hostname>._stackchan._tcp.local.` → priority 0 weight 0
//!   port 80 target `<hostname>.local.`
//! - `TXT` `<hostname>._stackchan._tcp.local.` → `txtvers=1`,
//!   `version=<crate>`, `path=/`, `mcp=/mcp`, `kai=1`,
//!   `yaw=<deg>`, `pitch=<deg>`
//!
//! The `kai=1` key is the variant marker — meganetaaan-line clients
//! ignore it, kai-aware clients use it to gate access to extension
//! endpoints (MCP, palette, soliloquy config). The other keys mirror
//! the upstream stackchan / m5stack-avatar convention so a generic
//! Bonjour browser shows the device alongside upstream units.
//!
//! `yaw` and `pitch` carry the live commanded head pose in degrees
//! (one decimal place, matches the meganetaaan `mimic_main` mod's
//! advertisement convention so a kai unit can lead a mixed-firmware
//! mimic stack). The pose advertisement runs as a periodic publisher
//! racing against the query loop. The throttling policy (publish
//! interval, deadband, heartbeat) lives in
//! [`stackchan_net::mdns_pose::PoseAnnouncer`] so it can be
//! host-tested without an esp toolchain.

use alloc::string::String;

use embassy_futures::select::{Either, select};
use embassy_net::Stack;
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant as EmbassyInstant, Timer};
use stackchan_core::Pose;
use stackchan_net::mdns_pose::{
    POSE_PUBLISH_MIN_INTERVAL_MS, PoseAnnouncer, format_pose_kv, parse_response_pose,
};

use super::snapshot;
use super::wifi::{WIFI_LINK_WATCH, WifiLinkState};

/// Latest-wins pose lifted from a leader's mDNS TXT record.
///
/// Set by `serve_loop` whenever an inbound multicast packet
/// carries a TXT record matching the configured
/// `behavior.follower_leader_hostname`. Drained by the follower
/// task ([`crate::net::mdns_follower::follower_task`]) which
/// fires a `RemoteCommand::LookAt` hold so the local head tracks
/// the leader without an HTTP round-trip.
///
/// Latest-wins semantics match the use case: a follower that
/// missed an intermediate update should still snap to the
/// freshest pose on the next tick rather than catching up
/// through a backlog.
pub static LEADER_POSE_SIGNAL: Signal<CriticalSectionRawMutex, Pose> = Signal::new();

/// IPv4 mDNS multicast group + port.
const MDNS_MULTICAST: embassy_net::IpAddress =
    embassy_net::IpAddress::Ipv4(embassy_net::Ipv4Address::new(224, 0, 0, 251));
/// Standard mDNS port.
const MDNS_PORT: u16 = 5353;

/// TTL on advertised records, in seconds. Two minutes mirrors the
/// canonical Avahi / Bonjour default and is short enough that the
/// network notices when the device leaves.
const MDNS_TTL_SECS: u32 = 120;

/// Maximum DNS message we'll accept or build. The full announcement
/// (PTR + SRV + TXT + A with no name compression) lands around
/// 320 bytes for a 9-character hostname; 512 is the classic DNS UDP
/// limit. Layout cost is roughly `6 × hostname_len + ~220` bytes
/// with the current TXT set, so the effective hostname ceiling is
/// ~48 characters — callers are expected to use short names, and
/// `build_announcement` returns `None` gracefully if the buffer
/// would overflow.
const MAX_DNS_BYTES: usize = 512;

/// HTTP port advertised in the SRV record. Matches `HTTP_PORT` in
/// the firmware's HTTP module — kept in sync manually because the
/// dependency direction is `mdns → http`, not the reverse.
const ADVERTISED_HTTP_PORT: u16 = 80;

/// DNS-SD service type for stackchan units. Matches the meganetaaan
/// upstream convention so mixed kai + upstream fleets browse together.
const SERVICE_LABELS: [&[u8]; 3] = [b"_stackchan", b"_tcp", b"local"];

/// Embassy task — owns one UDP socket on the mDNS multicast group.
/// Rebinds on each `Connected` transition so a Wi-Fi reconnect
/// doesn't strand the listener on a stale lease.
#[embassy_executor::task]
pub async fn mdns_task(stack: Stack<'static>, hostname: String, leader_hostname: String) -> ! {
    if hostname.is_empty() {
        defmt::info!("mdns: empty hostname, idle");
        park_forever().await;
    }

    // Acquire the receiver before any await; `Watch::receiver()`
    // initialises the receiver's generation counter to the watch's
    // *current* generation, so a publish that lands between task
    // spawn and this line would be invisible to a subsequent
    // `.changed().await`. The first read uses `.get()` instead,
    // which returns the stored value (or waits for the first
    // publish) regardless of when this receiver was created.
    let Some(mut link) = WIFI_LINK_WATCH.receiver() else {
        defmt::error!("mdns: WIFI_LINK_WATCH receiver slot exhausted; parking");
        park_forever().await;
    };

    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_buf = [0u8; MAX_DNS_BYTES];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_buf = [0u8; MAX_DNS_BYTES];

    let mut state = link.get().await;
    loop {
        if !matches!(state, WifiLinkState::Connected) {
            state = link.changed().await;
            continue;
        }

        let mut socket =
            UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
        if let Err(e) = socket.bind(MDNS_PORT) {
            defmt::warn!("mdns: bind failed ({:?})", e);
            Timer::after(Duration::from_secs(5)).await;
            state = link.changed().await;
            continue;
        }
        if let Err(e) = stack.join_multicast_group(MDNS_MULTICAST) {
            defmt::warn!("mdns: multicast join failed ({:?}); responder idle", e);
            socket.close();
            // Without multicast we can't respond to queries; fall
            // back to waiting for the next link transition rather
            // than busy-looping.
            Timer::after(Duration::from_secs(60)).await;
            state = link.changed().await;
            continue;
        }

        let Some(our_ip) = stack.config_v4().map(|c| c.address.address()) else {
            defmt::warn!("mdns: no IPv4 lease yet; will retry");
            socket.close();
            Timer::after(Duration::from_secs(2)).await;
            state = link.changed().await;
            continue;
        };

        defmt::info!(
            "mdns: announcing {=str}.local at {=u8}.{=u8}.{=u8}.{=u8} (service _stackchan._tcp port {=u16})",
            hostname.as_str(),
            our_ip.octets()[0],
            our_ip.octets()[1],
            our_ip.octets()[2],
            our_ip.octets()[3],
            ADVERTISED_HTTP_PORT,
        );

        // Send unsolicited announcement once.
        let initial_pose = current_pose();
        send_announcement(&socket, &hostname, our_ip, initial_pose).await;
        let mut announcer = PoseAnnouncer::new();
        announcer.record_publish(initial_pose, EmbassyInstant::now().as_millis());

        // Serve queries + periodic pose updates until the link drops
        // or anything errors out.
        serve_loop(&socket, &hostname, &leader_hostname, our_ip, &mut announcer).await;

        // Best-effort multicast-group leave: serve_loop returned
        // because the link dropped or the socket errored, so the
        // group membership is moot. A failure here just means the
        // stack will reject the next join with `AlreadyExists` until
        // the link transition forces a teardown.
        let _ = stack.leave_multicast_group(MDNS_MULTICAST);
        socket.close();
        // serve_loop returned (link dropped or socket failure) —
        // wait for the next state transition before re-binding.
        state = link.changed().await;
    }
}

/// Read the current commanded head pose from the avatar snapshot.
/// Mirrors what the HTTP `GET /state` route reads — non-destructive,
/// no `Signal::try_take` race against the head task.
fn current_pose() -> Pose {
    snapshot::read().head_pose
}

/// Park forever — used by the empty-hostname idle path.
async fn park_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}

/// Listen for queries while periodically publishing fresh pose
/// announcements. Both branches share the socket; the `select` race
/// guarantees the periodic publisher fires even when no query is
/// inbound, while still answering queries with sub-millisecond
/// latency between timer ticks.
async fn serve_loop(
    socket: &UdpSocket<'_>,
    hostname: &str,
    leader_hostname: &str,
    our_ip: embassy_net::Ipv4Address,
    announcer: &mut PoseAnnouncer,
) {
    let mut buf = [0u8; MAX_DNS_BYTES];
    loop {
        let race = select(
            socket.recv_from(&mut buf),
            Timer::after(Duration::from_millis(POSE_PUBLISH_MIN_INTERVAL_MS)),
        )
        .await;

        match race {
            Either::First(Ok((n, peer))) => {
                if n < 12 {
                    continue;
                }
                let kind = classify_query(&buf[..n], hostname);
                if matches!(kind, QueryKind::None) {
                    // Not a query directed at us. Could still be
                    // a leader's TXT announcement worth following.
                    if !leader_hostname.is_empty()
                        && let Some(pose) = parse_response_pose(&buf[..n], leader_hostname)
                    {
                        LEADER_POSE_SIGNAL.signal(pose);
                    }
                    continue;
                }
                // Both A and service-type queries are answered with
                // the same announcement payload — building one record
                // set keeps the wire surface uniform and avoids drift
                // between the unsolicited and reactive paths.
                let mut resp = [0u8; MAX_DNS_BYTES];
                let resp_id = u16::from_be_bytes([buf[0], buf[1]]);
                let pose = current_pose();
                let Some(len) = build_announcement(&mut resp, resp_id, hostname, our_ip, pose)
                else {
                    continue;
                };

                // Multicast peer is the standard mDNS path; unicast
                // clients also exist but multicasting reaches everyone
                // subscribed.
                let target = embassy_net::IpEndpoint::new(MDNS_MULTICAST, MDNS_PORT);
                if let Err(e) = socket.send_to(&resp[..len], target).await {
                    defmt::warn!(
                        "mdns: send response to {} failed ({:?})",
                        defmt::Debug2Format(&peer.endpoint.addr),
                        e,
                    );
                } else {
                    // A query response carries the current pose just
                    // like a periodic update, so stamp the announcer
                    // — otherwise a steady stream of queries would
                    // leave the announcer thinking pose is stale and
                    // emit redundant unsolicited multicasts right
                    // after each reactive answer.
                    announcer.record_publish(pose, EmbassyInstant::now().as_millis());
                }
            }
            Either::First(Err(e)) => {
                defmt::warn!("mdns: recv error ({:?})", e);
                return;
            }
            Either::Second(()) => {
                let pose = current_pose();
                if !announcer.should_publish(pose, EmbassyInstant::now().as_millis()) {
                    continue;
                }
                send_announcement(socket, hostname, our_ip, pose).await;
                // Snap the publish stamp *after* the send completes,
                // matching the query-response branch above. The two
                // branches now agree on what "publish window starts"
                // means; otherwise the periodic branch would record
                // the start slightly earlier than the actual send and
                // its throttle window would run slightly short.
                announcer.record_publish(pose, EmbassyInstant::now().as_millis());
            }
        }
    }
}

/// Send one unsolicited mDNS announcement so caches pick us up
/// without waiting for a query.
async fn send_announcement(
    socket: &UdpSocket<'_>,
    hostname: &str,
    our_ip: embassy_net::Ipv4Address,
    pose: Pose,
) {
    let mut resp = [0u8; MAX_DNS_BYTES];
    let Some(len) = build_announcement(&mut resp, 0, hostname, our_ip, pose) else {
        return;
    };
    let target = embassy_net::IpEndpoint::new(MDNS_MULTICAST, MDNS_PORT);
    if let Err(e) = socket.send_to(&resp[..len], target).await {
        defmt::warn!("mdns: announce send failed ({:?})", e);
    }
}

/// Query classification — what (if anything) the inbound message is
/// asking about. Used to skip messages aimed at other hosts without
/// allocating a response buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryKind {
    /// Not for us, or malformed.
    None,
    /// `A`-record query for `<hostname>.local`.
    HostA,
    /// `PTR` query for `_stackchan._tcp.local`.
    ServicePtr,
}

/// Classify the first question in `msg`. Tolerant: ignores any
/// further questions and any malformed bits past the first
/// answer-eligible question. Returns the most specific match —
/// `HostA` and `ServicePtr` are mutually exclusive given different
/// QNAMEs and QTYPEs.
fn classify_query(msg: &[u8], hostname: &str) -> QueryKind {
    if msg.len() < 12 {
        return QueryKind::None;
    }
    let qdcount = u16::from_be_bytes([msg[4], msg[5]]);
    if qdcount == 0 {
        return QueryKind::None;
    }
    let Some((qname, after_name)) = read_qname(msg, 12) else {
        return QueryKind::None;
    };
    if after_name + 4 > msg.len() {
        return QueryKind::None;
    }
    let qtype = u16::from_be_bytes([msg[after_name], msg[after_name + 1]]);

    // `A` (1) or `ANY` (255) query for our hostname. Avahi resolvers
    // sometimes follow an SRV resolution with an `ANY` for the host
    // name; treating `ANY` as host-A here keeps `avahi-browse -r`
    // resolves from timing out.
    if (qtype == 1 || qtype == 255) && matches_local_hostname(&qname, hostname) {
        return QueryKind::HostA;
    }
    // `PTR` query for our service type. `ANY` is also a valid mDNS
    // query type; treat it as service-type if the name matches.
    if (qtype == 12 || qtype == 255) && qname.eq_ignore_ascii_case("_stackchan._tcp.local") {
        return QueryKind::ServicePtr;
    }
    QueryKind::None
}

/// Walk DNS labels starting at `off`, returning the joined dotted
/// name and the offset just past the final root-label byte. Bails
/// on compression pointers — mDNS queries rarely use them and we
/// don't need to handle them for hostname-only matching.
fn read_qname(msg: &[u8], mut off: usize) -> Option<(heapless::String<128>, usize)> {
    let mut out: heapless::String<128> = heapless::String::new();
    loop {
        if off >= msg.len() {
            return None;
        }
        let len = msg[off] as usize;
        if len == 0 {
            return Some((out, off + 1));
        }
        if len & 0xC0 != 0 {
            // Compression pointer; bail.
            return None;
        }
        if off + 1 + len > msg.len() {
            return None;
        }
        if !out.is_empty() {
            out.push('.').ok()?;
        }
        for &b in &msg[off + 1..off + 1 + len] {
            out.push(b as char).ok()?;
        }
        off += 1 + len;
    }
}

/// Case-insensitive match of `qname` against `<hostname>.local`.
fn matches_local_hostname(qname: &str, hostname: &str) -> bool {
    let mut parts = qname.splitn(2, '.');
    let host = parts.next().unwrap_or("");
    let tld = parts.next().unwrap_or("");
    host.eq_ignore_ascii_case(hostname) && tld.eq_ignore_ascii_case("local")
}

/// Encode a full mDNS announcement (PTR + SRV + TXT + A) into `out`.
/// Returns `None` if the buffer is too small.
///
/// All four answers reference the same hostname, but no DNS name
/// compression is performed — at our message sizes the duplication
/// is well under the 512-byte budget and avoids the pointer-bookkeeping
/// that compression requires.
///
/// `pose` is encoded into the TXT record as the live `yaw` / `pitch`
/// keys — see [`write_txt_answer`].
fn build_announcement(
    out: &mut [u8; MAX_DNS_BYTES],
    transaction_id: u16,
    hostname: &str,
    our_ip: embassy_net::Ipv4Address,
    pose: Pose,
) -> Option<usize> {
    // Header: response, authoritative, ANCOUNT=4.
    out[0..2].copy_from_slice(&transaction_id.to_be_bytes());
    out[2..4].copy_from_slice(&0x8400u16.to_be_bytes()); // QR=1, AA=1
    out[4..6].copy_from_slice(&0u16.to_be_bytes()); // qdcount
    out[6..8].copy_from_slice(&4u16.to_be_bytes()); // ancount = PTR + SRV + TXT + A
    out[8..10].copy_from_slice(&0u16.to_be_bytes()); // nscount
    out[10..12].copy_from_slice(&0u16.to_be_bytes()); // arcount

    let mut off = 12;
    off = write_ptr_answer(out, off, hostname)?;
    off = write_srv_answer(out, off, hostname, ADVERTISED_HTTP_PORT)?;
    off = write_txt_answer(out, off, hostname, pose)?;
    off = write_a_answer(out, off, hostname, our_ip)?;
    Some(off)
}

/// PTR answer: name `_stackchan._tcp.local.`, RDATA = instance name.
fn write_ptr_answer(
    out: &mut [u8; MAX_DNS_BYTES],
    mut off: usize,
    hostname: &str,
) -> Option<usize> {
    // Owner name = service type (no cache-flush bit on PTR; multiple
    // service instances may share the same PTR owner).
    off = write_name(out, off, &SERVICE_LABELS)?;
    off = write_record_header(out, off, 12 /* PTR */, false, MDNS_TTL_SECS)?;

    // RDLENGTH placeholder; backfill once we know the encoded size.
    let rdlen_off = off;
    off += 2;
    let rdata_start = off;

    // RDATA: full instance name `<hostname>._stackchan._tcp.local.`
    let instance_labels: [&[u8]; 4] = [
        hostname.as_bytes(),
        SERVICE_LABELS[0],
        SERVICE_LABELS[1],
        SERVICE_LABELS[2],
    ];
    off = write_name(out, off, &instance_labels)?;

    let rdlen = u16::try_from(off - rdata_start).ok()?;
    out.get_mut(rdlen_off..rdlen_off + 2)?
        .copy_from_slice(&rdlen.to_be_bytes());
    Some(off)
}

/// SRV answer: name `<instance>`, RDATA = priority/weight/port/target.
fn write_srv_answer(
    out: &mut [u8; MAX_DNS_BYTES],
    mut off: usize,
    hostname: &str,
    port: u16,
) -> Option<usize> {
    let instance_labels: [&[u8]; 4] = [
        hostname.as_bytes(),
        SERVICE_LABELS[0],
        SERVICE_LABELS[1],
        SERVICE_LABELS[2],
    ];
    off = write_name(out, off, &instance_labels)?;
    off = write_record_header(out, off, 33 /* SRV */, true, MDNS_TTL_SECS)?;

    let rdlen_off = off;
    off += 2;
    let rdata_start = off;

    // priority + weight (both zero — single instance, no preference).
    out.get_mut(off..off + 2)?
        .copy_from_slice(&0u16.to_be_bytes());
    off += 2;
    out.get_mut(off..off + 2)?
        .copy_from_slice(&0u16.to_be_bytes());
    off += 2;
    out.get_mut(off..off + 2)?
        .copy_from_slice(&port.to_be_bytes());
    off += 2;
    // Target: `<hostname>.local.`
    let host_labels: [&[u8]; 2] = [hostname.as_bytes(), SERVICE_LABELS[2]];
    off = write_name(out, off, &host_labels)?;

    let rdlen = u16::try_from(off - rdata_start).ok()?;
    out.get_mut(rdlen_off..rdlen_off + 2)?
        .copy_from_slice(&rdlen.to_be_bytes());
    Some(off)
}

/// TXT answer: name `<instance>`, RDATA = length-prefixed strings.
///
/// Includes the live commanded head pose as `yaw=<deg>` and
/// `pitch=<deg>` (one decimal). Followers in a mimic stack lift these
/// straight off the TXT without needing the HTTP plane.
fn write_txt_answer(
    out: &mut [u8; MAX_DNS_BYTES],
    mut off: usize,
    hostname: &str,
    pose: Pose,
) -> Option<usize> {
    let instance_labels: [&[u8]; 4] = [
        hostname.as_bytes(),
        SERVICE_LABELS[0],
        SERVICE_LABELS[1],
        SERVICE_LABELS[2],
    ];
    off = write_name(out, off, &instance_labels)?;
    off = write_record_header(out, off, 16 /* TXT */, true, MDNS_TTL_SECS)?;

    let rdlen_off = off;
    off += 2;
    let rdata_start = off;

    // DNS-SD TXT keys. `txtvers` is the conventional first key per
    // RFC 6763 §6.7. `version` mirrors upstream stackchan TXT for
    // browser parity. `kai=1` is the variant marker that gates
    // kai-only routes (MCP / palette / behavior config).
    off = write_txt_string(out, off, b"txtvers=1")?;
    off = write_txt_string(
        out,
        off,
        concat!("version=", env!("CARGO_PKG_VERSION")).as_bytes(),
    )?;
    off = write_txt_string(out, off, b"path=/")?;
    off = write_txt_string(out, off, b"mcp=/mcp")?;
    off = write_txt_string(out, off, b"kai=1")?;

    // Pose keys — `yaw` / `pitch` follow meganetaaan `mimic_main`'s
    // naming so a kai unit can lead a mixed-firmware mimic stack
    // without a translation layer on the follower side.
    let yaw_kv = format_pose_kv("yaw", pose.pan_deg)?;
    off = write_txt_string(out, off, yaw_kv.as_bytes())?;
    let pitch_kv = format_pose_kv("pitch", pose.tilt_deg)?;
    off = write_txt_string(out, off, pitch_kv.as_bytes())?;

    let rdlen = u16::try_from(off - rdata_start).ok()?;
    out.get_mut(rdlen_off..rdlen_off + 2)?
        .copy_from_slice(&rdlen.to_be_bytes());
    Some(off)
}

/// A answer: name `<hostname>.local.`, RDATA = IPv4 (4 bytes).
fn write_a_answer(
    out: &mut [u8; MAX_DNS_BYTES],
    mut off: usize,
    hostname: &str,
    our_ip: embassy_net::Ipv4Address,
) -> Option<usize> {
    let host_labels: [&[u8]; 2] = [hostname.as_bytes(), SERVICE_LABELS[2]];
    off = write_name(out, off, &host_labels)?;
    off = write_record_header(out, off, 1 /* A */, true, MDNS_TTL_SECS)?;

    out.get_mut(off..off + 2)?
        .copy_from_slice(&4u16.to_be_bytes());
    off += 2;
    out.get_mut(off..off + 4)?.copy_from_slice(&our_ip.octets());
    off += 4;
    Some(off)
}

/// Encode TYPE(2) + CLASS(2) + TTL(4) — the fixed-width prefix of
/// every record before its variable-length RDATA. `cache_flush`
/// sets the high bit on CLASS for "this answer replaces any prior
/// record" (mDNS unique-record signal).
fn write_record_header(
    out: &mut [u8; MAX_DNS_BYTES],
    mut off: usize,
    rrtype: u16,
    cache_flush: bool,
    ttl: u32,
) -> Option<usize> {
    out.get_mut(off..off + 2)?
        .copy_from_slice(&rrtype.to_be_bytes());
    off += 2;
    let class = if cache_flush { 0x8001 } else { 0x0001 };
    out.get_mut(off..off + 2)?
        .copy_from_slice(&u16::to_be_bytes(class));
    off += 2;
    out.get_mut(off..off + 4)?
        .copy_from_slice(&ttl.to_be_bytes());
    off += 4;
    Some(off)
}

/// Write a sequence of labels followed by the root-label terminator
/// (zero byte). Each label must be 1..=63 bytes per DNS limits.
fn write_name(out: &mut [u8; MAX_DNS_BYTES], mut off: usize, labels: &[&[u8]]) -> Option<usize> {
    for label in labels {
        off = write_label(out, off, label)?;
    }
    *out.get_mut(off)? = 0;
    Some(off + 1)
}

/// Write a single label (length byte + bytes). Returns the offset
/// just past the label, or `None` if the buffer would overflow or
/// the label exceeds the 63-byte DNS limit.
fn write_label(out: &mut [u8; MAX_DNS_BYTES], off: usize, label: &[u8]) -> Option<usize> {
    if label.is_empty() || label.len() > 63 {
        return None;
    }
    if off + 1 + label.len() > out.len() {
        return None;
    }
    out[off] = u8::try_from(label.len()).ok()?;
    out[off + 1..off + 1 + label.len()].copy_from_slice(label);
    Some(off + 1 + label.len())
}

/// Write one TXT-record string (length byte + UTF-8 bytes). Each
/// string is capped at 255 bytes — well above any kv-pair we emit.
fn write_txt_string(out: &mut [u8; MAX_DNS_BYTES], off: usize, s: &[u8]) -> Option<usize> {
    if s.len() > 255 {
        return None;
    }
    if off + 1 + s.len() > out.len() {
        return None;
    }
    out[off] = u8::try_from(s.len()).ok()?;
    out[off + 1..off + 1 + s.len()].copy_from_slice(s);
    Some(off + 1 + s.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_local_hostname_is_case_insensitive() {
        assert!(matches_local_hostname("stackchan.local", "stackchan"));
        assert!(matches_local_hostname("StackChan.LOCAL", "stackchan"));
        assert!(!matches_local_hostname("stackchan.com", "stackchan"));
        assert!(!matches_local_hostname("not-us.local", "stackchan"));
    }

    /// Build a minimal DNS query asking for QNAME of QTYPE.
    fn build_query(qname_labels: &[&[u8]], qtype: u16) -> alloc::vec::Vec<u8> {
        let mut v: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        // Header: id=0, flags=0, qdcount=1, others=0.
        v.extend_from_slice(&0u16.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes());
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes());
        for l in qname_labels {
            v.push(u8::try_from(l.len()).unwrap());
            v.extend_from_slice(l);
        }
        v.push(0);
        v.extend_from_slice(&qtype.to_be_bytes());
        v.extend_from_slice(&1u16.to_be_bytes()); // CLASS=IN
        v
    }

    #[test]
    fn classify_query_recognises_host_a() {
        let q = build_query(&[b"stackchan", b"local"], 1);
        assert_eq!(classify_query(&q, "stackchan"), QueryKind::HostA);
    }

    #[test]
    fn classify_query_recognises_service_ptr() {
        let q = build_query(&[b"_stackchan", b"_tcp", b"local"], 12);
        assert_eq!(classify_query(&q, "stackchan"), QueryKind::ServicePtr);
    }

    #[test]
    fn classify_query_treats_any_as_service_when_name_matches() {
        let q = build_query(&[b"_stackchan", b"_tcp", b"local"], 255);
        assert_eq!(classify_query(&q, "stackchan"), QueryKind::ServicePtr);
    }

    #[test]
    fn classify_query_treats_any_for_hostname_as_host_a() {
        // Avahi's `-r` resolve sends ANY for the host name as a
        // follow-up after SRV resolution; without this match we'd
        // time out the resolve flow.
        let q = build_query(&[b"stackchan", b"local"], 255);
        assert_eq!(classify_query(&q, "stackchan"), QueryKind::HostA);
    }

    #[test]
    fn classify_query_rejects_other_hosts() {
        let q = build_query(&[b"someone-else", b"local"], 1);
        assert_eq!(classify_query(&q, "stackchan"), QueryKind::None);
    }

    #[test]
    fn classify_query_rejects_unknown_qtype_for_host() {
        // AAAA query for our host — we don't serve IPv6.
        let q = build_query(&[b"stackchan", b"local"], 28);
        assert_eq!(classify_query(&q, "stackchan"), QueryKind::None);
    }

    #[test]
    fn build_announcement_round_trip() {
        let ip = embassy_net::Ipv4Address::new(192, 168, 1, 42);
        let mut out = [0u8; MAX_DNS_BYTES];
        let n = build_announcement(&mut out, 0, "stackchan", ip, Pose::default()).unwrap();
        // Header: QR=1 AA=1, ANCOUNT=4 (PTR + SRV + TXT + A).
        assert_eq!(u16::from_be_bytes([out[2], out[3]]), 0x8400);
        assert_eq!(u16::from_be_bytes([out[6], out[7]]), 4);
        // The IP appears in the final A-record RDATA — last 4 bytes
        // of the announcement are the IPv4 octets.
        assert_eq!(&out[n - 4..n], &[192, 168, 1, 42]);
    }

    #[test]
    fn announcement_contains_service_type_and_kai_marker() {
        let ip = embassy_net::Ipv4Address::new(10, 0, 0, 1);
        let mut out = [0u8; MAX_DNS_BYTES];
        let n = build_announcement(&mut out, 0, "stackchan", ip, Pose::default()).unwrap();
        let bytes = &out[..n];
        // The service-type label sequence appears in the PTR owner.
        let needle = b"\x0a_stackchan\x04_tcp\x05local\x00";
        assert!(
            bytes.windows(needle.len()).any(|w| w == needle),
            "service type labels missing from announcement"
        );
        // `kai=1` TXT marker is what kai-aware clients gate on.
        let kai = b"kai=1";
        assert!(
            bytes.windows(kai.len()).any(|w| w == kai),
            "kai marker missing from TXT"
        );
    }

    #[test]
    fn announcement_srv_record_carries_http_port() {
        let ip = embassy_net::Ipv4Address::new(10, 0, 0, 1);
        let mut out = [0u8; MAX_DNS_BYTES];
        let n = build_announcement(&mut out, 0, "stackchan", ip, Pose::default()).unwrap();
        // SRV RDATA is priority(0,0) + weight(0,0) + port — search
        // for the literal HTTP port in the live announcement bytes
        // (scanning the full 512-byte buffer would risk a vacuous
        // hit in the zero-padded tail).
        let port_be = ADVERTISED_HTTP_PORT.to_be_bytes();
        assert!(
            out[..n].windows(2).any(|w| w == port_be),
            "advertised HTTP port not encoded in announcement"
        );
    }

    #[test]
    fn announcement_fits_in_buffer_for_long_hostname() {
        // 33-character hostname — comfortably under the 63-byte
        // single-label DNS cap and representative of real fleets.
        let host = "stackchan-abcdef0123456789-abcdef";
        assert_eq!(host.len(), 33);
        let ip = embassy_net::Ipv4Address::new(10, 0, 0, 1);
        let mut out = [0u8; MAX_DNS_BYTES];
        assert!(build_announcement(&mut out, 0, host, ip, Pose::default()).is_some());
    }

    #[test]
    fn announcement_includes_pose_keys() {
        let ip = embassy_net::Ipv4Address::new(10, 0, 0, 1);
        let mut out = [0u8; MAX_DNS_BYTES];
        let pose = Pose::new(12.3, -4.5);
        let n = build_announcement(&mut out, 0, "stackchan", ip, pose).unwrap();
        let bytes = &out[..n];
        let yaw_needle = b"yaw=12.3";
        assert!(
            bytes.windows(yaw_needle.len()).any(|w| w == yaw_needle),
            "yaw TXT key missing from announcement"
        );
        let pitch_needle = b"pitch=-4.5";
        assert!(
            bytes.windows(pitch_needle.len()).any(|w| w == pitch_needle),
            "pitch TXT key missing from announcement"
        );
    }
}
