//! BluFi frame parsing + building.
//!
//! Implements just enough of Espressif's [BluFi] protocol to
//! provision Wi-Fi credentials over BLE from the official ESP
//! BLE Provisioning Android / iOS app (or any
//! BluFi-conformant central):
//!
//! - [`parse_frame`] decodes an inbound frame's header + data +
//!   optional CRC, returning a typed [`Frame`].
//! - [`build_frame`] serializes an outbound frame with the
//!   conventional `Direction = device→central` and
//!   `Checksum = on` flags, plus the trailing CRC16.
//! - [`Type`] / [`ControlSubtype`] / [`DataSubtype`] are the
//!   canonical enums for the subset of frame kinds the
//!   provisioning flow uses (set SSID, set password, connect to
//!   AP, ack, status notify).
//! - [`crc16_ccitt`] is the polynomial 0x1021 CRC the spec uses
//!   over `sequence + data_length + data` (cleartext).
//!
//! ## Wire format
//!
//! ```text
//!   byte 0:  Type           ((subtype << 2) | type)
//!   byte 1:  Frame Control  (encrypted | checksum | direction | ack | fragment | …)
//!   byte 2:  Sequence       (per-direction monotonic, wraps at 0xFF)
//!   byte 3:  Data Length    (length of the Data field, NOT incl. CRC)
//!   byte 4…: Data           (Data Length bytes)
//!   last 2:  CRC16-CCITT    (over Sequence + Data Length + Data; present iff FrameControl bit 1 set)
//! ```
//!
//! ## Out of scope (foundation slice)
//!
//! - Diffie-Hellman key negotiation + AES-CCM payload encryption.
//!   Operators relying on link-layer encryption from BLE LE
//!   Secure Connections get a comparable security posture; full
//!   BluFi crypto is a follow-up.
//! - Fragment reassembly. The `Fragment follows` FrameControl bit
//!   is parsed and surfaced as [`Frame::fragmented`]; the
//!   higher-level GATT handler is responsible for buffering
//!   continuations.
//! - GATT integration. The [`SERVICE_UUID`] / [`WRITE_CHAR_UUID`]
//!   / [`NOTIFY_CHAR_UUID`] constants are exported so a follow-up
//!   PR can wire the firmware-side GATT service against this
//!   parser without re-deriving them.
//!
//! [BluFi]: https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-guides/ble/blufi.html

#![allow(
    clippy::doc_markdown,
    reason = "spec-heavy reference module — backticking every protocol acronym (BluFi, BLE, AP, AES, SSID, WPA2, DH, STA, MAC, GATT, BSSID, …) per occurrence would bury the prose under markup"
)]

use alloc::vec::Vec;

/// 16-bit BluFi GATT service UUID.
pub const SERVICE_UUID: u16 = 0xFFFF;

/// Characteristic the central writes to in order to send a frame
/// to the device.
pub const WRITE_CHAR_UUID: u16 = 0xFF01;

/// Characteristic the device notifies on to push a frame to the
/// central.
pub const NOTIFY_CHAR_UUID: u16 = 0xFF02;

/// Polynomial for the CRC16-CCITT integrity check the spec uses.
pub const CRC16_POLY: u16 = 0x1021;

/// Initial register value for the CRC16-CCITT computation.
pub const CRC16_INIT: u16 = 0xFFFF;

/// Frame Control bit: payload is AES-CCM encrypted.
pub const FC_ENCRYPTED: u8 = 0x01;
/// Frame Control bit: a trailing CRC16 is present.
pub const FC_CHECKSUM: u8 = 0x02;
/// Frame Control bit: direction is device→central (set on
/// outbound frames; cleared on inbound).
pub const FC_DIRECTION_DEV_TO_CENTRAL: u8 = 0x04;
/// Frame Control bit: this frame requires the peer to ack.
pub const FC_ACK_REQUIRED: u8 = 0x08;
/// Frame Control bit: a fragment continuation follows this
/// frame. Reassembly is the GATT handler's responsibility — the
/// parser surfaces the bit unchanged.
pub const FC_FRAGMENT: u8 = 0x10;

/// Top-level frame type: low 2 bits of the Type byte. Distinguishes
/// the structurally-identical Control vs. Data shapes; the upper 6
/// bits carry the [`ControlSubtype`] / [`DataSubtype`] enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    /// Connection / negotiation / status / ack.
    Control,
    /// SSID / password / certificate / custom data.
    Data,
}

/// Control-frame subtypes used by the provisioning flow.
///
/// `#[repr(u8)]` so the wire byte and the enum variant share one
/// numeric value, which keeps the round-trip parse / build paths
/// trivially correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ControlSubtype {
    /// `0x00` — acknowledge a previously received frame's
    /// sequence number. Data byte 0 is the sequence being acked.
    Ack = 0x00,
    /// `0x01` — set the security mode (encryption + checksum
    /// gates on subsequent frames). Data byte 0 is a bitmask.
    SetSecMode = 0x01,
    /// `0x02` — set the Wi-Fi operating mode. Data byte 0 is one
    /// of `WIFI_OP_NULL` (0) / `WIFI_OP_STA` (1) / `WIFI_OP_AP`
    /// (2) / `WIFI_OP_APSTA` (3).
    SetWifiOpMode = 0x02,
    /// `0x03` — commit the staged SSID + password and connect.
    /// No payload data.
    ConnectToAp = 0x03,
    /// `0x04` — disconnect the station. No payload data.
    DisconnectFromAp = 0x04,
    /// `0x05` — request the current Wi-Fi status from the
    /// device. No payload data; the device replies with a
    /// [`DataSubtype::ReportWifiStatus`] frame.
    GetWifiStatus = 0x05,
    /// `0x06` — instruct the AP to deauth a client. Unused in the
    /// STA-only provisioning flow but kept for completeness.
    DeauthSta = 0x06,
    /// `0x07` — request a version handshake.
    GetVersion = 0x07,
    /// `0x08` — disconnect BluFi after provisioning succeeds.
    Disconnect = 0x08,
    /// `0x09` — set the encryption-mode policy (host-only).
    GetWifiList = 0x09,
}

/// Data-frame subtypes used by the provisioning flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DataSubtype {
    /// `0x00` — Diffie-Hellman negotiation payload (skip; we're
    /// not implementing crypto in this slice).
    NegotiationData = 0x00,
    /// `0x01` — set the station BSSID (peer MAC). Optional.
    SendStaBssid = 0x01,
    /// `0x02` — set the station SSID. Data is the raw SSID
    /// bytes (UTF-8 / printable ASCII per 802.11).
    SendStaSsid = 0x02,
    /// `0x03` — set the station password. Data is the raw WPA2
    /// passphrase bytes (8–63 ASCII chars).
    SendStaPassword = 0x03,
    /// `0x04` — set the SoftAP SSID. Unused in STA-only flow.
    SendSoftApSsid = 0x04,
    /// `0x05` — set the SoftAP password. Unused in STA-only.
    SendSoftApPassword = 0x05,
    /// `0x06` — set SoftAP max connections. Unused in STA-only.
    SendSoftApMaxConn = 0x06,
    /// `0x07` — set SoftAP auth mode. Unused in STA-only.
    SendSoftApAuthMode = 0x07,
    /// `0x08` — set SoftAP channel. Unused in STA-only.
    SendSoftApChannel = 0x08,
    /// `0x09` — set the username for WPA2-Enterprise. Unused.
    SendUsername = 0x09,
    /// `0x0A` — set the CA certificate for WPA2-Enterprise.
    SendCaCert = 0x0A,
    /// `0x0B` — set the client certificate (WPA2-Enterprise).
    SendClientCert = 0x0B,
    /// `0x0C` — set the server certificate (WPA2-Enterprise).
    SendServerCert = 0x0C,
    /// `0x0D` — set the client private key (WPA2-Enterprise).
    SendClientPrivateKey = 0x0D,
    /// `0x0E` — set the server private key (WPA2-Enterprise).
    SendServerPrivateKey = 0x0E,
    /// `0x0F` — device replies with the Wi-Fi connection status.
    /// Data is [opmode, conn_state, extra_info...].
    ReportWifiStatus = 0x0F,
    /// `0x10` — report a list of nearby APs after a scan.
    ReportWifiList = 0x10,
    /// `0x11` — error report. Data byte 0 is the error code.
    Error = 0x11,
    /// `0x12` — custom payload passed straight through to the
    /// application layer. Operator-defined semantics.
    CustomData = 0x12,
    /// `0x13` — set the maximum reassembly buffer size (negotiated
    /// at handshake time so each side can size its fragment
    /// staging area).
    SetMaxFragment = 0x13,
}

/// One decoded BluFi frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// `Control` or `Data`.
    pub frame_type: Type,
    /// Raw subtype value from the upper 6 bits of the Type byte.
    /// Callers can match against [`ControlSubtype`] /
    /// [`DataSubtype`] via the typed helpers
    /// [`Frame::control_subtype`] / [`Frame::data_subtype`].
    pub subtype: u8,
    /// Per-direction monotonic sequence (wraps at 0xFF).
    pub sequence: u8,
    /// Frame Control byte verbatim. Use the `FC_*` constants to
    /// gate on individual bits; we re-expose the raw byte rather
    /// than expand into a struct because the spec keeps adding
    /// bits and a future implementation that handles encryption
    /// or fragmentation needs the originals.
    pub frame_control: u8,
    /// Frame payload (Data Length bytes from the wire, after any
    /// optional CRC has been stripped off).
    pub data: Vec<u8>,
    /// `true` iff the Fragment-follows bit is set; the GATT
    /// handler is responsible for buffering continuations.
    pub fragmented: bool,
    /// `true` iff the encryption bit is set. The parser does
    /// not decrypt — a follow-up that implements AES-CCM is the
    /// natural consumer.
    pub encrypted: bool,
}

impl Frame {
    /// Try to interpret the raw `subtype` as a [`ControlSubtype`].
    ///
    /// Returns `None` for unknown subtype values or when this
    /// isn't a control frame.
    #[must_use]
    pub const fn control_subtype(&self) -> Option<ControlSubtype> {
        if !matches!(self.frame_type, Type::Control) {
            return None;
        }
        match self.subtype {
            0x00 => Some(ControlSubtype::Ack),
            0x01 => Some(ControlSubtype::SetSecMode),
            0x02 => Some(ControlSubtype::SetWifiOpMode),
            0x03 => Some(ControlSubtype::ConnectToAp),
            0x04 => Some(ControlSubtype::DisconnectFromAp),
            0x05 => Some(ControlSubtype::GetWifiStatus),
            0x06 => Some(ControlSubtype::DeauthSta),
            0x07 => Some(ControlSubtype::GetVersion),
            0x08 => Some(ControlSubtype::Disconnect),
            0x09 => Some(ControlSubtype::GetWifiList),
            _ => None,
        }
    }

    /// Try to interpret the raw `subtype` as a [`DataSubtype`].
    ///
    /// Returns `None` for unknown subtype values or when this
    /// isn't a data frame.
    #[must_use]
    pub const fn data_subtype(&self) -> Option<DataSubtype> {
        if !matches!(self.frame_type, Type::Data) {
            return None;
        }
        match self.subtype {
            0x00 => Some(DataSubtype::NegotiationData),
            0x01 => Some(DataSubtype::SendStaBssid),
            0x02 => Some(DataSubtype::SendStaSsid),
            0x03 => Some(DataSubtype::SendStaPassword),
            0x04 => Some(DataSubtype::SendSoftApSsid),
            0x05 => Some(DataSubtype::SendSoftApPassword),
            0x06 => Some(DataSubtype::SendSoftApMaxConn),
            0x07 => Some(DataSubtype::SendSoftApAuthMode),
            0x08 => Some(DataSubtype::SendSoftApChannel),
            0x09 => Some(DataSubtype::SendUsername),
            0x0A => Some(DataSubtype::SendCaCert),
            0x0B => Some(DataSubtype::SendClientCert),
            0x0C => Some(DataSubtype::SendServerCert),
            0x0D => Some(DataSubtype::SendClientPrivateKey),
            0x0E => Some(DataSubtype::SendServerPrivateKey),
            0x0F => Some(DataSubtype::ReportWifiStatus),
            0x10 => Some(DataSubtype::ReportWifiList),
            0x11 => Some(DataSubtype::Error),
            0x12 => Some(DataSubtype::CustomData),
            0x13 => Some(DataSubtype::SetMaxFragment),
            _ => None,
        }
    }
}

/// Reasons [`parse_frame`] can refuse a frame. All variants are
/// recoverable from the GATT handler's view — drop the frame, log
/// the cause, and wait for the next one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Buffer too short to even hold the 4-byte header.
    Truncated,
    /// `Data Length` field claims more bytes than are present in
    /// the buffer (or fewer than the trailing CRC needs).
    BadDataLength,
    /// `Frame Control` bit 1 says a CRC is present but the
    /// trailing bytes don't match the computed CRC16-CCITT over
    /// `sequence + data_length + data`.
    BadCrc,
}

/// Parse one BluFi frame from `buf`.
///
/// `buf` must contain exactly one frame — the GATT handler is
/// responsible for slicing per ATT write (BluFi never packs
/// multiple frames into one write).
///
/// # Errors
///
/// See [`ParseError`].
pub fn parse_frame(buf: &[u8]) -> Result<Frame, ParseError> {
    if buf.len() < 4 {
        return Err(ParseError::Truncated);
    }
    let type_byte = buf[0];
    let frame_control = buf[1];
    let sequence = buf[2];
    let data_length = buf[3] as usize;

    let has_crc = frame_control & FC_CHECKSUM != 0;
    let body_len = data_length + if has_crc { 2 } else { 0 };
    if 4 + body_len > buf.len() {
        return Err(ParseError::BadDataLength);
    }

    let data_end = 4 + data_length;
    let data = buf[4..data_end].to_vec();

    if has_crc {
        let expected = u16::from_le_bytes([buf[data_end], buf[data_end + 1]]);
        // CRC is computed over sequence + data_length + data
        // (cleartext, per spec).
        let mut crc_input: Vec<u8> = Vec::with_capacity(2 + data_length);
        crc_input.push(sequence);
        crc_input.push(buf[3]);
        crc_input.extend_from_slice(&buf[4..data_end]);
        let computed = crc16_ccitt(&crc_input);
        if computed != expected {
            return Err(ParseError::BadCrc);
        }
    }

    let frame_type = match type_byte & 0b11 {
        0b00 => Type::Control,
        _ => Type::Data,
    };
    let subtype = type_byte >> 2;

    Ok(Frame {
        frame_type,
        subtype,
        sequence,
        frame_control,
        data,
        fragmented: frame_control & FC_FRAGMENT != 0,
        encrypted: frame_control & FC_ENCRYPTED != 0,
    })
}

/// Serialize a frame for transmission.
///
/// Sets the device→central direction bit and adds a trailing
/// CRC16; the encryption / fragmentation bits default off and
/// can be flipped on by the caller for a follow-up implementation.
///
/// Returns the byte vector ready to hand to a GATT notify.
///
/// # Errors
///
/// Returns [`BuildError::DataTooLong`] when `data.len() > 255`
/// — the Data Length field is a single byte and longer
/// payloads need fragmentation, which the caller is responsible
/// for splitting into multiple `build_frame` calls with the
/// Fragment-follows bit set.
pub fn build_frame(
    frame_type: Type,
    subtype: u8,
    sequence: u8,
    data: &[u8],
) -> Result<Vec<u8>, BuildError> {
    if data.len() > u8::MAX as usize {
        return Err(BuildError::DataTooLong);
    }
    let type_byte = ((subtype & 0b0011_1111) << 2)
        | match frame_type {
            Type::Control => 0b00,
            Type::Data => 0b01,
        };
    let frame_control = FC_DIRECTION_DEV_TO_CENTRAL | FC_CHECKSUM;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "data.len() <= u8::MAX checked above"
    )]
    let data_length = data.len() as u8;

    let mut out: Vec<u8> = Vec::with_capacity(6 + data.len());
    out.push(type_byte);
    out.push(frame_control);
    out.push(sequence);
    out.push(data_length);
    out.extend_from_slice(data);

    // CRC over sequence + data_length + data.
    let mut crc_input: Vec<u8> = Vec::with_capacity(2 + data.len());
    crc_input.push(sequence);
    crc_input.push(data_length);
    crc_input.extend_from_slice(data);
    let crc = crc16_ccitt(&crc_input);
    out.extend_from_slice(&crc.to_le_bytes());
    Ok(out)
}

/// Reasons [`build_frame`] can refuse to serialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildError {
    /// Data payload exceeds 255 bytes; needs fragmentation.
    DataTooLong,
}

/// CRC16-CCITT over the given bytes.
///
/// Polynomial `0x1021`, initial `0xFFFF`. The non-`const`
/// arithmetic + table-free implementation matches the byte cost
/// (`~8 cycles` per byte on Xtensa) of an inlined lookup table
/// for the 60-byte-or-so inputs the BluFi spec uses.
#[must_use]
pub fn crc16_ccitt(bytes: &[u8]) -> u16 {
    let mut crc: u16 = CRC16_INIT;
    for &b in bytes {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ CRC16_POLY
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests assert structural invariants; .expect / .unwrap is the standard test idiom"
)]
mod tests {
    use super::*;

    #[test]
    fn crc16_matches_known_vectors() {
        // Empty input: register stays at the init value.
        assert_eq!(crc16_ccitt(&[]), CRC16_INIT);
        // Canonical CCITT test vector: 9 bytes of ASCII "123456789"
        // → 0x29B1.
        assert_eq!(crc16_ccitt(b"123456789"), 0x29B1);
    }

    #[test]
    fn type_byte_packs_subtype_in_high_bits_and_type_in_low() {
        // Data Frame (low 2 bits = 01) carrying Send STA SSID
        // (subtype 0x02): Type byte = (0x02 << 2) | 0x01 = 0x09.
        let bytes = build_frame(Type::Data, DataSubtype::SendStaSsid as u8, 0, b"my-ap").unwrap();
        assert_eq!(bytes[0], 0x09);
    }

    #[test]
    fn build_then_parse_round_trip_ack() {
        // Ack control frame for sequence 7: type=Control, subtype=Ack,
        // data = [0x07] (the sequence being acked).
        let built = build_frame(Type::Control, ControlSubtype::Ack as u8, 42, &[0x07]).unwrap();
        let parsed = parse_frame(&built).unwrap();
        assert_eq!(parsed.frame_type, Type::Control);
        assert_eq!(parsed.control_subtype(), Some(ControlSubtype::Ack));
        assert_eq!(parsed.sequence, 42);
        assert_eq!(parsed.data, [0x07]);
        // Builder sets Direction=device→central + Checksum=on by default.
        assert!(parsed.frame_control & FC_DIRECTION_DEV_TO_CENTRAL != 0);
        assert!(parsed.frame_control & FC_CHECKSUM != 0);
        assert!(!parsed.fragmented);
        assert!(!parsed.encrypted);
    }

    #[test]
    fn build_then_parse_round_trip_ssid() {
        let built = build_frame(
            Type::Data,
            DataSubtype::SendStaSsid as u8,
            5,
            b"my-home-network",
        )
        .unwrap();
        let parsed = parse_frame(&built).unwrap();
        assert_eq!(parsed.data_subtype(), Some(DataSubtype::SendStaSsid));
        assert_eq!(parsed.data, b"my-home-network");
    }

    #[test]
    fn build_then_parse_round_trip_password() {
        let built = build_frame(
            Type::Data,
            DataSubtype::SendStaPassword as u8,
            6,
            b"correct-horse-battery-staple",
        )
        .unwrap();
        let parsed = parse_frame(&built).unwrap();
        assert_eq!(parsed.data_subtype(), Some(DataSubtype::SendStaPassword));
        assert_eq!(parsed.data, b"correct-horse-battery-staple");
    }

    #[test]
    fn parse_frame_rejects_short_buffer() {
        assert_eq!(parse_frame(&[]), Err(ParseError::Truncated));
        assert_eq!(parse_frame(&[0, 0, 0]), Err(ParseError::Truncated));
    }

    #[test]
    fn parse_frame_rejects_data_length_longer_than_buffer() {
        // Header says 10-byte data + checksum-on (so +2 CRC), but
        // only the header is present.
        let buf = [0x09, FC_CHECKSUM, 0x00, 0x0A];
        assert_eq!(parse_frame(&buf), Err(ParseError::BadDataLength));
    }

    #[test]
    fn parse_frame_rejects_corrupted_crc() {
        let mut built = build_frame(Type::Control, ControlSubtype::Ack as u8, 1, &[0x00]).unwrap();
        // Tamper with the last CRC byte.
        let last = built.len() - 1;
        built[last] ^= 0xFF;
        assert_eq!(parse_frame(&built), Err(ParseError::BadCrc));
    }

    #[test]
    fn parse_frame_accepts_frame_with_checksum_off() {
        // Without the checksum bit, no trailing CRC is expected.
        let buf = [
            (ControlSubtype::Ack as u8) << 2, // Type byte: subtype=0 (Ack), type=00 (Control)
            0x00,                             // Frame control: nothing set
            0x09,                             // Sequence
            0x01,                             // Data length
            0x42,                             // Data payload
        ];
        let parsed = parse_frame(&buf).unwrap();
        assert_eq!(parsed.control_subtype(), Some(ControlSubtype::Ack));
        assert_eq!(parsed.sequence, 0x09);
        assert_eq!(parsed.data, [0x42]);
    }

    #[test]
    fn parse_frame_surfaces_fragment_and_encrypted_bits() {
        let mut buf = build_frame(Type::Data, DataSubtype::SendStaSsid as u8, 0, &[0; 4]).unwrap();
        // Set the fragment + encrypted bits in Frame Control.
        buf[1] |= FC_FRAGMENT | FC_ENCRYPTED;
        // Recompute CRC since FrameControl is NOT in the CRC input,
        // but the parser doesn't care which inputs went into the
        // CRC the sender used — only that they match the trailing
        // bytes. The pre-built CRC still matches since the CRC
        // input (sequence + data_length + data) didn't change.
        let parsed = parse_frame(&buf).unwrap();
        assert!(parsed.fragmented);
        assert!(parsed.encrypted);
    }

    #[test]
    fn parse_frame_distinguishes_control_from_data_via_low_bits() {
        // Same subtype value (0x03) means different things based
        // on the Type bits — ConnectToAp (control) vs.
        // SendStaPassword (data).
        let control_byte = (ControlSubtype::ConnectToAp as u8) << 2;
        let data_byte = ((DataSubtype::SendStaPassword as u8) << 2) | 0x01;
        assert_ne!(control_byte, data_byte);

        let control =
            build_frame(Type::Control, ControlSubtype::ConnectToAp as u8, 0, &[]).unwrap();
        let parsed = parse_frame(&control).unwrap();
        assert_eq!(parsed.frame_type, Type::Control);
        assert_eq!(parsed.control_subtype(), Some(ControlSubtype::ConnectToAp));
        assert!(parsed.data_subtype().is_none());
    }

    #[test]
    fn build_frame_rejects_payload_over_255_bytes() {
        let big = [0u8; 256];
        assert_eq!(
            build_frame(Type::Data, DataSubtype::SendStaPassword as u8, 0, &big),
            Err(BuildError::DataTooLong)
        );
        // 255 still fits.
        let edge = [0u8; 255];
        assert!(build_frame(Type::Data, DataSubtype::SendStaPassword as u8, 0, &edge).is_ok());
    }

    #[test]
    fn unknown_subtype_returns_none_from_helpers() {
        let f = Frame {
            frame_type: Type::Control,
            subtype: 0x3F, // upper-bound legal value but unassigned
            sequence: 0,
            frame_control: 0,
            data: Vec::new(),
            fragmented: false,
            encrypted: false,
        };
        assert_eq!(f.control_subtype(), None);
    }
}
