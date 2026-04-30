//! Error types for RON config parsing and validation.

use alloc::string::String;

/// Parse / validate failure for [`crate::Config`] round-trips.
///
/// Variants carry the offending value where it aids debugging — the
/// firmware logs these via `defmt::Debug2Format`, and the catalog at
/// `docs/errors.md` mirrors the same per-variant guidance.
///
/// The `Parse` and `Serialize` variants are gated behind the `parse`
/// feature because they wrap `ron`-side error types — `ron` is only
/// available on host builds. Firmware-side parsers map their own
/// failures to whichever validator variant fits, or surface the
/// underlying error through the firmware's `StorageError`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// RON deserialize failure — syntax error, missing field, or
    /// type mismatch. The wrapped [`ron::error::SpannedError`] carries
    /// `(line, col)` so callers can surface a precise diagnostic.
    #[cfg(feature = "parse")]
    #[error("RON parse error: {0}")]
    Parse(#[from] ron::error::SpannedError),

    /// RON serialize failure on round-trip. Should not happen with a
    /// well-formed [`crate::Config`]; treat as a bug if observed.
    ///
    /// No `#[from]` on this variant: `ron::Error` is also the inner
    /// error code embedded in [`ron::error::SpannedError`] (the parse
    /// path), so an automatic `From<ron::Error>` would silently tag
    /// any deserialize-side error as a serialize one. Callers map
    /// explicitly via `Result::map_err`.
    #[cfg(feature = "parse")]
    #[error("RON serialize error: {0}")]
    Serialize(ron::Error),

    /// `wifi.ssid` was empty or whitespace-only after trim. The
    /// firmware treats an empty SSID as "no Wi-Fi configured" via
    /// `WifiConfig::default`, but an explicitly-blank value in the
    /// file is almost always a mistake.
    #[error("wifi.ssid is empty or whitespace-only")]
    EmptySsid,

    /// `wifi.country` was not exactly two **uppercase** ASCII letters.
    /// ESP-WIFI expects an ISO-3166 alpha-2 country code in canonical
    /// case (e.g. `"US"`, `"JP"`) to set channel availability and TX
    /// power per regulatory domain; lowercase silently mis-applies the
    /// regulatory mask at the driver layer.
    #[error("wifi.country must be exactly two uppercase ASCII letters (e.g. \"US\"); got {0:?}")]
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

    /// A `time.sntp_servers` entry was empty or whitespace-only.
    /// Caught at parse time so the firmware's "try in order" loop
    /// doesn't burn its full per-server timeout on an unresolvable
    /// hostname before falling back. The `usize` carries the offending
    /// index in the original list.
    #[error("time.sntp_servers[{0}] is empty or whitespace-only")]
    EmptySntpServer(usize),

    /// `audio.volume_pct` was outside `0..=100`. The wire format is
    /// a percentile; the firmware maps it linearly across the AW88298
    /// dB range. The `u8` carries the offending value.
    #[error("audio.volume_pct must be 0..=100; got {0}")]
    InvalidVolumePct(u8),

    /// `tracker.fov_h_deg` or `tracker.fov_v_deg` was non-finite,
    /// non-positive, or larger than 180°. Lens FOVs outside that
    /// range can't be physical; the carried `f32` is the offending
    /// value.
    #[error("tracker FOV must be a finite value in (0.0, 180.0]; got {0}")]
    InvalidFovDeg(f32),

    /// `tracker.target_smoothing_alpha` was outside `[0.05, 1.0]`.
    /// Below 0.05 effectively freezes the published target;
    /// above 1.0 has no defined meaning for an EMA. The carried
    /// `f32` is the offending value.
    #[error("tracker.target_smoothing_alpha must be in [0.05, 1.0]; got {0}")]
    InvalidSmoothingAlpha(f32),

    /// Hand-rolled bare parser failure (firmware-side path that
    /// avoids `serde + ron`). Carries a short reason string in lieu
    /// of `ron`'s line/col `SpannedError` — the firmware logs this
    /// via `defmt::Debug2Format` and the operator triages from the
    /// boot log.
    #[error("bare RON parse error: {0}")]
    BareParse(String),
}
