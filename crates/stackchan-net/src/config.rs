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
    /// CoreS3 lens is roughly 62°. Range: 1..=180.
    pub fov_h_deg: f32,
    /// Camera vertical field of view in degrees. ~49° on the same
    /// lens. Range: 1..=180.
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
    Ok(())
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
}
