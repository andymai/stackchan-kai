//! ESP-NOW frame format for Stack-chan remote control.
//!
//! ## Frame layout
//!
//! ```text
//! offset  bytes   field
//! 0       4       MAGIC = b"STKC"
//! 4       1       VERSION = 1
//! 5       1       kind (see [`EspNowKind`])
//! 6       N       JSON body (UTF-8) — empty for kinds with no payload
//! ```
//!
//! The JSON body for each kind is **byte-identical** to the HTTP API
//! body for the same action — so a remote (Arduino `M5StickC` sketch,
//! Python script over the host serial port, ...) reuses the same
//! payloads it would POST over HTTP, prefixed with the kind byte.
//! On the firmware side, the existing
//! [`crate::http_command`] parsers consume the JSON body as-is.
//!
//! ## Why kind-byte prefix instead of an `"action"` field
//!
//! A leading `"action"` key would force the parser to make two
//! passes over the body — once to discover the kind, once to bind
//! the rest. The kind byte is one fixed read. The wire size cost is
//! negligible (1 byte vs ~12 bytes for `"action":"x",`) and the
//! semantics are unambiguous: `kind` is mandatory, every byte after
//! position 6 is JSON.

use crate::http_command::{
    JsonError, parse_enter_pairing, parse_look_at, parse_set_emotion, parse_speak,
};
use stackchan_core::input::RemoteCommand;

/// Frame magic — first four bytes of every Stack-chan ESP-NOW frame.
/// Receivers that don't recognise this magic drop the frame silently.
pub const ESP_NOW_MAGIC: [u8; 4] = *b"STKC";

/// Wire-format version. Bumped on any breaking layout change so
/// senders + receivers can negotiate. Mismatching version on the
/// receiver drops the frame with a `VersionMismatch` decode error.
pub const ESP_NOW_VERSION: u8 = 1;

/// Header length in bytes (magic + version + kind).
pub const HEADER_LEN: usize = 6;

/// ESP-NOW peers max payload (ESP-IDF `ESP_NOW_MAX_DATA_LEN`).
pub const MAX_FRAME_LEN: usize = 250;

/// Maximum JSON body size = `MAX_FRAME_LEN - HEADER_LEN`.
pub const MAX_BODY_LEN: usize = MAX_FRAME_LEN - HEADER_LEN;

/// Frame kind. The numeric values are part of the wire contract —
/// **append only**. Re-using or shifting an existing value breaks
/// every shipped remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EspNowKind {
    /// Empty-body keepalive. Receivers ignore the body and treat the
    /// frame as proof-of-life from the peer.
    Heartbeat = 0x00,
    /// JSON body matches `POST /emotion` schema.
    SetEmotion = 0x01,
    /// JSON body matches `POST /look-at` schema.
    LookAt = 0x02,
    /// Empty body — corresponds to `POST /reset`.
    Reset = 0x03,
    /// JSON body matches `POST /speak` schema.
    Speak = 0x04,
    /// JSON body matches `POST /pair` schema.
    EnterPairing = 0x05,
}

impl EspNowKind {
    /// Decode a wire byte into an [`EspNowKind`]. `None` on unknown.
    #[must_use]
    pub const fn from_wire(b: u8) -> Option<Self> {
        match b {
            0x00 => Some(Self::Heartbeat),
            0x01 => Some(Self::SetEmotion),
            0x02 => Some(Self::LookAt),
            0x03 => Some(Self::Reset),
            0x04 => Some(Self::Speak),
            0x05 => Some(Self::EnterPairing),
            _ => None,
        }
    }

    /// True iff this kind expects a non-empty JSON body.
    #[must_use]
    pub const fn has_body(self) -> bool {
        match self {
            Self::Heartbeat | Self::Reset => false,
            Self::SetEmotion | Self::LookAt | Self::Speak | Self::EnterPairing => true,
        }
    }
}

/// Decode failure for a received ESP-NOW frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Frame shorter than [`HEADER_LEN`] — too small to even be a header.
    TooShort,
    /// Magic prefix didn't match [`ESP_NOW_MAGIC`].
    BadMagic,
    /// Header version byte didn't match [`ESP_NOW_VERSION`].
    VersionMismatch(u8),
    /// Header kind byte wasn't one of the known [`EspNowKind`] values.
    UnknownKind(u8),
    /// Frame exceeded [`MAX_FRAME_LEN`] — caller should drop.
    Oversize(usize),
    /// JSON body did not parse against the kind's schema. Carries
    /// the underlying [`JsonError`] for the firmware log path.
    BadBody(JsonError),
    /// Frame UTF-8 invalid (kinds with bodies require valid UTF-8).
    NotUtf8,
}

impl From<JsonError> for DecodeError {
    fn from(e: JsonError) -> Self {
        Self::BadBody(e)
    }
}

/// Decoded inbound ESP-NOW frame.
///
/// `Heartbeat` carries no payload; the rest map onto a
/// [`RemoteCommand`] ready to enqueue on the firmware-side
/// `REMOTE_COMMAND_SIGNAL`.
#[derive(Debug, Clone, PartialEq)]
pub enum InboundFrame {
    /// Liveness only — caller may use as proof-of-life for the peer.
    Heartbeat,
    /// Decoded remote command, ready to forward to the modifier graph.
    Command(RemoteCommand),
}

/// Parse a raw ESP-NOW frame buffer into an [`InboundFrame`].
///
/// Validates magic, version, kind, and body shape. Bodies are parsed
/// through the same [`crate::http_command`] parsers the HTTP control
/// plane uses — wire compatibility with a `POST /<route>` body is the
/// stable contract, not the byte layout of any individual command.
///
/// # Errors
///
/// Returns the matching [`DecodeError`] variant on any structural or
/// schema mismatch.
pub fn decode(buf: &[u8]) -> Result<InboundFrame, DecodeError> {
    if buf.len() > MAX_FRAME_LEN {
        return Err(DecodeError::Oversize(buf.len()));
    }
    if buf.len() < HEADER_LEN {
        return Err(DecodeError::TooShort);
    }
    if buf[0..4] != ESP_NOW_MAGIC {
        return Err(DecodeError::BadMagic);
    }
    if buf[4] != ESP_NOW_VERSION {
        return Err(DecodeError::VersionMismatch(buf[4]));
    }
    let kind = EspNowKind::from_wire(buf[5]).ok_or(DecodeError::UnknownKind(buf[5]))?;
    let body = &buf[HEADER_LEN..];
    let body_str = if body.is_empty() {
        ""
    } else {
        core::str::from_utf8(body).map_err(|_| DecodeError::NotUtf8)?
    };
    match kind {
        EspNowKind::Heartbeat => Ok(InboundFrame::Heartbeat),
        EspNowKind::Reset => Ok(InboundFrame::Command(RemoteCommand::Reset)),
        EspNowKind::SetEmotion => Ok(InboundFrame::Command(parse_set_emotion(body_str)?)),
        EspNowKind::LookAt => Ok(InboundFrame::Command(parse_look_at(body_str)?)),
        EspNowKind::Speak => Ok(InboundFrame::Command(parse_speak(body_str)?)),
        EspNowKind::EnterPairing => Ok(InboundFrame::Command(parse_enter_pairing(body_str)?)),
    }
}

/// Encode the 6-byte header for `kind` into `buf[..HEADER_LEN]`. Returns
/// the header length on success. Used by senders that follow it with
/// a JSON body of their own composition.
///
/// # Errors
///
/// Returns [`DecodeError::TooShort`] if `buf` can't hold
/// [`HEADER_LEN`] bytes.
pub fn encode_header(buf: &mut [u8], kind: EspNowKind) -> Result<usize, DecodeError> {
    if buf.len() < HEADER_LEN {
        return Err(DecodeError::TooShort);
    }
    buf[0..4].copy_from_slice(&ESP_NOW_MAGIC);
    buf[4] = ESP_NOW_VERSION;
    buf[5] = kind as u8;
    Ok(HEADER_LEN)
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test-only: unwrap + match-with-panic for variant extraction is the standard pattern"
)]
mod tests {
    use super::*;
    use stackchan_core::Emotion;
    use stackchan_core::voice::{Locale, PhraseId, Priority};

    #[test]
    fn header_round_trip() {
        let mut buf = [0_u8; HEADER_LEN];
        let n = encode_header(&mut buf, EspNowKind::SetEmotion).unwrap();
        assert_eq!(n, HEADER_LEN);
        assert_eq!(&buf[0..4], &ESP_NOW_MAGIC);
        assert_eq!(buf[4], ESP_NOW_VERSION);
        assert_eq!(buf[5], 0x01);
    }

    #[test]
    fn decode_heartbeat_empty_body() {
        let buf = [b'S', b'T', b'K', b'C', 1, 0x00];
        assert_eq!(decode(&buf), Ok(InboundFrame::Heartbeat));
    }

    #[test]
    fn decode_reset_empty_body() {
        let buf = [b'S', b'T', b'K', b'C', 1, 0x03];
        assert_eq!(
            decode(&buf),
            Ok(InboundFrame::Command(RemoteCommand::Reset))
        );
    }

    #[test]
    fn decode_set_emotion() {
        let mut frame: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        frame.extend_from_slice(&ESP_NOW_MAGIC);
        frame.push(ESP_NOW_VERSION);
        frame.push(EspNowKind::SetEmotion as u8);
        frame.extend_from_slice(br#"{"emotion":"happy","hold_ms":1500}"#);
        let decoded = decode(&frame).unwrap();
        match decoded {
            InboundFrame::Command(RemoteCommand::SetEmotion { emotion, hold_ms }) => {
                assert_eq!(emotion, Emotion::Happy);
                assert_eq!(hold_ms, 1500);
            }
            other => panic!("expected SetEmotion, got {other:?}"),
        }
    }

    #[test]
    fn decode_look_at() {
        let mut frame: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        frame.extend_from_slice(&ESP_NOW_MAGIC);
        frame.push(ESP_NOW_VERSION);
        frame.push(EspNowKind::LookAt as u8);
        frame.extend_from_slice(br#"{"pan_deg":12.0,"tilt_deg":-3.0}"#);
        let decoded = decode(&frame).unwrap();
        match decoded {
            InboundFrame::Command(RemoteCommand::LookAt { target, .. }) => {
                assert!((target.pan_deg - 12.0).abs() < f32::EPSILON);
                assert!((target.tilt_deg - -3.0).abs() < f32::EPSILON);
            }
            other => panic!("expected LookAt, got {other:?}"),
        }
    }

    #[test]
    fn decode_speak() {
        let mut frame: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        frame.extend_from_slice(&ESP_NOW_MAGIC);
        frame.push(ESP_NOW_VERSION);
        frame.push(EspNowKind::Speak as u8);
        frame.extend_from_slice(br#"{"phrase":"wake_chirp"}"#);
        let decoded = decode(&frame).unwrap();
        match decoded {
            InboundFrame::Command(RemoteCommand::Speak {
                phrase,
                locale,
                priority,
            }) => {
                assert_eq!(phrase, PhraseId::WakeChirp);
                assert_eq!(locale, Locale::En);
                assert_eq!(priority, Priority::Normal);
            }
            other => panic!("expected Speak, got {other:?}"),
        }
    }

    #[test]
    fn decode_enter_pairing_default() {
        let mut frame: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        frame.extend_from_slice(&ESP_NOW_MAGIC);
        frame.push(ESP_NOW_VERSION);
        frame.push(EspNowKind::EnterPairing as u8);
        frame.extend_from_slice(b"{}");
        let decoded = decode(&frame).unwrap();
        match decoded {
            InboundFrame::Command(RemoteCommand::EnterPairing { duration_ms }) => {
                assert_eq!(duration_ms, 30_000);
            }
            other => panic!("expected EnterPairing, got {other:?}"),
        }
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let buf = [0, 0, 0, 0, 1, 0];
        assert_eq!(decode(&buf), Err(DecodeError::BadMagic));
    }

    #[test]
    fn decode_rejects_version_mismatch() {
        let buf = [b'S', b'T', b'K', b'C', 99, 0];
        assert_eq!(decode(&buf), Err(DecodeError::VersionMismatch(99)));
    }

    #[test]
    fn decode_rejects_unknown_kind() {
        let buf = [b'S', b'T', b'K', b'C', 1, 0xff];
        assert_eq!(decode(&buf), Err(DecodeError::UnknownKind(0xff)));
    }

    #[test]
    fn decode_rejects_too_short() {
        let buf = [b'S', b'T', b'K'];
        assert_eq!(decode(&buf), Err(DecodeError::TooShort));
    }

    #[test]
    fn decode_rejects_oversize() {
        let buf = [0_u8; MAX_FRAME_LEN + 1];
        assert_eq!(decode(&buf), Err(DecodeError::Oversize(MAX_FRAME_LEN + 1)));
    }
}
