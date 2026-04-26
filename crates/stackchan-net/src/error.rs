//! Error types for RON config parsing and validation.

use alloc::string::String;

/// Parse / validate failure for [`crate::Config`] round-trips.
///
/// Variants carry the offending value where it aids debugging — the
/// firmware logs these via `defmt::Debug2Format`, and the catalog at
/// `docs/errors.md` mirrors the same per-variant guidance.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// RON deserialize failure — syntax error, missing field, or
    /// type mismatch. The wrapped [`ron::error::SpannedError`] carries
    /// `(line, col)` so callers can surface a precise diagnostic.
    #[error("RON parse error: {0}")]
    Parse(#[from] ron::error::SpannedError),

    /// RON serialize failure on round-trip. Should not happen with a
    /// well-formed [`crate::Config`]; treat as a bug if observed.
    #[error("RON serialize error: {0}")]
    Serialize(#[from] ron::Error),

    /// `wifi.ssid` was empty or whitespace-only after trim. The
    /// firmware treats an empty SSID as "no Wi-Fi configured" via
    /// `WifiConfig::default`, but an explicitly-blank value in the
    /// file is almost always a mistake.
    #[error("wifi.ssid is empty or whitespace-only")]
    EmptySsid,

    /// `wifi.country` was not exactly two ASCII letters. ESP-WIFI
    /// expects an ISO-3166 alpha-2 country code (e.g. `"US"`, `"JP"`)
    /// to set channel availability and TX power per regulatory domain.
    #[error("wifi.country must be exactly two ASCII letters; got {0:?}")]
    InvalidCountry(String),

    /// `mdns.hostname` failed RFC-952 subset: ASCII letters / digits /
    /// hyphens, must start with a letter, must not end with a hyphen,
    /// length 1-63. The hostname is advertised on `.local` so a
    /// malformed value would never resolve.
    #[error("mdns.hostname is not a valid RFC-952 label: {0:?}")]
    InvalidHostname(String),

    /// `time.sntp_servers` was empty. The firmware needs at least one
    /// candidate to attempt SNTP; an empty list would mean the RTC
    /// never advances past whatever the backup battery preserved.
    #[error("time.sntp_servers must contain at least one entry")]
    NoSntpServers,
}
