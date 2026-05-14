//! WebSocket primitives for the live-state push path.
//!
//! Implements just enough of RFC 6455 to run a server-only push
//! stream over `GET /state/ws`:
//!
//! - [`compute_accept_key`] hashes the client's
//!   `Sec-WebSocket-Key` with the RFC 6455 GUID and base64-encodes
//!   the SHA-1 digest. This is the value the handshake echoes back
//!   in `Sec-WebSocket-Accept`.
//! - [`parse_websocket_key`] locates the `Sec-WebSocket-Key`
//!   header value inside the raw header byte slice. Case-
//!   insensitive on the header name, whitespace-tolerant on the
//!   value.
//! - [`write_text_frame`] emits one unfragmented server-side
//!   text frame (FIN = 1, opcode = 0x1, mask bit = 0).
//! - [`write_ping_frame`] emits a zero-payload ping (opcode = 0x9)
//!   so reverse-proxy and NAT idle timers don't tear long-lived
//!   connections down.
//!
//! Client-to-server frames, fragmentation, binary opcodes, and
//! per-frame masking aren't implemented because the live-state
//! endpoint never reads from the client after the handshake.
//! Adding bidirectional support is a follow-up — the primitives
//! here cover everything `GET /state/ws` needs today.

use embassy_net::tcp::TcpSocket;
use embedded_io_async::Write as _;
use sha1_smol::Sha1;

/// Magic GUID concatenated with the client key before hashing.
/// Defined in RFC 6455 § 4.2.2.
const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Length of the base64 encoding of a 20-byte SHA-1 digest. 20
/// bytes → ceil(20/3) × 4 = 28 chars (with one trailing `=`).
pub const ACCEPT_KEY_LEN: usize = 28;

/// Compute the value of the server's `Sec-WebSocket-Accept`
/// header from the client's `Sec-WebSocket-Key`.
///
/// Per RFC 6455 § 4.2.2: `base64(sha1(key + GUID))`. The output
/// length is fixed at [`ACCEPT_KEY_LEN`] bytes.
#[must_use]
pub fn compute_accept_key(key: &[u8]) -> heapless::String<ACCEPT_KEY_LEN> {
    let mut sha = Sha1::new();
    sha.update(key);
    sha.update(WS_GUID);
    let digest = sha.digest().bytes();
    base64_encode_20(&digest)
}

/// Find the `Sec-WebSocket-Key` header value in a raw header
/// byte slice. Case-insensitive on the header name.
///
/// Returns the trimmed value bytes, or `None` if the header
/// isn't present or its line is malformed (no `:` separator).
#[must_use]
pub fn parse_websocket_key(headers: &[u8]) -> Option<&[u8]> {
    parse_header_value(headers, b"sec-websocket-key")
}

/// Generic case-insensitive header value lookup. Pulled out so
/// the `Upgrade` / `Connection` / `Sec-WebSocket-Version` gates
/// can layer on later without rewriting the scan.
#[must_use]
pub fn parse_header_value<'a>(headers: &'a [u8], name_lower: &[u8]) -> Option<&'a [u8]> {
    for line in headers.split(|&b| b == b'\n') {
        // Strip optional trailing `\r` from `\r\n` splits.
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        // `?` here would abort the whole scan on any colon-less
        // line — including the empty trailing element that
        // splitting a `\n`-terminated header block always produces.
        // Skip to the next line instead so a malformed line before
        // the target header doesn't make the caller spuriously
        // 400 the request.
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            continue;
        };
        let (raw_name, raw_value) = line.split_at(colon);
        if raw_name.len() != name_lower.len() {
            continue;
        }
        if !raw_name
            .iter()
            .zip(name_lower.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
        {
            continue;
        }
        // Strip the `:` and surrounding whitespace.
        let value = raw_value
            .strip_prefix(b":")
            .unwrap_or(raw_value)
            .iter()
            .position(|&b| b != b' ' && b != b'\t')
            .map_or(&raw_value[..0], |start| {
                let v = &raw_value[1..][start..];
                // Trim trailing whitespace.
                let end = v
                    .iter()
                    .rposition(|&b| b != b' ' && b != b'\t' && b != b'\r')
                    .map_or(0, |e| e + 1);
                &v[..end]
            });
        return Some(value);
    }
    None
}

/// Write one unfragmented server-side WebSocket text frame
/// (opcode = 0x1, FIN = 1, mask = 0).
///
/// Encodes the payload-length prefix per RFC 6455 § 5.2 — 7-bit
/// inline for ≤125 bytes, 7 + 16-bit for ≤65 535, 7 + 64-bit
/// otherwise. The send is two `write_all` calls (header bytes +
/// payload bytes) so the framing stays trivially correct vs.
/// constructing one large staging buffer.
///
/// # Errors
///
/// Propagates a socket write failure as a unit-error `()`. The
/// caller's outer loop logs and rebinds — same shape as
/// `handle_state_stream`'s SSE path.
pub async fn write_text_frame(socket: &mut TcpSocket<'_>, payload: &[u8]) -> Result<(), ()> {
    let mut header = [0_u8; 10];
    let header_len = encode_text_frame_header(payload.len(), &mut header);
    socket
        .write_all(&header[..header_len])
        .await
        .map_err(|_| ())?;
    socket.write_all(payload).await.map_err(|_| ())?;
    socket.flush().await.map_err(|_| ())
}

/// Write a zero-payload server-side WebSocket ping frame
/// (opcode = 0x9, FIN = 1, mask = 0, length = 0).
///
/// # Errors
///
/// Propagates a socket write failure as a unit-error `()`.
pub async fn write_ping_frame(socket: &mut TcpSocket<'_>) -> Result<(), ()> {
    socket.write_all(&[0x89, 0x00]).await.map_err(|_| ())?;
    socket.flush().await.map_err(|_| ())
}

/// Fill `header` with the frame prefix for an unfragmented
/// server-side text frame carrying `payload_len` bytes, returning
/// the prefix length.
///
/// Layout per RFC 6455 § 5.2:
///   - byte 0: `0x81` (FIN = 1, opcode = 0x1)
///   - byte 1 low 7 bits: `len_field` (`< 126`, `126`, or `127`)
///   - byte 1 high bit (mask): `0` — server-to-client never masks
///   - extended length (0 / 2 / 8 bytes, big-endian) if the
///     `len_field` is `126` or `127`.
fn encode_text_frame_header(payload_len: usize, header: &mut [u8; 10]) -> usize {
    header[0] = 0x81;
    if payload_len < 126 {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "payload_len < 126 fits in u8 by construction"
        )]
        {
            header[1] = payload_len as u8;
        }
        2
    } else if let Ok(len) = u16::try_from(payload_len) {
        header[1] = 126;
        header[2..4].copy_from_slice(&len.to_be_bytes());
        4
    } else {
        header[1] = 127;
        let len = payload_len as u64;
        header[2..10].copy_from_slice(&len.to_be_bytes());
        10
    }
}

/// Base64-encode exactly 20 bytes into 28 chars (one trailing
/// `=` pad).
///
/// Hand-rolled for the WebSocket handshake's fixed-size SHA-1
/// digest so we don't pull a `base64` crate version into the dep
/// graph alongside whatever trouble-host transitively brought in.
/// Not exported — the only caller is [`compute_accept_key`].
fn base64_encode_20(bytes: &[u8; 20]) -> heapless::String<ACCEPT_KEY_LEN> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = heapless::String::<ACCEPT_KEY_LEN>::new();
    // Six full triplets cover the first 18 bytes.
    let mut i = 0;
    while i + 3 <= 18 {
        let b0 = bytes[i];
        let b1 = bytes[i + 1];
        let b2 = bytes[i + 2];
        let _ = out.push(ALPHABET[(b0 >> 2) as usize] as char);
        let _ = out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        let _ = out.push(ALPHABET[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        let _ = out.push(ALPHABET[(b2 & 0x3F) as usize] as char);
        i += 3;
    }
    // Two trailing bytes → 3 chars + 1 pad.
    let b0 = bytes[18];
    let b1 = bytes[19];
    let _ = out.push(ALPHABET[(b0 >> 2) as usize] as char);
    let _ = out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
    let _ = out.push(ALPHABET[((b1 & 0x0F) << 2) as usize] as char);
    let _ = out.push('=');
    out
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests assert RFC 6455 fixtures; .expect / .unwrap is the standard test idiom"
)]
mod tests {
    use super::*;

    #[test]
    fn accept_key_matches_rfc6455_example() {
        // From RFC 6455 § 1.3:
        //   key:    "dGhlIHNhbXBsZSBub25jZQ=="
        //   accept: "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        let accept = compute_accept_key(b"dGhlIHNhbXBsZSBub25jZQ==");
        assert_eq!(accept.as_str(), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn parse_websocket_key_case_insensitive_header_name() {
        let headers =
            b"Host: 192.168.1.1\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nUpgrade: websocket\r\n";
        let key = parse_websocket_key(headers).unwrap();
        assert_eq!(key, b"dGhlIHNhbXBsZSBub25jZQ==");
    }

    #[test]
    fn parse_websocket_key_lowercase_header_works_too() {
        let headers = b"sec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\n";
        let key = parse_websocket_key(headers).unwrap();
        assert_eq!(key, b"dGhlIHNhbXBsZSBub25jZQ==");
    }

    #[test]
    fn parse_websocket_key_returns_none_when_absent() {
        let headers = b"Host: x\r\nConnection: keep-alive\r\n";
        assert!(parse_websocket_key(headers).is_none());
    }

    #[test]
    fn parse_websocket_key_tolerates_extra_whitespace() {
        let headers = b"Sec-WebSocket-Key:    dGhlIHNhbXBsZSBub25jZQ==   \r\n";
        let key = parse_websocket_key(headers).unwrap();
        assert_eq!(key, b"dGhlIHNhbXBsZSBub25jZQ==");
    }

    #[test]
    fn parse_websocket_key_skips_colon_less_lines() {
        // A colon-less line before the target header used to abort
        // the scan via the trailing `?` and surface as a spurious
        // 400 at the handler. The `continue` path must keep
        // scanning so the target header still gets found.
        let headers = b"garbage-line-no-colon\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n";
        let key = parse_websocket_key(headers).unwrap();
        assert_eq!(key, b"dGhlIHNhbXBsZSBub25jZQ==");
    }

    #[test]
    fn frame_header_encodes_short_payload_in_two_bytes() {
        let mut h = [0_u8; 10];
        let n = encode_text_frame_header(5, &mut h);
        assert_eq!(n, 2);
        assert_eq!(h[0], 0x81);
        assert_eq!(h[1], 5);
    }

    #[test]
    fn frame_header_encodes_125_byte_payload_in_two_bytes() {
        let mut h = [0_u8; 10];
        let n = encode_text_frame_header(125, &mut h);
        assert_eq!(n, 2);
        assert_eq!(h[1], 125);
    }

    #[test]
    fn frame_header_encodes_126_byte_payload_with_16bit_length() {
        let mut h = [0_u8; 10];
        let n = encode_text_frame_header(126, &mut h);
        assert_eq!(n, 4);
        assert_eq!(h[1], 126);
        assert_eq!(u16::from_be_bytes([h[2], h[3]]), 126);
    }

    #[test]
    fn frame_header_encodes_64k_payload_with_16bit_length() {
        let mut h = [0_u8; 10];
        let n = encode_text_frame_header(usize::from(u16::MAX), &mut h);
        assert_eq!(n, 4);
        assert_eq!(h[1], 126);
        assert_eq!(u16::from_be_bytes([h[2], h[3]]), u16::MAX);
    }

    #[test]
    fn frame_header_encodes_oversize_payload_with_64bit_length() {
        let mut h = [0_u8; 10];
        let n = encode_text_frame_header(usize::from(u16::MAX) + 1, &mut h);
        assert_eq!(n, 10);
        assert_eq!(h[1], 127);
        assert_eq!(
            u64::from_be_bytes([h[2], h[3], h[4], h[5], h[6], h[7], h[8], h[9]]),
            u64::from(u16::MAX) + 1
        );
    }
}
