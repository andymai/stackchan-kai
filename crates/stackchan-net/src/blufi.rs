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
//! - Fragment reassembly *state*. The `Fragment follows`
//!   FrameControl bit is parsed and surfaced as
//!   [`Frame::fragmented`]; [`fragment_content`] is the stateless
//!   helper that strips a fragmented frame's leading
//!   total-content-length prefix. The accumulation buffer lives in
//!   the GATT handler, which owns per-chain state.
//! - GATT integration. The [`SERVICE_UUID`] / [`WRITE_CHAR_UUID`]
//!   / [`NOTIFY_CHAR_UUID`] `u16` constants are exported for code
//!   paths that consume UUIDs at runtime — service-discovery filters,
//!   defmt logs, central-side scanners. The firmware-side
//!   `#[gatt_service]` proc-macro only accepts string literals,
//!   so the BLE peripheral re-derives the 128-bit canonical forms
//!   (`0000ffff-0000-1000-8000-00805f9b34fb` etc.) directly in the
//!   service declaration. Treat the two as parallel views of the
//!   same UUIDs rather than a single source of truth — when a
//!   future revision changes one, update both.
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

/// Major version byte we advertise in a `ControlSubtype::GetVersion` reply.
///
/// Matches the value the ESP-IDF v5.x reference implementation
/// reports so the official ESP BLE Provisioning Android / iOS app
/// keeps its compatibility path.
pub const PROTOCOL_VERSION_MAJOR: u8 = 0x01;
/// Minor version byte; see [`PROTOCOL_VERSION_MAJOR`].
pub const PROTOCOL_VERSION_MINOR: u8 = 0x02;

/// Wi-Fi operating-mode byte used in the first slot of a
/// `ReportWifiStatus` payload. Spec enumerates Null/STA/AP/APSTA; the
/// firmware only ever runs STA, so the helper hard-codes that mode.
pub const WIFI_OP_MODE_STA: u8 = 0x01;

/// STA-side connection state for `ReportWifiStatus` (Data byte 1).
///
/// Only the codes the firmware actually emits are spelled out.
/// `repr(u8)` matches the spec byte verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WifiConnState {
    /// STA is associated with an AP.
    Connected = 0x00,
    /// STA is not currently associated.
    NotConnected = 0x01,
}

/// BluFi error-code byte for an outbound `DataSubtype::Error` frame.
///
/// Values match `esp_blufi_error_state_t` in ESP-IDF
/// (`components/bt/common/api/include/api/esp_blufi_api.h`) so the
/// standard ESP BLE Provisioning app surfaces the documented dialog.
/// Only the subset the firmware actively emits is enumerated;
/// adding a variant is the right move when a new failure path needs
/// to be distinguished on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ErrorCode {
    /// `0x09` — `ESP_BLUFI_DATA_FORMAT_ERROR`. Used as the catch-all
    /// for the provisioning path's commit-side rejections (empty
    /// SSID, below-spec PSK length, missing config snapshot, SD
    /// write IO error). The official Android / iOS provisioning
    /// app surfaces "Data format error" on this code.
    DataFormatError = 0x09,
}

/// Build the 3-byte `ReportWifiStatus` payload the firmware emits.
///
/// Spec wire layout: `[opmode, sta_conn_state, softap_conn_state]`,
/// optionally followed by TLVs (SSID, BSSID, channel) we don't ship.
/// SoftAP byte is always `0x00` since the firmware only runs STA.
#[must_use]
pub const fn build_report_wifi_status_payload(sta: WifiConnState) -> [u8; 3] {
    [WIFI_OP_MODE_STA, sta as u8, 0x00]
}

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
    /// Type byte's low 2 bits aren't one of the spec-assigned
    /// values (`0b00` Control / `0b01` Data). The other two
    /// values are unspecified by the BluFi protocol; treating
    /// them as `Data` would hide a malformed or future-format
    /// frame from the GATT handler.
    UnknownType,
    /// A frame with `FC_FRAGMENT` set carried fewer than the two
    /// bytes its leading total-content-length prefix needs. A
    /// conformant central always prefixes a fragmented frame's data
    /// with that `u16`, so a sub-2-byte payload is malformed.
    FragmentTooShort,
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
    let encrypted = frame_control & FC_ENCRYPTED != 0;
    let body_len = data_length + if has_crc { 2 } else { 0 };
    if 4 + body_len > buf.len() {
        return Err(ParseError::BadDataLength);
    }

    let data_end = 4 + data_length;
    let data = buf[4..data_end].to_vec();

    // Spec § 4: the CRC is computed over the *cleartext*
    // `sequence + data_length + data` before AES-CCM
    // encryption is applied to the data field. On encrypted
    // frames the `data` slice we see here is still ciphertext —
    // we can't verify the CRC until a future decryption layer
    // runs. Skip verification for encrypted frames and let the
    // consumer re-check after it decrypts; verify normally for
    // cleartext frames (which is everything in the foundation
    // slice).
    //
    // The CRC input — `sequence + data_length + data` — is
    // already contiguous in `buf` at `&buf[2..data_end]`, so
    // no temporary allocation is needed. Greptile flagged the
    // old `Vec`-based path as wasted heap on Xtensa.
    if has_crc && !encrypted {
        let expected = u16::from_le_bytes([buf[data_end], buf[data_end + 1]]);
        let computed = crc16_ccitt(&buf[2..data_end]);
        if computed != expected {
            return Err(ParseError::BadCrc);
        }
    }

    let frame_type = match type_byte & 0b11 {
        0b00 => Type::Control,
        0b01 => Type::Data,
        _ => return Err(ParseError::UnknownType),
    };
    let subtype = type_byte >> 2;

    Ok(Frame {
        frame_type,
        subtype,
        sequence,
        frame_control,
        data,
        fragmented: frame_control & FC_FRAGMENT != 0,
        encrypted,
    })
}

/// A fragmented frame's payload, split into its declared total
/// content length and the content bytes this frame contributes.
///
/// See [`fragment_content`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentContent<'a> {
    /// Total length of the fully-reassembled content across the
    /// whole fragment chain, taken from this frame's leading `u16`
    /// little-endian prefix. The receiver uses it to size its
    /// staging buffer and to know when the chain is complete.
    pub total_len: u16,
    /// This frame's slice of the content, borrowed from
    /// [`Frame::data`] with the 2-byte prefix removed.
    pub content: &'a [u8],
}

/// Strip a fragmented frame's leading total-content-length prefix.
///
/// Per the BluFi spec, every frame with `FC_FRAGMENT` set carries
/// `[total_content_length: u16 little-endian][content…]` in its
/// Data field; `content` is `Data Length - 2` bytes. The
/// *terminating* frame in a chain has `FC_FRAGMENT` clear and
/// carries **no** prefix — its data is the final content slice
/// verbatim, so this helper must not be called on it.
///
/// This is stateless: it reads one frame and returns its prefix +
/// content. Accumulating slices across the chain until the running
/// length reaches `total_len` is the GATT handler's job.
///
/// # Errors
///
/// Returns [`ParseError::FragmentTooShort`] when `frame.data` holds
/// fewer than the two bytes the prefix needs.
pub fn fragment_content(frame: &Frame) -> Result<FragmentContent<'_>, ParseError> {
    let Some((prefix, content)) = frame.data.split_first_chunk::<2>() else {
        return Err(ParseError::FragmentTooShort);
    };
    Ok(FragmentContent {
        total_len: u16::from_le_bytes(*prefix),
        content,
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
/// Fragment-follows bit set. Returns
/// [`BuildError::SubtypeOutOfRange`] when `subtype > 0x3F` —
/// only six bits are available on the wire, so a higher value
/// indicates a caller logic bug rather than data the wire could
/// silently truncate to a different subtype.
pub fn build_frame(
    frame_type: Type,
    subtype: u8,
    sequence: u8,
    data: &[u8],
) -> Result<Vec<u8>, BuildError> {
    if data.len() > u8::MAX as usize {
        return Err(BuildError::DataTooLong);
    }
    if subtype > 0x3F {
        return Err(BuildError::SubtypeOutOfRange);
    }
    let type_byte = (subtype << 2)
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

    // CRC over `sequence + data_length + data` — already
    // contiguous at `&out[2..]` now that the header + data is
    // written. No temporary allocation; Greptile flagged the
    // old `Vec`-based path as wasted heap on Xtensa.
    let crc = crc16_ccitt(&out[2..]);
    out.extend_from_slice(&crc.to_le_bytes());
    Ok(out)
}

/// Reasons [`build_frame`] can refuse to serialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildError {
    /// Data payload exceeds 255 bytes; needs fragmentation.
    DataTooLong,
    /// `subtype` exceeded the six-bit on-wire range
    /// (`> 0x3F`). Only the low 6 bits are encodable.
    SubtypeOutOfRange,
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
    fn fragment_content_splits_le_prefix_from_content() {
        // A fragmented SendStaSsid frame whose data is
        // [total_len_lo, total_len_hi, content…]. total_len = 0x0140
        // (320) — bigger than this frame's slice, as expected mid-chain.
        let mut buf = build_frame(
            Type::Data,
            DataSubtype::SendStaSsid as u8,
            0,
            &[0x40, 0x01, b'a', b'b', b'c'],
        )
        .unwrap();
        buf[1] |= FC_FRAGMENT;
        let frame = parse_frame(&buf).unwrap();
        let frag = fragment_content(&frame).unwrap();
        assert_eq!(frag.total_len, 0x0140);
        assert_eq!(frag.content, b"abc");
    }

    #[test]
    fn fragment_content_rejects_data_under_two_bytes() {
        let mut buf = build_frame(Type::Data, DataSubtype::SendStaSsid as u8, 0, &[0x07]).unwrap();
        buf[1] |= FC_FRAGMENT;
        let frame = parse_frame(&buf).unwrap();
        assert_eq!(fragment_content(&frame), Err(ParseError::FragmentTooShort));
    }

    #[test]
    fn fragmented_then_terminating_frame_round_trips() {
        // Two-frame chain reassembling "hello-world-network" (19 bytes).
        // Frame A: fragmented, prefix = total_len, carries "hello-world".
        // Frame B: terminating (FC_FRAGMENT clear, no prefix), carries
        // "-network".
        let payload = b"hello-world-network";
        let total_len = u16::try_from(payload.len()).unwrap();
        let head = &payload[..11];
        let tail = &payload[11..];

        let mut a_data = total_len.to_le_bytes().to_vec();
        a_data.extend_from_slice(head);
        let mut a_buf =
            build_frame(Type::Data, DataSubtype::SendStaSsid as u8, 0, &a_data).unwrap();
        a_buf[1] |= FC_FRAGMENT;
        let frame_a = parse_frame(&a_buf).unwrap();
        let frag_a = fragment_content(&frame_a).unwrap();

        let b_buf = build_frame(Type::Data, DataSubtype::SendStaSsid as u8, 1, tail).unwrap();
        let frame_b = parse_frame(&b_buf).unwrap();
        assert!(!frame_b.fragmented);

        let mut assembled = frag_a.content.to_vec();
        assembled.extend_from_slice(&frame_b.data);
        assert_eq!(assembled, payload);
        assert_eq!(assembled.len(), frag_a.total_len as usize);
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
    fn build_frame_rejects_subtype_above_6_bit_range() {
        // Only 6 bits of the type byte carry the subtype; a caller
        // passing `0x40` would have its low 6 bits (`0`) silently
        // emitted as subtype 0 (== Ack/NegotiationData) without
        // any error. Reject explicitly so the bug surfaces at the
        // call site, not on the wire.
        assert_eq!(
            build_frame(Type::Control, 0x40, 0, &[]),
            Err(BuildError::SubtypeOutOfRange)
        );
        assert_eq!(
            build_frame(Type::Data, 0xFF, 0, &[]),
            Err(BuildError::SubtypeOutOfRange)
        );
        // The largest legal subtype (`0x3F`) still encodes.
        assert!(build_frame(Type::Control, 0x3F, 0, &[]).is_ok());
    }

    #[test]
    fn parse_frame_skips_crc_verification_on_encrypted_frames() {
        // Greptile-caught: spec says CRC is over cleartext
        // sequence + data_length + data, computed before
        // AES-CCM encryption. Once encryption lands, the `data`
        // bytes on the wire are ciphertext — the parser can't
        // verify CRC against them, only against the post-decrypt
        // cleartext. So when FC_ENCRYPTED is set, CRC
        // verification is deferred to the consumer (which has the
        // key + can decrypt). The frame must still parse cleanly,
        // not get dropped as `BadCrc`, so the post-handshake
        // traffic from the official provisioning app survives.
        let buf = [
            // Type: Data + Send STA SSID.
            ((DataSubtype::SendStaSsid as u8) << 2) | 0x01,
            // FC: encrypted + checksum + direction-central-to-device.
            FC_ENCRYPTED | FC_CHECKSUM,
            0x42, // Sequence
            0x04, // Data length
            0xDE,
            0xAD,
            0xBE,
            0xEF, // "Ciphertext"
            0x00,
            0x00, // Bogus CRC — would otherwise reject.
        ];
        let parsed = parse_frame(&buf).expect("encrypted frame must parse despite a bogus CRC");
        assert!(parsed.encrypted);
        assert_eq!(parsed.data, [0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn parse_frame_rejects_undefined_type_bits() {
        // Greptile-caught: the low 2 bits of the Type byte have
        // four possible values but only `0b00` (Control) and
        // `0b01` (Data) are spec-assigned. The catch-all that
        // mapped everything else to Data was hiding malformed or
        // future-format frames from the GATT handler.
        let mut buf = build_frame(Type::Data, DataSubtype::SendStaSsid as u8, 0, b"x").unwrap();
        // Stamp the low bits to `0b10` (unassigned).
        buf[0] = (buf[0] & 0b1111_1100) | 0b10;
        // The fixed CRC bytes still match the original Type byte's
        // CRC; clear the checksum bit on FrameCtrl so we don't
        // fail CRC verification before reaching the type check.
        buf[1] &= !FC_CHECKSUM;
        // Trim the trailing CRC bytes since we just disabled it.
        buf.truncate(buf.len() - 2);
        assert_eq!(parse_frame(&buf), Err(ParseError::UnknownType));

        // `0b11` should also reject.
        buf[0] = (buf[0] & 0b1111_1100) | 0b11;
        assert_eq!(parse_frame(&buf), Err(ParseError::UnknownType));
    }

    #[test]
    fn report_wifi_status_payload_pins_byte_layout() {
        // STA mode + connected: ESP-IDF spec expects opmode in byte 0,
        // STA conn state in byte 1, SoftAP conn state in byte 2.
        assert_eq!(
            build_report_wifi_status_payload(WifiConnState::Connected),
            [0x01, 0x00, 0x00]
        );
        assert_eq!(
            build_report_wifi_status_payload(WifiConnState::NotConnected),
            [0x01, 0x01, 0x00]
        );
    }

    #[test]
    fn report_wifi_status_round_trips_through_build_frame() {
        // The full outbound frame the firmware sends on commit:
        // Data subtype `ReportWifiStatus` with the 3-byte payload.
        let payload = build_report_wifi_status_payload(WifiConnState::Connected);
        let built =
            build_frame(Type::Data, DataSubtype::ReportWifiStatus as u8, 0, &payload).unwrap();
        let parsed = parse_frame(&built).unwrap();
        assert_eq!(parsed.data_subtype(), Some(DataSubtype::ReportWifiStatus));
        assert_eq!(parsed.data, payload);
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

    #[test]
    fn control_subtype_decodes_every_known_value() {
        // Iterate every byte the spec assigns to a control subtype.
        // Catches a future re-numbering or accidental gap.
        let mapping: &[(u8, ControlSubtype)] = &[
            (0x00, ControlSubtype::Ack),
            (0x01, ControlSubtype::SetSecMode),
            (0x02, ControlSubtype::SetWifiOpMode),
            (0x03, ControlSubtype::ConnectToAp),
            (0x04, ControlSubtype::DisconnectFromAp),
            (0x05, ControlSubtype::GetWifiStatus),
            (0x06, ControlSubtype::DeauthSta),
            (0x07, ControlSubtype::GetVersion),
            (0x08, ControlSubtype::Disconnect),
            (0x09, ControlSubtype::GetWifiList),
        ];
        for (byte, expected) in mapping {
            let f = Frame {
                frame_type: Type::Control,
                subtype: *byte,
                sequence: 0,
                frame_control: 0,
                data: Vec::new(),
                fragmented: false,
                encrypted: false,
            };
            assert_eq!(
                f.control_subtype(),
                Some(*expected),
                "byte {byte:#x} should decode to {expected:?}",
            );
        }
    }

    #[test]
    fn control_subtype_returns_none_on_data_frame() {
        // Even with a valid control byte, a frame whose type isn't
        // Control must surface None — the type-class check happens
        // before the byte switch.
        let f = Frame {
            frame_type: Type::Data,
            subtype: 0x00, // would be Ack on a Control frame
            sequence: 0,
            frame_control: 0,
            data: Vec::new(),
            fragmented: false,
            encrypted: false,
        };
        assert_eq!(f.control_subtype(), None);
    }

    #[test]
    fn data_subtype_decodes_every_known_value() {
        let mapping: &[(u8, DataSubtype)] = &[
            (0x00, DataSubtype::NegotiationData),
            (0x01, DataSubtype::SendStaBssid),
            (0x02, DataSubtype::SendStaSsid),
            (0x03, DataSubtype::SendStaPassword),
            (0x04, DataSubtype::SendSoftApSsid),
            (0x05, DataSubtype::SendSoftApPassword),
            (0x06, DataSubtype::SendSoftApMaxConn),
            (0x07, DataSubtype::SendSoftApAuthMode),
            (0x08, DataSubtype::SendSoftApChannel),
            (0x09, DataSubtype::SendUsername),
            (0x0A, DataSubtype::SendCaCert),
            (0x0B, DataSubtype::SendClientCert),
            (0x0C, DataSubtype::SendServerCert),
            (0x0D, DataSubtype::SendClientPrivateKey),
            (0x0E, DataSubtype::SendServerPrivateKey),
            (0x0F, DataSubtype::ReportWifiStatus),
            (0x10, DataSubtype::ReportWifiList),
            (0x11, DataSubtype::Error),
            (0x12, DataSubtype::CustomData),
            (0x13, DataSubtype::SetMaxFragment),
        ];
        for (byte, expected) in mapping {
            let f = Frame {
                frame_type: Type::Data,
                subtype: *byte,
                sequence: 0,
                frame_control: 0,
                data: Vec::new(),
                fragmented: false,
                encrypted: false,
            };
            assert_eq!(
                f.data_subtype(),
                Some(*expected),
                "byte {byte:#x} should decode to {expected:?}",
            );
        }
    }

    #[test]
    fn data_subtype_returns_none_for_unknown_byte_and_non_data_frame() {
        // Unknown byte: 0x7F isn't assigned.
        let f = Frame {
            frame_type: Type::Data,
            subtype: 0x7F,
            sequence: 0,
            frame_control: 0,
            data: Vec::new(),
            fragmented: false,
            encrypted: false,
        };
        assert_eq!(f.data_subtype(), None);
        // Non-data frame: even a valid byte returns None.
        let f = Frame {
            frame_type: Type::Control,
            subtype: 0x00,
            sequence: 0,
            frame_control: 0,
            data: Vec::new(),
            fragmented: false,
            encrypted: false,
        };
        assert_eq!(f.data_subtype(), None);
    }
}
