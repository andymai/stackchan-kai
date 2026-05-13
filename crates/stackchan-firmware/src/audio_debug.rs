//! UDP audio debug stream — tees `AUDIO_FRAME_PUBSUB` frames to a
//! configured LAN target so a host can listen with `aplay` / `nc`.
//!
//! Opt-in via `behavior.audio_debug_udp_target` in `STACKCHAN.RON`.
//! Empty target (the default) parks the task and consumes no socket
//! or pubsub slot.
//!
//! ## Wire format
//!
//! Each 20 ms frame from [`crate::audio::AUDIO_FRAME_PUBSUB`] becomes
//! one UDP datagram carrying [`crate::audio::AUDIO_FRAME_SAMPLES`] ×
//! `i16` little-endian samples (`AUDIO_FRAME_SAMPLES × 2 = 640` bytes
//! of payload). No header, no sequence number on the wire — drops
//! show up as audible gaps at the receiver.
//!
//! ## Receiving on a host
//!
//! ```sh
//! # 1) Bind to UDP port 5005 on the host LAN interface.
//! # 2) Decode raw little-endian s16 mono @ 16 kHz with aplay.
//! nc -lu 192.168.1.42 5005 | aplay -r 16000 -f S16_LE -c 1 -t raw
//! ```
//!
//! Set `behavior.audio_debug_udp_target = "192.168.1.42:5005"` in
//! `STACKCHAN.RON` and reboot.

use alloc::string::String;

use embassy_net::Stack;
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_sync::pubsub::WaitResult;

use crate::audio::{AUDIO_FRAME_PUBSUB, AUDIO_FRAME_SAMPLES};
use crate::net::wifi::{WIFI_LINK_WATCH, WifiLinkState};

/// Payload size of one UDP datagram: one [`crate::audio::AudioFrame`]
/// flattened to little-endian `i16` bytes.
const DATAGRAM_BYTES: usize = AUDIO_FRAME_SAMPLES * 2;

/// Audio-debug UDP task entry point.
///
/// Idles until Wi-Fi is up, then subscribes to
/// [`AUDIO_FRAME_PUBSUB`] and forwards each frame to `target` as a
/// raw UDP datagram. Re-opens the socket on link drops.
///
/// `target` is a `"host:port"` literal — `192.168.1.42:5005` etc.
/// Empty / unparseable values park the task.
#[embassy_executor::task]
pub async fn audio_debug_task(stack: Stack<'static>, target: String) -> ! {
    if target.is_empty() {
        defmt::info!("audio-debug: target empty, idle");
        park_forever().await;
    }
    let Some(endpoint) = parse_endpoint(&target) else {
        defmt::error!(
            "audio-debug: invalid target '{=str}' (expected ipv4:port); idle",
            target.as_str(),
        );
        park_forever().await;
    };
    let Ok(mut subscriber) = AUDIO_FRAME_PUBSUB.subscriber() else {
        defmt::error!("audio-debug: subscriber slot exhausted; idle");
        park_forever().await;
    };
    let Some(mut link) = WIFI_LINK_WATCH.receiver() else {
        defmt::error!("audio-debug: WIFI_LINK_WATCH receiver slot exhausted; idle");
        park_forever().await;
    };

    defmt::info!(
        "audio-debug: streaming {=usize}-byte UDP datagrams to target (wait for link)",
        DATAGRAM_BYTES,
    );

    let mut rx_meta = [PacketMetadata::EMPTY; 2];
    let mut rx_buf = [0_u8; 16]; // RX unused; we're send-only
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_buf = [0_u8; DATAGRAM_BYTES + 64];

    // Outer loop: wait for the link to be Connected, then stream
    // until either the link drops or a transient socket error breaks
    // the inner loop. Never block on `link.changed()` mid-stream — a
    // transient send_to failure leaves the link Connected, so a
    // blocking wait would stall the task until the next real
    // disconnect/reconnect cycle. Instead, the inner loop selects
    // across both the link receiver and the next-frame subscriber
    // so link changes are observed promptly.
    loop {
        let mut state = link.get().await;
        while !matches!(state, WifiLinkState::Connected) {
            state = link.changed().await;
        }
        let mut socket =
            UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
        if let Err(e) = socket.bind(0) {
            defmt::warn!("audio-debug: bind failed ({:?}); backing off", e);
            embassy_time::Timer::after(embassy_time::Duration::from_secs(5)).await;
            continue;
        }

        defmt::info!("audio-debug: socket ready, streaming");
        loop {
            use embassy_futures::select::{Either, select};
            let next = select(subscriber.next_message(), link.changed()).await;
            match next {
                Either::First(WaitResult::Message(frame)) => {
                    let mut buf = [0_u8; DATAGRAM_BYTES];
                    for (i, &sample) in frame.samples.iter().enumerate() {
                        let le = sample.to_le_bytes();
                        buf[i * 2] = le[0];
                        buf[i * 2 + 1] = le[1];
                    }
                    if let Err(e) = socket.send_to(&buf, endpoint).await {
                        defmt::warn!(
                            "audio-debug: send_to failed: {:?}; reopening socket",
                            defmt::Debug2Format(&e),
                        );
                        break;
                    }
                }
                Either::First(WaitResult::Lagged(n)) => {
                    defmt::warn!("audio-debug: lagged {=u64} frames", n);
                }
                Either::Second(new_state) => {
                    if !matches!(new_state, WifiLinkState::Connected) {
                        defmt::info!("audio-debug: link dropped, closing socket");
                        break;
                    }
                    // Otherwise the link bounced through some other
                    // intermediate state and re-stabilised at Connected
                    // — keep streaming.
                }
            }
        }
        socket.close();
    }
}

/// Parse `"a.b.c.d:port"` into an [`embassy_net::IpEndpoint`].
///
/// Returns `None` on any malformed component — task logs and parks
/// rather than crashing.
fn parse_endpoint(s: &str) -> Option<embassy_net::IpEndpoint> {
    let (host, port_str) = s.rsplit_once(':')?;
    let port: u16 = port_str.parse().ok()?;
    let mut octets = [0_u8; 4];
    let mut count = 0;
    for part in host.split('.') {
        if count >= 4 {
            return None;
        }
        octets[count] = part.parse().ok()?;
        count += 1;
    }
    if count != 4 {
        return None;
    }
    Some(embassy_net::IpEndpoint::new(
        embassy_net::IpAddress::Ipv4(embassy_net::Ipv4Address::new(
            octets[0], octets[1], octets[2], octets[3],
        )),
        port,
    ))
}

/// Spin forever, parking the task. Used when the task can't proceed
/// (no target configured, no subscriber slot, etc.).
async fn park_forever() -> ! {
    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(3600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::parse_endpoint;

    #[test]
    fn parse_endpoint_accepts_ipv4_with_port() {
        let e = parse_endpoint("192.168.1.42:5005").unwrap();
        assert_eq!(e.port, 5005);
    }

    #[test]
    fn parse_endpoint_rejects_missing_port() {
        assert!(parse_endpoint("192.168.1.42").is_none());
    }

    #[test]
    fn parse_endpoint_rejects_non_numeric_port() {
        assert!(parse_endpoint("192.168.1.42:abc").is_none());
    }

    #[test]
    fn parse_endpoint_rejects_too_few_octets() {
        assert!(parse_endpoint("192.168.1:5005").is_none());
    }

    #[test]
    fn parse_endpoint_rejects_too_many_octets() {
        assert!(parse_endpoint("1.2.3.4.5:5005").is_none());
    }

    #[test]
    fn parse_endpoint_rejects_empty() {
        assert!(parse_endpoint("").is_none());
    }
}
