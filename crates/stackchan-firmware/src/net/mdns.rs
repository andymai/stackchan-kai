//! Minimal hostname-only mDNS responder.
//!
//! Joins the IPv4 mDNS multicast group `224.0.0.251:5353`, listens
//! for `A` queries for `<hostname>.local`, and answers with the
//! station's current IPv4 lease. Sends one unsolicited announcement
//! per `WIFI_LINK_SIGNAL::Connected` transition so phones / laptops
//! pick up the device without an explicit query.
//!
//! No `PTR` / `SRV` / `TXT` records — this is the smallest useful
//! surface. A future revision can extend the same wire-format
//! encoder for service-type advertising.

use alloc::string::String;

use embassy_net::Stack;
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_time::{Duration, Timer};

use super::wifi::{WIFI_LINK_SIGNAL, WifiLinkState};

/// IPv4 mDNS multicast group + port.
const MDNS_MULTICAST: embassy_net::IpAddress =
    embassy_net::IpAddress::Ipv4(embassy_net::Ipv4Address::new(224, 0, 0, 251));
/// Standard mDNS port.
const MDNS_PORT: u16 = 5353;

/// TTL on advertised records, in seconds. Two minutes mirrors the
/// canonical Avahi / Bonjour default and is short enough that the
/// network notices when the device leaves.
const MDNS_TTL_SECS: u32 = 120;

/// Maximum DNS message we'll accept or build. Hostname-only A
/// records sit well under 256 bytes; a 512-byte cap matches the
/// classic DNS UDP limit and keeps stack alloc bounded.
const MAX_DNS_BYTES: usize = 512;

/// Embassy task — owns one UDP socket on the mDNS multicast group.
/// Rebinds on each `Connected` transition so a Wi-Fi reconnect
/// doesn't strand the listener on a stale lease.
#[embassy_executor::task]
pub async fn mdns_task(stack: Stack<'static>, hostname: String) -> ! {
    if hostname.is_empty() {
        defmt::info!("mdns: empty hostname, idle");
        park_forever().await;
    }

    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_buf = [0u8; MAX_DNS_BYTES];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_buf = [0u8; MAX_DNS_BYTES];

    loop {
        // Wait for a Connected link before binding the socket.
        // embassy-net needs the IPv4 address to encode A-record
        // answers, and join_multicast_group needs the link up.
        if !matches!(WIFI_LINK_SIGNAL.wait().await, WifiLinkState::Connected) {
            continue;
        }

        let mut socket =
            UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
        if let Err(e) = socket.bind(MDNS_PORT) {
            defmt::warn!("mdns: bind failed ({:?})", e);
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }
        if let Err(e) = stack.join_multicast_group(MDNS_MULTICAST) {
            defmt::warn!("mdns: multicast join failed ({:?}); responder idle", e);
            socket.close();
            // Without multicast we can't respond to queries; fall
            // back to waiting for the next link transition rather
            // than busy-looping.
            Timer::after(Duration::from_secs(60)).await;
            continue;
        }

        let Some(our_ip) = stack.config_v4().map(|c| c.address.address()) else {
            defmt::warn!("mdns: no IPv4 lease yet; will retry");
            socket.close();
            Timer::after(Duration::from_secs(2)).await;
            continue;
        };

        defmt::info!(
            "mdns: announcing {=str}.local at {=u8}.{=u8}.{=u8}.{=u8}",
            hostname.as_str(),
            our_ip.octets()[0],
            our_ip.octets()[1],
            our_ip.octets()[2],
            our_ip.octets()[3],
        );

        // Send unsolicited announcement once.
        send_announcement(&socket, &hostname, our_ip).await;

        // Serve queries until the link drops or anything errors out.
        serve_loop(&socket, &hostname, our_ip).await;

        let _ = stack.leave_multicast_group(MDNS_MULTICAST);
        socket.close();
    }
}

/// Park forever — used by the empty-hostname idle path.
async fn park_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}

/// Listen for queries; respond when one matches our hostname.
async fn serve_loop(socket: &UdpSocket<'_>, hostname: &str, our_ip: embassy_net::Ipv4Address) {
    let mut buf = [0u8; MAX_DNS_BYTES];
    loop {
        let (n, peer) = match socket.recv_from(&mut buf).await {
            Ok(p) => p,
            Err(e) => {
                defmt::warn!("mdns: recv error ({:?})", e);
                return;
            }
        };
        if n < 12 {
            continue;
        }

        if !matches_a_query(&buf[..n], hostname) {
            continue;
        }

        // Build a response with the same transaction-id (mDNS
        // typically uses 0 but we mirror the requester just in case).
        let resp_id = u16::from_be_bytes([buf[0], buf[1]]);
        let mut resp = [0u8; MAX_DNS_BYTES];
        let Some(len) = build_response(&mut resp, resp_id, hostname, our_ip) else {
            continue;
        };

        // Multicast peer is the standard mDNS path; unicast clients
        // also exist but multicasting reaches everyone subscribed.
        let target = embassy_net::IpEndpoint::new(MDNS_MULTICAST, MDNS_PORT);
        if let Err(e) = socket.send_to(&resp[..len], target).await {
            defmt::warn!(
                "mdns: send response to {} failed ({:?})",
                defmt::Debug2Format(&peer.endpoint.addr),
                e,
            );
        }
    }
}

/// Send one unsolicited mDNS announcement so caches pick us up
/// without waiting for a query.
async fn send_announcement(
    socket: &UdpSocket<'_>,
    hostname: &str,
    our_ip: embassy_net::Ipv4Address,
) {
    let mut resp = [0u8; MAX_DNS_BYTES];
    let Some(len) = build_response(&mut resp, 0, hostname, our_ip) else {
        return;
    };
    let target = embassy_net::IpEndpoint::new(MDNS_MULTICAST, MDNS_PORT);
    if let Err(e) = socket.send_to(&resp[..len], target).await {
        defmt::warn!("mdns: announce send failed ({:?})", e);
    }
}

/// True iff `msg` is a DNS query for `<hostname>.local` of type `A`.
/// Tolerant: ignores any further questions and any malformed bits
/// past the first answer-eligible question.
fn matches_a_query(msg: &[u8], hostname: &str) -> bool {
    if msg.len() < 12 {
        return false;
    }
    let qdcount = u16::from_be_bytes([msg[4], msg[5]]);
    if qdcount == 0 {
        return false;
    }
    // Walk the first question's name labels, comparing case-
    // insensitively against `<hostname>.local`.
    let Some((qname, after_name)) = read_qname(msg, 12) else {
        return false;
    };
    if after_name + 4 > msg.len() {
        return false;
    }
    let qtype = u16::from_be_bytes([msg[after_name], msg[after_name + 1]]);
    if qtype != 1
    /* A */
    {
        return false;
    }
    matches_local_hostname(&qname, hostname)
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

/// Encode an mDNS A-record response into `out`. Returns `None` if
/// the buffer is too small (shouldn't happen for hostname-only
/// schema-v1 names).
fn build_response(
    out: &mut [u8; MAX_DNS_BYTES],
    transaction_id: u16,
    hostname: &str,
    our_ip: embassy_net::Ipv4Address,
) -> Option<usize> {
    // Header.
    out[0..2].copy_from_slice(&transaction_id.to_be_bytes());
    out[2..4].copy_from_slice(&0x8400u16.to_be_bytes()); // QR=1, AA=1
    out[4..6].copy_from_slice(&0u16.to_be_bytes()); // qdcount
    out[6..8].copy_from_slice(&1u16.to_be_bytes()); // ancount
    out[8..10].copy_from_slice(&0u16.to_be_bytes()); // nscount
    out[10..12].copy_from_slice(&0u16.to_be_bytes()); // arcount

    let mut off = 12;
    // Answer name: <hostname>.local (length-prefixed labels + 0).
    off = write_label(out, off, hostname.as_bytes())?;
    off = write_label(out, off, b"local")?;
    *out.get_mut(off)? = 0;
    off += 1;

    // TYPE=A
    out.get_mut(off..off + 2)?
        .copy_from_slice(&1u16.to_be_bytes());
    off += 2;
    // CLASS=IN with cache-flush bit (0x8001).
    out.get_mut(off..off + 2)?
        .copy_from_slice(&0x8001u16.to_be_bytes());
    off += 2;
    // TTL.
    out.get_mut(off..off + 4)?
        .copy_from_slice(&MDNS_TTL_SECS.to_be_bytes());
    off += 4;
    // RDLENGTH = 4
    out.get_mut(off..off + 2)?
        .copy_from_slice(&4u16.to_be_bytes());
    off += 2;
    // RDATA (IPv4)
    out.get_mut(off..off + 4)?.copy_from_slice(&our_ip.octets());
    off += 4;

    Some(off)
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
    // SAFETY-EQUIVALENT NOTE: `label.len()` is bounded above by the
    // 63-byte check directly above, so the cast is in-range.
    out[off] = u8::try_from(label.len()).ok()?;
    out[off + 1..off + 1 + label.len()].copy_from_slice(label);
    Some(off + 1 + label.len())
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

    #[test]
    fn build_response_round_trip() {
        let ip = embassy_net::Ipv4Address::new(192, 168, 1, 42);
        let mut out = [0u8; MAX_DNS_BYTES];
        let n = build_response(&mut out, 0, "stackchan", ip).unwrap();
        // Header marks QR=1, AA=1.
        assert_eq!(u16::from_be_bytes([out[2], out[3]]), 0x8400);
        // ANCOUNT=1.
        assert_eq!(u16::from_be_bytes([out[6], out[7]]), 1);
        // RDATA at the end matches our IP.
        assert_eq!(&out[n - 4..n], &[192, 168, 1, 42]);
    }
}
