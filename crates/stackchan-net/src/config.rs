//! Schema v1 of the Stack-chan RON config — `wifi`, `mdns`, `time`.
//!
//! The data types are always available (`no_std` + `alloc`, no extra
//! deps). The `serde` derives, [`parse_ron`], and [`render_ron`] are
//! gated behind the `parse` feature — host builds enable it, the
//! firmware target does not because `ron 0.10` hard-pins
//! `serde/std + base64/std` which are broken on
//! `xtensa-esp32s3-none-elf`. Firmware does its own hand-rolled
//! RON parsing (and produces these same types).

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

#[cfg(feature = "parse")]
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// Top-level on-disk config.
///
/// Defaults are tuned for offline-first boot: an empty SSID is a
/// no-op at the Wi-Fi layer, hostname `"stackchan"` is the canonical
/// mDNS label, `time` points at `pool.ntp.org` so SNTP picks up
/// once Wi-Fi is configured, and `auth.token` is empty so the HTTP
/// control plane stays LAN-open until the operator opts in.
#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "parse", derive(Serialize, Deserialize))]
pub struct Config {
    /// Wi-Fi station credentials and regulatory country code.
    pub wifi: WifiConfig,
    /// Local hostname advertised on `.local` via mDNS.
    pub mdns: MdnsConfig,
    /// Timezone label + SNTP server list.
    pub time: TimeConfig,
    /// HTTP control-plane authentication. Empty token = auth
    /// disabled (current LAN-open behaviour); non-empty token gates
    /// `PUT`/`POST` routes behind `Authorization: Bearer <token>`.
    #[cfg_attr(feature = "parse", serde(default))]
    pub auth: AuthConfig,
    /// Audio output: persistent volume + mute state. Mirrored to the
    /// AW88298 amplifier on boot and on every `POST /volume` / `POST
    /// /mute` write.
    #[cfg_attr(feature = "parse", serde(default))]
    pub audio: AudioConfig,
    /// Camera-tracker tuning: lens FOV, target smoothing, and
    /// orientation flips. Applied to the firmware tracker at boot;
    /// changes via `PUT /settings` take effect on the next boot
    /// (mirrors the mDNS / SNTP / audio-init pattern).
    #[cfg_attr(feature = "parse", serde(default))]
    pub tracker: TrackerSettings,
    /// ESP-NOW remote-control radio settings. Default: disabled. With
    /// `enabled = true` the firmware spawns the inbound RX task and the
    /// optional outbound heartbeat at `tx_rate_hz`. When `peer_mac` is
    /// non-empty the address is registered statically at boot;
    /// otherwise peer registration only happens during a `POST /pair`
    /// pairing window.
    #[cfg_attr(feature = "parse", serde(default))]
    pub esp_now: EspNowConfig,
}

/// Wi-Fi station credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "parse", derive(Serialize, Deserialize))]
pub struct WifiConfig {
    /// SSID of the access point to join. An empty string disables the
    /// Wi-Fi join attempt entirely (avatar runs offline-first).
    pub ssid: String,
    /// WPA2/WPA3 pre-shared key. Empty string permitted for open APs.
    pub psk: String,
    /// ISO-3166 alpha-2 country code. Default `"US"`. Determines
    /// channel availability and TX power per regulatory domain.
    pub country: String,
}

impl Default for WifiConfig {
    fn default() -> Self {
        Self {
            ssid: String::new(),
            psk: String::new(),
            country: "US".to_string(),
        }
    }
}

/// mDNS hostname configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "parse", derive(Serialize, Deserialize))]
pub struct MdnsConfig {
    /// Hostname advertised on `.local`. Default `"stackchan"` →
    /// device reachable as `stackchan.local`.
    pub hostname: String,
}

impl Default for MdnsConfig {
    fn default() -> Self {
        Self {
            hostname: "stackchan".to_string(),
        }
    }
}

/// HTTP control-plane authentication.
///
/// Default `token` is the empty string, which leaves the HTTP plane
/// LAN-open (matching the offline-first stance for Wi-Fi). Setting
/// a non-empty token requires `Authorization: Bearer <token>` on
/// `PUT`/`POST` routes; reads stay unauthenticated.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "parse", derive(Serialize, Deserialize))]
pub struct AuthConfig {
    /// Shared-secret bearer token. Empty = auth disabled.
    pub token: String,
}

/// Audio output configuration — persistent volume + mute state.
///
/// `volume_pct` is on the wire as an integer 0..=100 to keep the
/// operator-facing surface intuitive; the firmware maps it linearly
/// across the AW88298's dB range when applying to the amp. `0` is
/// audible-but-quiet, not silent — explicit `muted: true` is the
/// actual-silence path. Default `volume_pct = 50` lands at roughly
/// the chip's prior compile-time boot default; default `muted =
/// false` matches the behaviour the firmware shipped with before
/// runtime audio control existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "parse", derive(Serialize, Deserialize))]
pub struct AudioConfig {
    /// Output volume as a percentile (0..=100). Mapped linearly over
    /// dB by the firmware before being written to the amp.
    pub volume_pct: u8,
    /// Whether the output stage is muted. Independent of
    /// `volume_pct` so unmuting restores the prior level.
    pub muted: bool,
}

impl AudioConfig {
    /// Const-evaluable default. Exposed so static initializers (e.g.
    /// the firmware's `AvatarSnapshot` constant) can reference the
    /// canonical defaults without duplicating the literals — `Default`
    /// itself isn't `const`-evaluable.
    pub const DEFAULT: Self = Self {
        volume_pct: 50,
        muted: false,
    };
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Operator-tunable subset of `tracker::TrackerConfig`.
///
/// These are the fields most likely to need adjustment in the field
/// rather than at compile time. Lens FOV varies between hardware
/// revisions, smoothing is a taste call, and orientation flips depend
/// on physical mounting.
///
/// The full `tracker::TrackerConfig` algorithm tuning (P-gain, block
/// thresholds, dead zones, idle behaviour, ...) stays compile-time
/// only — operators shouldn't need it, and exposing it would balloon
/// the schema for marginal value.
///
/// Defaults match `tracker::TrackerConfig::DEFAULT` so a missing
/// `tracker:` block reproduces the firmware's pre-runtime-config
/// behaviour exactly.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "parse", derive(Serialize, Deserialize))]
pub struct TrackerSettings {
    /// Camera horizontal field of view in degrees. GC0308 with the
    /// CoreS3 lens is roughly 62°. Range: `(0, 180]` — must be
    /// strictly positive and at most 180°.
    pub fov_h_deg: f32,
    /// Camera vertical field of view in degrees. ~49° on the same
    /// lens. Range: `(0, 180]` — same gate as `fov_h_deg`.
    pub fov_v_deg: f32,
    /// Single-pole EMA on the published target pose. `1.0` is the
    /// pass-through default; lower values add inertia. Range:
    /// 0.05..=1.0 (clamped at the use site as defence-in-depth).
    pub target_smoothing_alpha: f32,
    /// Mirror the centroid horizontally before mapping to pan. Set
    /// when the camera is mounted left-right reversed relative to the
    /// head's pan direction.
    pub flip_x: bool,
    /// Mirror vertically. Set when the camera image is upside-down
    /// relative to the head's tilt direction.
    pub flip_y: bool,
}

impl TrackerSettings {
    /// Const-evaluable default mirroring `tracker::TrackerConfig::DEFAULT`.
    pub const DEFAULT: Self = Self {
        fov_h_deg: 62.0,
        fov_v_deg: 49.0,
        target_smoothing_alpha: 1.0,
        flip_x: false,
        flip_y: false,
    };
}

impl Default for TrackerSettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// ESP-NOW remote-control radio settings.
///
/// ESP-NOW is a connectionless 2.4 GHz protocol layered on top of
/// 802.11 — peers agree on a Wi-Fi channel without ever associating.
/// `channel = None` is the standard mode where this device follows
/// whatever channel the Wi-Fi STA picked when it associated; setting
/// `channel = Some(n)` is for ESP-NOW-only operation without
/// joining a Wi-Fi network.
///
/// All on-disk hex strings are case-insensitive and accept either bare
/// hex (`"aabbccddeeff"`) or RFC-style colon-delimited MAC
/// (`"aa:bb:cc:dd:ee:ff"`). Empty strings are sentinels for "not
/// configured" — see field docs for how each field treats them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "parse", derive(Serialize, Deserialize))]
pub struct EspNowConfig {
    /// Master switch. Default `false`; the firmware skips ESP-NOW init
    /// entirely. Setting to `true` requires either a non-empty
    /// `peer_mac` (static peer) or an operator-driven
    /// [`crate::http_command::parse_enter_pairing`] window before any
    /// frames are accepted.
    pub enabled: bool,
    /// Pre-master key as a 32-character hex string (16 bytes). Empty
    /// disables encryption — fine for development on a private LAN,
    /// but expose AES-CCM by setting a real PMK before shipping a
    /// device with `enabled = true` to a less-trusted environment.
    pub pmk_hex: String,
    /// Peer's MAC address. Either empty (no static peer; operator
    /// must use the pairing window), bare 12 hex chars, or
    /// colon-delimited (`xx:xx:xx:xx:xx:xx`). Case-insensitive.
    pub peer_mac: String,
    /// Local master key for the static peer. Same hex shape as
    /// `pmk_hex`. Required if `peer_mac` is non-empty AND `pmk_hex`
    /// is non-empty (encryption enabled with a static peer); empty
    /// in dev mode.
    pub lmk_hex: String,
    /// Wi-Fi channel to lock the radio to. `None` follows the Wi-Fi
    /// STA's chosen channel — the standard mode where ESP-NOW
    /// piggybacks on the Wi-Fi association. `Some(n)` (1..=14) is
    /// for ESP-NOW-only deployments without Wi-Fi association.
    pub channel: Option<u8>,
    /// Outbound heartbeat rate in Hz. `0` disables outbound entirely
    /// (RX-only). Range gated to `0..=20`. Default `5`.
    pub tx_rate_hz: u8,
}

impl EspNowConfig {
    /// Const-evaluable default: disabled, no peer, no encryption,
    /// channel follows STA, 5 Hz heartbeat (a no-op while disabled).
    pub const DEFAULT: Self = Self {
        enabled: false,
        pmk_hex: String::new(),
        peer_mac: String::new(),
        lmk_hex: String::new(),
        channel: None,
        tx_rate_hz: 5,
    };
}

impl Default for EspNowConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Time / SNTP configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "parse", derive(Serialize, Deserialize))]
pub struct TimeConfig {
    /// IANA timezone label (e.g. `"UTC"`, `"America/Los_Angeles"`).
    /// Currently parsed but unused — the BM8563 RTC stores UTC.
    pub tz: String,
    /// SNTP servers to query in order. The firmware tries each with
    /// a 5-second timeout before falling back to the next.
    pub sntp_servers: Vec<String>,
}

impl Default for TimeConfig {
    fn default() -> Self {
        Self {
            tz: "UTC".to_string(),
            sntp_servers: vec!["pool.ntp.org".to_string()],
        }
    }
}

/// Parse + validate a RON document into a [`Config`].
///
/// # Errors
///
/// Returns [`ConfigError::Parse`] on malformed RON, or one of the
/// validation variants ([`ConfigError::EmptySsid`],
/// [`ConfigError::InvalidCountry`], [`ConfigError::InvalidHostname`],
/// [`ConfigError::NoSntpServers`]) on out-of-range values.
#[cfg(feature = "parse")]
pub fn parse_ron(input: &str) -> Result<Config, ConfigError> {
    let config: Config = ron::from_str(input)?;
    validate(&config)?;
    Ok(config)
}

/// Render a [`Config`] back to a pretty-printed RON string.
///
/// Used to persist user changes back to SD, and as the round-trip
/// pair to [`parse_ron`]. **Not directly safe for unauthed network
/// readback** — see Security below.
///
/// # Security
///
/// This serializer faithfully renders every field, including
/// `wifi.psk`. Any caller that exposes the output over an unauthed
/// channel must redact the PSK on the read path (separate read/write
/// DTOs, or a masked-render variant). The `parse_ron` ↔ `render_ron`
/// round trip is preserved here so SD reads/writes stay lossless.
///
/// # Errors
///
/// Returns [`ConfigError::Serialize`] on serializer failure. Should
/// not happen with a well-formed [`Config`].
#[cfg(feature = "parse")]
pub fn render_ron(config: &Config) -> Result<String, ConfigError> {
    let pretty = ron::ser::PrettyConfig::new();
    ron::ser::to_string_pretty(config, pretty).map_err(ConfigError::Serialize)
}

/// Run the v1 schema validators against a [`Config`].
///
/// Public so firmware-side parsers can reuse the same gate the
/// `parse_ron` host path runs. The firmware wraps any failure in
/// `defmt::Debug2Format` for logging.
///
/// # Errors
///
/// Returns one of the validation variants
/// ([`ConfigError::EmptySsid`], [`ConfigError::InvalidCountry`],
/// [`ConfigError::InvalidHostname`], [`ConfigError::NoSntpServers`],
/// [`ConfigError::EmptySntpServer`]) on out-of-range values.
pub fn validate(config: &Config) -> Result<(), ConfigError> {
    // SSID: empty *file value* is rejected. `Config::default()` uses
    // an empty SSID as a sentinel for "no wifi configured" and never
    // routes through this validator.
    if config.wifi.ssid.trim().is_empty() {
        return Err(ConfigError::EmptySsid);
    }
    if !is_valid_country(&config.wifi.country) {
        return Err(ConfigError::InvalidCountry(config.wifi.country.clone()));
    }
    if !is_valid_hostname(&config.mdns.hostname) {
        return Err(ConfigError::InvalidHostname(config.mdns.hostname.clone()));
    }
    if config.time.sntp_servers.is_empty() {
        return Err(ConfigError::NoSntpServers);
    }
    if let Some(idx) = config
        .time
        .sntp_servers
        .iter()
        .position(|s| s.trim().is_empty())
    {
        return Err(ConfigError::EmptySntpServer(idx));
    }
    if config.audio.volume_pct > 100 {
        return Err(ConfigError::InvalidVolumePct(config.audio.volume_pct));
    }
    if !is_valid_fov_deg(config.tracker.fov_h_deg) {
        return Err(ConfigError::InvalidFovDeg(config.tracker.fov_h_deg));
    }
    if !is_valid_fov_deg(config.tracker.fov_v_deg) {
        return Err(ConfigError::InvalidFovDeg(config.tracker.fov_v_deg));
    }
    if !is_valid_smoothing_alpha(config.tracker.target_smoothing_alpha) {
        return Err(ConfigError::InvalidSmoothingAlpha(
            config.tracker.target_smoothing_alpha,
        ));
    }
    validate_esp_now(&config.esp_now)?;
    Ok(())
}

/// Run the ESP-NOW sub-block validators against [`EspNowConfig`].
///
/// Empty hex strings short-circuit — they're sentinels for "not set".
/// Non-empty strings must match the documented shape (32 hex chars for
/// keys, 12 raw or colon-delimited for the MAC).
///
/// # Errors
///
/// Returns one of [`ConfigError::InvalidEspNowKey`],
/// [`ConfigError::InvalidEspNowMac`],
/// [`ConfigError::InvalidEspNowChannel`], or
/// [`ConfigError::InvalidEspNowTxRate`] on out-of-range values.
fn validate_esp_now(c: &EspNowConfig) -> Result<(), ConfigError> {
    if !is_redacted_or_empty(&c.pmk_hex) && !is_valid_hex_key(&c.pmk_hex) {
        return Err(ConfigError::InvalidEspNowKey("pmk_hex"));
    }
    if !is_redacted_or_empty(&c.lmk_hex) && !is_valid_hex_key(&c.lmk_hex) {
        return Err(ConfigError::InvalidEspNowKey("lmk_hex"));
    }
    if !c.peer_mac.is_empty() && parse_mac(&c.peer_mac).is_none() {
        return Err(ConfigError::InvalidEspNowMac(c.peer_mac.clone()));
    }
    // Encrypted-static-peer requires both PMK + LMK. Pinning here so
    // an inconsistent triple (peer_mac + pmk set, lmk left blank)
    // can't drop the firmware into a silent unencrypted-unicast
    // fallback.
    if !c.peer_mac.is_empty() && !is_redacted_or_empty(&c.pmk_hex) && c.lmk_hex.is_empty() {
        return Err(ConfigError::InvalidEspNowKey("lmk_hex"));
    }
    if let Some(ch) = c.channel
        && !(1..=14).contains(&ch)
    {
        return Err(ConfigError::InvalidEspNowChannel(ch));
    }
    if c.tx_rate_hz > 20 {
        return Err(ConfigError::InvalidEspNowTxRate(c.tx_rate_hz));
    }
    Ok(())
}

/// True iff the value is empty or matches the `***` redaction sentinel
/// (mirroring `wifi.psk` / `auth.token` behaviour). The sentinel
/// flows through to the downstream
/// [`crate::bare_json::merge_settings_with_current`] step, which
/// substitutes the persisted value back in.
fn is_redacted_or_empty(s: &str) -> bool {
    s.is_empty() || s == "***"
}

/// True iff `s` is exactly 32 case-insensitive hex characters
/// (16 bytes — the ESP-NOW PMK / LMK size).
fn is_valid_hex_key(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Parse a MAC address from either `xx:xx:xx:xx:xx:xx` or bare 12-hex
/// form. Returns the raw 6 bytes on success, `None` on any structural
/// mismatch. Case-insensitive on the hex digits.
#[must_use]
pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let bytes = s.as_bytes();
    let valid = match bytes.len() {
        12 => bytes.iter().all(u8::is_ascii_hexdigit),
        17 => {
            // `xx:xx:xx:xx:xx:xx` — colons at positions 2, 5, 8, 11, 14.
            for i in [2_usize, 5, 8, 11, 14] {
                if bytes[i] != b':' {
                    return None;
                }
            }
            (0..17).all(|i| matches!(i, 2 | 5 | 8 | 11 | 14) || bytes[i].is_ascii_hexdigit())
        }
        _ => return None,
    };
    if !valid {
        return None;
    }
    let mut out = [0_u8; 6];
    let mut hex = [0_u8; 12];
    if bytes.len() == 12 {
        hex.copy_from_slice(bytes);
    } else {
        // Skip the colons at 2/5/8/11/14.
        let mut j = 0;
        for (i, &b) in bytes.iter().enumerate() {
            if matches!(i, 2 | 5 | 8 | 11 | 14) {
                continue;
            }
            hex[j] = b;
            j += 1;
        }
    }
    for (i, byte) in out.iter_mut().enumerate() {
        let hi = hex_nibble(hex[i * 2])?;
        let lo = hex_nibble(hex[i * 2 + 1])?;
        *byte = (hi << 4) | lo;
    }
    Some(out)
}

/// Convert one ASCII hex digit to its numeric value (0..=15).
const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// True iff `deg` is a finite positive value within `(0, 180]`. Lens
/// FOVs outside that range can't be physical; rejecting at the
/// validator catches typos before they reach the camera task.
fn is_valid_fov_deg(deg: f32) -> bool {
    deg.is_finite() && deg > 0.0 && deg <= 180.0
}

/// True iff `alpha` is a finite value in `[0.05, 1.0]`. Lower than
/// 0.05 effectively freezes the published target for tens of seconds,
/// which is a UX bug; higher than 1.0 has no defined meaning for an
/// EMA. Matches the runtime clamp inside `Tracker::publish`.
fn is_valid_smoothing_alpha(alpha: f32) -> bool {
    alpha.is_finite() && (0.05..=1.0).contains(&alpha)
}

/// True iff `s` is exactly two uppercase ASCII letters (ISO-3166
/// alpha-2). esp-wifi's regulatory-domain API expects the canonical
/// uppercase form (`"US"`, `"JP"`); a lowercase value would silently
/// pass an alphabetic check and then mis-apply the channel/TX mask
/// at the driver layer, so the validator pins the case here.
fn is_valid_country(s: &str) -> bool {
    s.len() == 2 && s.bytes().all(|b| b.is_ascii_uppercase())
}

/// True iff `s` is an RFC-952 subset hostname: ASCII letters / digits
/// / hyphens, must start with a letter, must not end with a hyphen,
/// length 1-63.
fn is_valid_hostname(s: &str) -> bool {
    if s.is_empty() || s.len() > 63 {
        return false;
    }
    let bytes = s.as_bytes();
    if !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    if bytes[bytes.len() - 1] == b'-' {
        return false;
    }
    bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'-')
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test-only: unwrap on Option / Result is the standard test idiom for asserting parse success"
)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_offline_first() {
        let c = Config::default();
        assert!(c.wifi.ssid.is_empty(), "empty SSID = no Wi-Fi attempt");
        assert_eq!(c.wifi.country, "US");
        assert_eq!(c.mdns.hostname, "stackchan");
        assert_eq!(c.time.tz, "UTC");
        assert_eq!(c.time.sntp_servers, vec!["pool.ntp.org".to_string()]);
        assert_eq!(c.audio.volume_pct, 50);
        assert!(!c.audio.muted);
    }

    #[test]
    fn validate_rejects_volume_above_100() {
        let mut c = Config::default();
        c.wifi.ssid = "x".to_string();
        c.audio.volume_pct = 101;
        assert!(matches!(
            validate(&c),
            Err(ConfigError::InvalidVolumePct(101))
        ));
    }

    #[test]
    fn validate_accepts_volume_at_boundaries() {
        let mut c = Config::default();
        c.wifi.ssid = "x".to_string();
        for pct in [0u8, 1, 50, 99, 100] {
            c.audio.volume_pct = pct;
            assert!(validate(&c).is_ok(), "expected pct={pct} to pass");
        }
    }

    #[test]
    fn validates_country_length_and_case() {
        assert!(is_valid_country("US"));
        assert!(is_valid_country("JP"));
        assert!(!is_valid_country("USA"));
        assert!(!is_valid_country("U"));
        assert!(!is_valid_country(""));
        assert!(!is_valid_country("U1"));
        assert!(!is_valid_country("us"));
        assert!(!is_valid_country("jp"));
        assert!(!is_valid_country("Us"));
    }

    #[test]
    fn validates_hostname_rfc952_subset() {
        assert!(is_valid_hostname("stackchan"));
        assert!(is_valid_hostname("stackchan-01"));
        assert!(is_valid_hostname("a"));
        assert!(!is_valid_hostname(&"a".repeat(64)));
        assert!(!is_valid_hostname(""));
        assert!(!is_valid_hostname("1stackchan"));
        assert!(!is_valid_hostname("-stackchan"));
        assert!(!is_valid_hostname("stackchan-"));
        assert!(!is_valid_hostname("stack_chan"));
    }

    #[test]
    fn esp_now_default_is_disabled_and_empty() {
        let c = EspNowConfig::default();
        assert!(!c.enabled);
        assert!(c.pmk_hex.is_empty());
        assert!(c.peer_mac.is_empty());
        assert!(c.lmk_hex.is_empty());
        assert!(c.channel.is_none());
        assert_eq!(c.tx_rate_hz, 5);
    }

    #[test]
    fn parse_mac_accepts_both_formats() {
        let bare = parse_mac("aabbccddeeff").unwrap();
        let colon = parse_mac("aa:bb:cc:dd:ee:ff").unwrap();
        let upper = parse_mac("AA:BB:CC:DD:EE:FF").unwrap();
        assert_eq!(bare, [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
        assert_eq!(bare, colon);
        assert_eq!(bare, upper);
    }

    #[test]
    fn parse_mac_rejects_garbage() {
        assert!(parse_mac("").is_none());
        assert!(parse_mac("not-a-mac").is_none());
        assert!(parse_mac("aa:bb:cc:dd:ee").is_none()); // too short
        assert!(parse_mac("aa:bb:cc:dd:ee:ff:gg").is_none()); // too long
        assert!(parse_mac("zz:bb:cc:dd:ee:ff").is_none()); // bad hex
        assert!(parse_mac("aabbccddeefz").is_none()); // bad hex bare
    }

    #[test]
    fn validate_rejects_bad_pmk() {
        let mut c = Config::default();
        c.wifi.ssid = "x".to_string();
        c.esp_now.pmk_hex = "tooshort".to_string();
        assert!(matches!(
            validate(&c),
            Err(ConfigError::InvalidEspNowKey("pmk_hex"))
        ));
    }

    #[test]
    fn validate_rejects_bad_peer_mac() {
        let mut c = Config::default();
        c.wifi.ssid = "x".to_string();
        c.esp_now.peer_mac = "not-a-mac".to_string();
        assert!(matches!(
            validate(&c),
            Err(ConfigError::InvalidEspNowMac(_))
        ));
    }

    #[test]
    fn validate_rejects_channel_out_of_range() {
        let mut c = Config::default();
        c.wifi.ssid = "x".to_string();
        c.esp_now.channel = Some(15);
        assert!(matches!(
            validate(&c),
            Err(ConfigError::InvalidEspNowChannel(15))
        ));
    }

    #[test]
    fn validate_rejects_tx_rate_too_high() {
        let mut c = Config::default();
        c.wifi.ssid = "x".to_string();
        c.esp_now.tx_rate_hz = 21;
        assert!(matches!(
            validate(&c),
            Err(ConfigError::InvalidEspNowTxRate(21))
        ));
    }

    #[test]
    fn validate_rejects_static_peer_with_pmk_but_no_lmk() {
        // peer_mac + pmk_hex set without lmk_hex would silently
        // register an encrypted peer with no per-peer key. Reject.
        let mut c = Config::default();
        c.wifi.ssid = "x".to_string();
        c.esp_now.peer_mac = "aa:bb:cc:dd:ee:ff".to_string();
        c.esp_now.pmk_hex = "0123456789abcdef0123456789abcdef".to_string();
        // lmk_hex left empty.
        assert!(matches!(
            validate(&c),
            Err(ConfigError::InvalidEspNowKey("lmk_hex"))
        ));
    }

    #[test]
    fn validate_accepts_static_peer_with_no_encryption() {
        // peer_mac + lmk + pmk all empty — open-mode static peer is
        // valid. Only the (peer + pmk + no-lmk) triple is rejected.
        let mut c = Config::default();
        c.wifi.ssid = "x".to_string();
        c.esp_now.peer_mac = "aa:bb:cc:dd:ee:ff".to_string();
        assert!(validate(&c).is_ok());
    }

    #[test]
    fn validate_accepts_well_formed_esp_now() {
        let mut c = Config::default();
        c.wifi.ssid = "x".to_string();
        c.esp_now.enabled = true;
        c.esp_now.pmk_hex = "0123456789abcdef0123456789abcdef".to_string();
        c.esp_now.peer_mac = "aa:bb:cc:dd:ee:ff".to_string();
        c.esp_now.lmk_hex = "fedcba9876543210fedcba9876543210".to_string();
        c.esp_now.channel = Some(6);
        c.esp_now.tx_rate_hz = 5;
        assert!(validate(&c).is_ok());
    }
}
