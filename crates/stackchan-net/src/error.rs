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

    /// `head.pan_trim_deg` or `head.tilt_trim_deg` was non-finite or
    /// outside `[-90.0, 90.0]`. A head trim is a small per-unit
    /// zero-point correction; values past that range can't be physical
    /// and would clamp to a servo rail. The carried `f32` is the
    /// offending value.
    #[error("head trim must be a finite value in [-90.0, 90.0]; got {0}")]
    InvalidHeadTrim(f32),

    /// Hand-rolled bare parser failure (firmware-side path that
    /// avoids `serde + ron`). Carries a short reason string in lieu
    /// of `ron`'s line/col `SpannedError` — the firmware logs this
    /// via `defmt::Debug2Format` and the operator triages from the
    /// boot log.
    #[error("bare RON parse error: {0}")]
    BareParse(String),

    /// `esp_now.pmk_hex` or `esp_now.lmk_hex` was non-empty but not
    /// exactly 32 hex characters (16 bytes). Carries the field name
    /// so the operator can fix the right line.
    #[error("esp_now.{0} must be exactly 32 hex chars (16 bytes)")]
    InvalidEspNowKey(&'static str),

    /// `esp_now.peer_mac` was non-empty but did not parse as either
    /// `xx:xx:xx:xx:xx:xx` or bare 12 hex chars. Carries the offending
    /// value.
    #[error("esp_now.peer_mac is not a valid MAC: {0:?}")]
    InvalidEspNowMac(String),

    /// `esp_now.channel` was outside the valid 2.4 GHz range (1..=14).
    /// Carries the offending value.
    #[error("esp_now.channel must be in 1..=14; got {0}")]
    InvalidEspNowChannel(u8),

    /// `esp_now.tx_rate_hz` exceeded the documented maximum (20 Hz).
    /// Carries the offending value.
    #[error("esp_now.tx_rate_hz must be <= 20; got {0}")]
    InvalidEspNowTxRate(u8),

    /// `behavior.wake_word_arena_kib` was `0`. The wake-word task
    /// allocates the tensor arena once at boot; an empty arena
    /// makes `Interpreter::new` return `None` and leaves the task
    /// permanently parked with only a `defmt::error!` to show for
    /// it. Reject at validation time so the operator gets feedback
    /// from `PUT /settings` instead of needing to reboot to see
    /// the failure.
    #[error("behavior.wake_word_arena_kib must be >= 1; got 0")]
    InvalidWakeWordArenaKib,

    /// `behavior.agent_sidecar_token` exceeded the per-request HTTP
    /// header budget. The firmware's request-header buffer is sized
    /// to absorb a long bearer secret plus the fixed `X-Session-Id`
    /// line; tokens above this cap would silently fail every POST
    /// with `HeaderTooLong`. Reject at validation time so the
    /// operator sees the failure on `PUT /settings` instead of
    /// after the next push-to-talk.
    #[error("behavior.agent_sidecar_token must be <= 256 bytes; got {0}")]
    AgentSidecarTokenTooLong(usize),

    /// `behavior.agent_sidecar_token` contained an ASCII control
    /// character (CR, LF, tab, NUL, …). Embedding `\r\n` in the
    /// token would split the HTTP request header section and inject
    /// an arbitrary header line into the upstream sidecar POST —
    /// reject at validation time so the corrupt value never reaches
    /// the wire.
    #[error("behavior.agent_sidecar_token contains an ASCII control character")]
    AgentSidecarTokenInvalidChars,

    /// `behavior.persona_name` exceeded the documented 64-byte slug
    /// cap. The firmware sends this as `X-Persona-Name` on every
    /// `POST /v1/listen`, so an oversize value would either trip the
    /// per-request header buffer or surface as a 4xx from the
    /// sidecar's persona-load step.
    #[error("behavior.persona_name must be <= 64 bytes; got {0}")]
    PersonaNameTooLong(usize),

    /// `behavior.persona_name` contained an ASCII control char or a
    /// path-traversal token (`/`, `\`, `..`). The sidecar uses the
    /// slug as a filename component under its `personas/` directory,
    /// so any of those would either inject extra HTTP headers via
    /// embedded `\r\n` or pivot the lookup outside the persona dir.
    /// The sidecar revalidates per-request; this gate stops the bad
    /// value at the firmware boundary so operators see the failure
    /// on `PUT /settings` instead of after the next push-to-talk.
    #[error("behavior.persona_name contains an invalid character")]
    PersonaNameInvalidChars,

    /// A disk-loaded `Config` had the redaction sentinel (`"***"`)
    /// in a secret field. The sentinel is meaningful only on the
    /// HTTP `PUT /settings` merge path — on disk there's nothing to
    /// merge against, so a literal `"***"` is operator error
    /// (typically copy-paste from a redacted `GET /settings` body
    /// dumped to SD without filling in the real value). Reject at
    /// load so the device fails fast with a clear message instead
    /// of trying to associate to Wi-Fi with PSK = `"***"`. Carries
    /// the field name so the operator can fix the right line.
    #[error("{0} contains the redaction sentinel \"***\" on disk — supply the real value")]
    RedactionSentinelOnDisk(&'static str),

    /// `appearance.palette` was non-empty but didn't parse via
    /// `stackchan_core::Palette::from_wire_str`. The boot config pins a
    /// default look by wire name (`"default"`, `"dark"`, `"cute"`,
    /// `"dog"`); an unknown value would silently fall back to the
    /// variant default at boot, so reject it at load with the offending
    /// string.
    #[error("appearance.palette is not a known palette: {0:?}")]
    InvalidPalette(String),

    /// `appearance.face_geometry` was non-empty but didn't parse via
    /// `stackchan_core::FaceGeometry::from_wire_str`. Same rationale as
    /// [`Self::InvalidPalette`]: an unknown preset name would silently
    /// fall back rather than surface the operator's typo.
    #[error("appearance.face_geometry is not a known preset: {0:?}")]
    InvalidFaceGeometry(String),
}
