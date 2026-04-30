//! Hand-rolled JSON parser + renderer for [`crate::Config`].
//!
//! Symmetric with [`crate::bare::parse_ron_bare`] /
//! [`crate::bare::render_ron_bare`] but for the wire format the
//! firmware HTTP control plane uses on `GET /settings` and
//! `PUT /settings`. SD persistence stays RON; HTTP stays JSON.
//!
//! Keeping the parser hand-rolled (no `serde`, no `serde_json`)
//! mirrors the RON approach: the firmware crate disables
//! [`stackchan-net`'s default `parse` feature](crate) to avoid
//! pulling `serde/std` onto `xtensa-esp32s3-none-elf`. See
//! [`crate::bare`] for the full rationale.
//!
//! Schema is fixed to v1: top-level object, three nested objects,
//! string fields, one `Vec<String>`. Anything outside that surface
//! returns [`ConfigError::BareParse`].

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write as _;

use crate::config::{
    AudioConfig, AuthConfig, Config, MdnsConfig, TimeConfig, TrackerSettings, WifiConfig, validate,
};
use crate::error::ConfigError;

/// Sentinel emitted by [`render_settings_json`] when `redact_secrets = true`.
///
/// Doubles as a "preserve current value" marker on the input side:
/// a `PUT /settings` body that echoes `"***"` back for the PSK
/// keeps whatever's currently persisted instead of overwriting with
/// the placeholder. Callers merge via [`merge_settings_with_current`]
/// before writing back to disk.
pub const PSK_REDACTED: &str = "***";

/// Sentinel emitted in place of a non-empty `auth.token` on render.
///
/// Same `"preserve current value"` semantics on the input side as
/// [`PSK_REDACTED`] — see [`merge_settings_with_current`].
pub const TOKEN_REDACTED: &str = "***";

/// Parse a schema-v1 JSON object into a [`Config`].
///
/// Whitespace tolerant. Unknown keys are rejected (typo guard).
/// String escapes: `\\`, `\"`, `\n`, `\t`, `\r`. Numbers, booleans,
/// nulls, and `\u{...}` escapes are not supported — the schema
/// doesn't need them.
///
/// # Errors
///
/// Returns [`ConfigError::BareParse`] on any structural mismatch
/// (missing field, unknown key, bad string escape, runaway literal,
/// or `wifi.psk` set to the [`PSK_REDACTED`] sentinel), then runs
/// the shared [`validate`] gate so out-of-range values surface the
/// same `Invalid*` variants as [`crate::parse_ron`].
pub fn parse_settings_json(input: &str) -> Result<Config, ConfigError> {
    let mut p = Parser::new(input);
    let config = p.parse_config()?;
    p.skip_ws();
    if !p.input.is_empty() {
        return Err(bare_err("trailing data after object", ""));
    }
    validate(&config)?;
    Ok(config)
}

/// Substitute the redacted PSK / token sentinels in a parsed body
/// with the values from `current`.
///
/// `PUT /settings` bodies may echo `"***"` back for [`PSK_REDACTED`]
/// or [`TOKEN_REDACTED`]; this helper means "keep current value"
/// rather than "overwrite with literal `***`."
///
/// Without this step, an operator who edits the hostname in the
/// dashboard form (or via curl with the JSON returned by
/// `GET /settings`) would unintentionally clobber the real PSK or
/// token with the placeholder string. Anything other than the
/// sentinel is taken at face value: an empty PSK clears the key
/// (open-AP), an empty token disables auth, an explicit value
/// overwrites.
///
/// Fields the wire format doesn't redact (`ssid`, `country`,
/// `mdns.hostname`, `time.*`, `audio.*`, `tracker.*`) pass through
/// from `new` unchanged.
#[must_use]
pub fn merge_settings_with_current(new: Config, current: &Config) -> Config {
    Config {
        wifi: WifiConfig {
            ssid: new.wifi.ssid,
            psk: if new.wifi.psk == PSK_REDACTED {
                current.wifi.psk.clone()
            } else {
                new.wifi.psk
            },
            country: new.wifi.country,
        },
        mdns: new.mdns,
        time: new.time,
        auth: AuthConfig {
            token: if new.auth.token == TOKEN_REDACTED {
                current.auth.token.clone()
            } else {
                new.auth.token
            },
        },
        audio: new.audio,
        tracker: new.tracker,
    }
}

/// Render a [`Config`] to a compact JSON string.
///
/// `redact_secrets = true` replaces `wifi.psk` and any non-empty
/// `auth.token` with `"***"` so the output is safe to expose to
/// unauthed callers (HTTP `GET /settings` passes `true`). An empty
/// token renders as `""` regardless — there's nothing to redact and
/// the empty string is its own meaningful value (= auth disabled).
/// `redact_secrets = false` round-trips losslessly with
/// [`parse_settings_json`].
///
/// # Errors
///
/// Currently infallible — kept as `Result` for symmetry with
/// [`crate::render_ron`].
pub fn render_settings_json(config: &Config, redact_secrets: bool) -> Result<String, ConfigError> {
    let mut out = String::new();
    out.push('{');
    out.push_str("\"wifi\":{");
    push_string_field(&mut out, "ssid", &config.wifi.ssid);
    out.push(',');
    push_string_field(
        &mut out,
        "psk",
        if redact_secrets {
            PSK_REDACTED
        } else {
            &config.wifi.psk
        },
    );
    out.push(',');
    push_string_field(&mut out, "country", &config.wifi.country);
    out.push_str("},\"mdns\":{");
    push_string_field(&mut out, "hostname", &config.mdns.hostname);
    out.push_str("},\"time\":{");
    push_string_field(&mut out, "tz", &config.time.tz);
    out.push_str(",\"sntp_servers\":[");
    for (idx, s) in config.time.sntp_servers.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        push_string_literal(&mut out, s);
    }
    out.push_str("]},\"auth\":{");
    let token_view: &str = if redact_secrets && !config.auth.token.is_empty() {
        TOKEN_REDACTED
    } else {
        &config.auth.token
    };
    push_string_field(&mut out, "token", token_view);
    out.push_str("},\"audio\":{");
    let _ = write!(
        out,
        "\"volume_pct\":{},\"muted\":{}",
        config.audio.volume_pct, config.audio.muted
    );
    out.push_str("},\"tracker\":{");
    let _ = write!(
        out,
        "\"fov_h_deg\":{fov_h:?},\"fov_v_deg\":{fov_v:?},\
         \"target_smoothing_alpha\":{alpha:?},\
         \"flip_x\":{flip_x},\"flip_y\":{flip_y}",
        fov_h = config.tracker.fov_h_deg,
        fov_v = config.tracker.fov_v_deg,
        alpha = config.tracker.target_smoothing_alpha,
        flip_x = config.tracker.flip_x,
        flip_y = config.tracker.flip_y,
    );
    out.push_str("}}");
    Ok(out)
}

/// Helper: emit `"name":"value"` (no leading comma — the caller
/// places commas between fields).
fn push_string_field(out: &mut String, name: &str, value: &str) {
    out.push('"');
    out.push_str(name);
    out.push_str("\":");
    push_string_literal(out, value);
}

/// Helper: emit a quoted JSON string with `\\`, `\"`, `\n`, `\t`,
/// `\r` escapes. Other control characters pass through unescaped —
/// this is firmware-emitted output bound for a trusted client, not
/// arbitrary user data.
fn push_string_literal(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out.push('"');
}

/// Recursive-descent parser over a `&str` cursor. Mirrors the
/// shape of [`crate::bare::Parser`] but for JSON delimiters.
struct Parser<'a> {
    /// Remaining input. `advance` slides the start pointer; nothing
    /// is allocated for tokens.
    input: &'a str,
}

impl<'a> Parser<'a> {
    /// Construct a fresh parser over `input`.
    const fn new(input: &'a str) -> Self {
        Self { input }
    }

    /// Top-level grammar: parse the schema-v1 outer object.
    fn parse_config(&mut self) -> Result<Config, ConfigError> {
        self.skip_ws();
        self.expect_char('{')?;
        let mut wifi: Option<WifiConfig> = None;
        let mut mdns: Option<MdnsConfig> = None;
        let mut time: Option<TimeConfig> = None;
        let mut auth: Option<AuthConfig> = None;
        let mut audio: Option<AudioConfig> = None;
        let mut tracker: Option<TrackerSettings> = None;
        loop {
            self.skip_ws();
            if self.try_consume_char('}') {
                break;
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect_char(':')?;
            self.skip_ws();
            match key.as_str() {
                "wifi" => {
                    if wifi.is_some() {
                        return Err(bare_err("duplicate top-level field", "wifi"));
                    }
                    wifi = Some(self.parse_wifi()?);
                }
                "mdns" => {
                    if mdns.is_some() {
                        return Err(bare_err("duplicate top-level field", "mdns"));
                    }
                    mdns = Some(self.parse_mdns()?);
                }
                "time" => {
                    if time.is_some() {
                        return Err(bare_err("duplicate top-level field", "time"));
                    }
                    time = Some(self.parse_time()?);
                }
                "auth" => {
                    if auth.is_some() {
                        return Err(bare_err("duplicate top-level field", "auth"));
                    }
                    auth = Some(self.parse_auth()?);
                }
                "audio" => {
                    if audio.is_some() {
                        return Err(bare_err("duplicate top-level field", "audio"));
                    }
                    audio = Some(self.parse_audio()?);
                }
                "tracker" => {
                    if tracker.is_some() {
                        return Err(bare_err("duplicate top-level field", "tracker"));
                    }
                    tracker = Some(self.parse_tracker()?);
                }
                other => return Err(bare_err("unknown top-level field", other)),
            }
            self.skip_ws();
            if !self.try_consume_char(',') && !self.peek_eq('}') {
                return Err(bare_err("expected ',' or '}'", ""));
            }
        }
        Ok(Config {
            wifi: wifi.ok_or_else(|| bare_err("missing field 'wifi'", ""))?,
            mdns: mdns.ok_or_else(|| bare_err("missing field 'mdns'", ""))?,
            time: time.ok_or_else(|| bare_err("missing field 'time'", ""))?,
            // `auth`, `audio`, `tracker` are optional for migration:
            // bodies emitted before each block landed don't have them,
            // and the defaults match the firmware's prior hard-coded
            // behaviour.
            auth: auth.unwrap_or_default(),
            audio: audio.unwrap_or_default(),
            tracker: tracker.unwrap_or_default(),
        })
    }

    /// Parse the `"wifi": { "ssid": ..., "psk": ..., "country": ... }` block.
    fn parse_wifi(&mut self) -> Result<WifiConfig, ConfigError> {
        self.expect_char('{')?;
        let mut ssid: Option<String> = None;
        let mut psk: Option<String> = None;
        let mut country: Option<String> = None;
        loop {
            self.skip_ws();
            if self.try_consume_char('}') {
                break;
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect_char(':')?;
            self.skip_ws();
            let value = self.parse_string()?;
            match key.as_str() {
                "ssid" => {
                    if ssid.is_some() {
                        return Err(bare_err("duplicate wifi field", "ssid"));
                    }
                    ssid = Some(value);
                }
                "psk" => {
                    if psk.is_some() {
                        return Err(bare_err("duplicate wifi field", "psk"));
                    }
                    psk = Some(value);
                }
                "country" => {
                    if country.is_some() {
                        return Err(bare_err("duplicate wifi field", "country"));
                    }
                    country = Some(value);
                }
                other => return Err(bare_err("unknown wifi field", other)),
            }
            self.skip_ws();
            if !self.try_consume_char(',') && !self.peek_eq('}') {
                return Err(bare_err("expected ',' or '}' in wifi", ""));
            }
        }
        let psk = psk.ok_or_else(|| bare_err("missing wifi.psk", ""))?;
        // Note: a literal `"***"` here is *not* an error — it's the
        // "preserve current PSK" sentinel. The HTTP handler merges
        // it against the current snapshot via
        // [`merge_settings_with_current`] before writing back.
        Ok(WifiConfig {
            ssid: ssid.ok_or_else(|| bare_err("missing wifi.ssid", ""))?,
            psk,
            country: country.ok_or_else(|| bare_err("missing wifi.country", ""))?,
        })
    }

    /// Parse the `"mdns": { "hostname": ... }` block.
    fn parse_mdns(&mut self) -> Result<MdnsConfig, ConfigError> {
        self.expect_char('{')?;
        let mut hostname: Option<String> = None;
        loop {
            self.skip_ws();
            if self.try_consume_char('}') {
                break;
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect_char(':')?;
            self.skip_ws();
            let value = self.parse_string()?;
            match key.as_str() {
                "hostname" => {
                    if hostname.is_some() {
                        return Err(bare_err("duplicate mdns field", "hostname"));
                    }
                    hostname = Some(value);
                }
                other => return Err(bare_err("unknown mdns field", other)),
            }
            self.skip_ws();
            if !self.try_consume_char(',') && !self.peek_eq('}') {
                return Err(bare_err("expected ',' or '}' in mdns", ""));
            }
        }
        Ok(MdnsConfig {
            hostname: hostname.ok_or_else(|| bare_err("missing mdns.hostname", ""))?,
        })
    }

    /// Parse the `"time": { "tz": ..., "sntp_servers": [...] }` block.
    fn parse_time(&mut self) -> Result<TimeConfig, ConfigError> {
        self.expect_char('{')?;
        let mut tz: Option<String> = None;
        let mut sntp_servers: Option<Vec<String>> = None;
        loop {
            self.skip_ws();
            if self.try_consume_char('}') {
                break;
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect_char(':')?;
            self.skip_ws();
            match key.as_str() {
                "tz" => {
                    if tz.is_some() {
                        return Err(bare_err("duplicate time field", "tz"));
                    }
                    tz = Some(self.parse_string()?);
                }
                "sntp_servers" => {
                    if sntp_servers.is_some() {
                        return Err(bare_err("duplicate time field", "sntp_servers"));
                    }
                    sntp_servers = Some(self.parse_string_list()?);
                }
                other => return Err(bare_err("unknown time field", other)),
            }
            self.skip_ws();
            if !self.try_consume_char(',') && !self.peek_eq('}') {
                return Err(bare_err("expected ',' or '}' in time", ""));
            }
        }
        Ok(TimeConfig {
            tz: tz.ok_or_else(|| bare_err("missing time.tz", ""))?,
            sntp_servers: sntp_servers.ok_or_else(|| bare_err("missing time.sntp_servers", ""))?,
        })
    }

    /// Parse the `"auth": { "token": ... }` block.
    fn parse_auth(&mut self) -> Result<AuthConfig, ConfigError> {
        self.expect_char('{')?;
        let mut token: Option<String> = None;
        loop {
            self.skip_ws();
            if self.try_consume_char('}') {
                break;
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect_char(':')?;
            self.skip_ws();
            let value = self.parse_string()?;
            match key.as_str() {
                "token" => {
                    if token.is_some() {
                        return Err(bare_err("duplicate auth field", "token"));
                    }
                    token = Some(value);
                }
                other => return Err(bare_err("unknown auth field", other)),
            }
            self.skip_ws();
            if !self.try_consume_char(',') && !self.peek_eq('}') {
                return Err(bare_err("expected ',' or '}' in auth", ""));
            }
        }
        let token = token.unwrap_or_default();
        // `"***"` is the "preserve current token" sentinel; the
        // HTTP handler merges via [`merge_settings_with_current`]
        // before persisting.
        Ok(AuthConfig { token })
    }

    /// Parse the `"audio": { "volume_pct": <int>, "muted": <bool> }` block.
    /// Both fields are optional on the wire for forward-compat — a
    /// body that omits one keeps the default. The validator catches
    /// out-of-range `volume_pct` after parse with
    /// `ConfigError::InvalidVolumePct`.
    fn parse_audio(&mut self) -> Result<AudioConfig, ConfigError> {
        self.expect_char('{')?;
        let mut volume_pct: Option<u8> = None;
        let mut muted: Option<bool> = None;
        loop {
            self.skip_ws();
            if self.try_consume_char('}') {
                break;
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect_char(':')?;
            self.skip_ws();
            match key.as_str() {
                "volume_pct" => {
                    if volume_pct.is_some() {
                        return Err(bare_err("duplicate audio field", "volume_pct"));
                    }
                    volume_pct = Some(self.parse_u8()?);
                }
                "muted" => {
                    if muted.is_some() {
                        return Err(bare_err("duplicate audio field", "muted"));
                    }
                    muted = Some(self.parse_bool()?);
                }
                other => return Err(bare_err("unknown audio field", other)),
            }
            self.skip_ws();
            if !self.try_consume_char(',') && !self.peek_eq('}') {
                return Err(bare_err("expected ',' or '}' in audio", ""));
            }
        }
        let defaults = AudioConfig::default();
        Ok(AudioConfig {
            volume_pct: volume_pct.unwrap_or(defaults.volume_pct),
            muted: muted.unwrap_or(defaults.muted),
        })
    }

    /// Parse the `"tracker": { ... }` block. All five fields are
    /// optional on the wire; missing fields fall back to
    /// [`TrackerSettings::DEFAULT`]. Range validation lives in the
    /// `validate` pass after parse so out-of-range values surface
    /// with the exact offending number via `ConfigError::Invalid…`.
    fn parse_tracker(&mut self) -> Result<TrackerSettings, ConfigError> {
        self.expect_char('{')?;
        // See `bare::parse_tracker` for the local-naming rationale.
        let mut pan_fov: Option<f32> = None;
        let mut tilt_fov: Option<f32> = None;
        let mut alpha: Option<f32> = None;
        let mut flip_x: Option<bool> = None;
        let mut flip_y: Option<bool> = None;
        loop {
            self.skip_ws();
            if self.try_consume_char('}') {
                break;
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect_char(':')?;
            self.skip_ws();
            match key.as_str() {
                "fov_h_deg" => {
                    if pan_fov.is_some() {
                        return Err(bare_err("duplicate tracker field", "fov_h_deg"));
                    }
                    pan_fov = Some(self.parse_f32()?);
                }
                "fov_v_deg" => {
                    if tilt_fov.is_some() {
                        return Err(bare_err("duplicate tracker field", "fov_v_deg"));
                    }
                    tilt_fov = Some(self.parse_f32()?);
                }
                "target_smoothing_alpha" => {
                    if alpha.is_some() {
                        return Err(bare_err(
                            "duplicate tracker field",
                            "target_smoothing_alpha",
                        ));
                    }
                    alpha = Some(self.parse_f32()?);
                }
                "flip_x" => {
                    if flip_x.is_some() {
                        return Err(bare_err("duplicate tracker field", "flip_x"));
                    }
                    flip_x = Some(self.parse_bool()?);
                }
                "flip_y" => {
                    if flip_y.is_some() {
                        return Err(bare_err("duplicate tracker field", "flip_y"));
                    }
                    flip_y = Some(self.parse_bool()?);
                }
                other => return Err(bare_err("unknown tracker field", other)),
            }
            self.skip_ws();
            if !self.try_consume_char(',') && !self.peek_eq('}') {
                return Err(bare_err("expected ',' or '}' in tracker", ""));
            }
        }
        let defaults = TrackerSettings::DEFAULT;
        Ok(TrackerSettings {
            fov_h_deg: pan_fov.unwrap_or(defaults.fov_h_deg),
            fov_v_deg: tilt_fov.unwrap_or(defaults.fov_v_deg),
            target_smoothing_alpha: alpha.unwrap_or(defaults.target_smoothing_alpha),
            flip_x: flip_x.unwrap_or(defaults.flip_x),
            flip_y: flip_y.unwrap_or(defaults.flip_y),
        })
    }

    /// Parse a JSON number into `f32`. Delegates to
    /// [`crate::bare::scan_f32`] so the RON and JSON parsers consume
    /// the identical grammar (and so a leading `+` is rejected
    /// uniformly — RFC 8259 §6 disallows it, and the RON path also
    /// has no use for it).
    fn parse_f32(&mut self) -> Result<f32, ConfigError> {
        crate::bare::scan_f32(&mut self.input)
    }

    /// Parse a contiguous run of decimal digits as `u8`. Out-of-range
    /// (>100) values pass parse but trip
    /// `ConfigError::InvalidVolumePct` in `validate` so the operator
    /// gets the exact offending value back.
    fn parse_u8(&mut self) -> Result<u8, ConfigError> {
        let bytes = self.input.as_bytes();
        let mut end = 0;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == 0 {
            return Err(bare_err("expected unsigned integer", ""));
        }
        let (digits, rest) = self.input.split_at(end);
        let parsed: u16 = digits
            .parse()
            .map_err(|_| bare_err("not a u8 literal", digits))?;
        if parsed > u16::from(u8::MAX) {
            return Err(bare_err("u8 literal out of range", digits));
        }
        self.input = rest;
        #[allow(clippy::cast_possible_truncation)]
        Ok(parsed as u8)
    }

    /// Parse a bare JSON `true` / `false` literal.
    fn parse_bool(&mut self) -> Result<bool, ConfigError> {
        if self.input.starts_with("true") {
            self.advance("true".len());
            Ok(true)
        } else if self.input.starts_with("false") {
            self.advance("false".len());
            Ok(false)
        } else {
            Err(bare_err("expected boolean literal", ""))
        }
    }

    /// Parse `[ "...", "...", ... ]` into a `Vec<String>`.
    fn parse_string_list(&mut self) -> Result<Vec<String>, ConfigError> {
        self.expect_char('[')?;
        let mut out: Vec<String> = Vec::new();
        loop {
            self.skip_ws();
            if self.try_consume_char(']') {
                break;
            }
            let s = self.parse_string()?;
            out.push(s);
            self.skip_ws();
            if !self.try_consume_char(',') && !self.peek_eq(']') {
                return Err(bare_err("expected ',' or ']' in list", ""));
            }
        }
        Ok(out)
    }

    /// Parse a `"..."` string literal with `\\`, `\"`, `\n`, `\t`,
    /// `\r` escapes.
    fn parse_string(&mut self) -> Result<String, ConfigError> {
        self.expect_char('"')?;
        let mut out = String::new();
        loop {
            let Some(ch) = self.peek_char() else {
                return Err(bare_err("unterminated string literal", ""));
            };
            if ch == '"' {
                self.advance(1);
                return Ok(out);
            }
            if ch == '\\' {
                self.advance(1);
                let Some(esc) = self.peek_char() else {
                    return Err(bare_err("dangling backslash", ""));
                };
                match esc {
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    other => return Err(bare_err("unsupported escape", &other.to_string())),
                }
                self.advance(esc.len_utf8());
            } else {
                out.push(ch);
                self.advance(ch.len_utf8());
            }
        }
    }

    /// Peek the first byte (interpreted as a `char`) without
    /// advancing.
    fn peek_char(&self) -> Option<char> {
        self.input.chars().next()
    }

    /// Drop `n` bytes from the front of [`Self::input`].
    const fn advance(&mut self, n: usize) {
        self.input = self.input.split_at(n).1;
    }

    /// Skip ASCII whitespace.
    fn skip_ws(&mut self) {
        let bytes = self.input.as_bytes();
        let mut i = 0;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        self.advance(i);
    }

    /// Require the next char to equal `expected`; advance past it
    /// on success.
    fn expect_char(&mut self, expected: char) -> Result<(), ConfigError> {
        if self.try_consume_char(expected) {
            Ok(())
        } else {
            Err(bare_err("expected char", &expected.to_string()))
        }
    }

    /// Consume the next char if it equals `c`. Returns whether it
    /// did.
    fn try_consume_char(&mut self, c: char) -> bool {
        if self.peek_eq(c) {
            self.advance(c.len_utf8());
            true
        } else {
            false
        }
    }

    /// Whether the next char equals `c`.
    fn peek_eq(&self, c: char) -> bool {
        self.peek_char() == Some(c)
    }
}

/// Build a `BareParse` error. Format mirrors [`crate::bare::bare_err`]
/// so error strings stay homogeneous across the two parsers.
fn bare_err(reason: &str, detail: &str) -> ConfigError {
    let mut s = String::with_capacity(reason.len() + detail.len() + 4);
    s.push_str(reason);
    if !detail.is_empty() {
        s.push_str(": ");
        s.push_str(detail);
    }
    ConfigError::BareParse(s)
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test-only: match-with-panic for variant extraction"
)]
mod tests {
    use super::*;

    fn full_config() -> Config {
        Config {
            wifi: WifiConfig {
                ssid: "myssid".to_string(),
                psk: "secret".to_string(),
                country: "US".to_string(),
            },
            mdns: MdnsConfig {
                hostname: "stackchan".to_string(),
            },
            time: TimeConfig {
                tz: "UTC".to_string(),
                sntp_servers: vec!["pool.ntp.org".to_string(), "time.google.com".to_string()],
            },
            auth: AuthConfig {
                token: "shared-secret".to_string(),
            },
            audio: AudioConfig {
                volume_pct: 33,
                muted: true,
            },
            tracker: TrackerSettings {
                fov_h_deg: 60.0,
                fov_v_deg: 45.0,
                target_smoothing_alpha: 0.4,
                flip_x: true,
                flip_y: false,
            },
        }
    }

    #[test]
    fn round_trips_through_render_then_parse() {
        let original = full_config();
        let rendered = render_settings_json(&original, false).unwrap();
        let parsed = parse_settings_json(&rendered).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn render_redacts_psk_when_requested() {
        let original = full_config();
        let rendered = render_settings_json(&original, true).unwrap();
        assert!(
            rendered.contains("\"psk\":\"***\""),
            "expected redacted psk in: {rendered}"
        );
        assert!(
            !rendered.contains("secret"),
            "psk leaked through redaction: {rendered}"
        );
    }

    #[test]
    fn parses_minimal_pretty_input() {
        let input = r#"{
            "wifi":   { "ssid": "home", "psk": "p", "country": "US" },
            "mdns":   { "hostname": "stackchan" },
            "time":   { "tz": "UTC", "sntp_servers": ["pool.ntp.org"] }
        }"#;
        let parsed = parse_settings_json(input).unwrap();
        assert_eq!(parsed.wifi.ssid, "home");
        assert_eq!(parsed.time.sntp_servers, vec!["pool.ntp.org".to_string()]);
    }

    #[test]
    fn rejects_unknown_top_level_key() {
        let mut original = render_settings_json(&full_config(), false).unwrap();
        // Inject an unknown key by replacing the closing brace with one.
        original.pop(); // remove trailing }
        original.push_str(",\"extra\":\"junk\"}");
        let err = parse_settings_json(&original).unwrap_err();
        assert!(matches!(err, ConfigError::BareParse(_)), "got {err:?}");
    }

    #[test]
    fn rejects_missing_required_block() {
        let input = r#"{"wifi":{"ssid":"a","psk":"b","country":"US"},"mdns":{"hostname":"x"}}"#;
        let err = parse_settings_json(input).unwrap_err();
        assert!(matches!(err, ConfigError::BareParse(_)), "got {err:?}");
    }

    #[test]
    fn rejects_missing_wifi_field() {
        let input = r#"{"wifi":{"ssid":"a","country":"US"},"mdns":{"hostname":"x"},"time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]}}"#;
        let err = parse_settings_json(input).unwrap_err();
        assert!(matches!(err, ConfigError::BareParse(_)), "got {err:?}");
    }

    #[test]
    fn rejects_trailing_garbage_after_object() {
        let mut input = render_settings_json(&full_config(), false).unwrap();
        input.push_str(" extra");
        let err = parse_settings_json(&input).unwrap_err();
        assert!(matches!(err, ConfigError::BareParse(_)), "got {err:?}");
    }

    #[test]
    fn surfaces_validation_errors() {
        // Empty SNTP server list — schema-shape valid, validation rejects.
        let input = r#"{"wifi":{"ssid":"a","psk":"b","country":"US"},"mdns":{"hostname":"x"},"time":{"tz":"UTC","sntp_servers":[]}}"#;
        let err = parse_settings_json(input).unwrap_err();
        assert!(
            matches!(err, ConfigError::NoSntpServers),
            "expected NoSntpServers, got {err:?}"
        );
    }

    #[test]
    fn render_redacts_token_when_requested() {
        let original = full_config();
        let rendered = render_settings_json(&original, true).unwrap();
        assert!(
            rendered.contains("\"token\":\"***\""),
            "expected redacted token in: {rendered}"
        );
        assert!(
            !rendered.contains("shared-secret"),
            "token leaked through redaction: {rendered}"
        );
    }

    #[test]
    fn render_does_not_redact_empty_token() {
        // An empty token is meaningful (= auth disabled); it should
        // render as `""` even when `redact_secrets = true`, because
        // `***` is the rejected sentinel and a no-auth device must
        // be able to round-trip its (empty) token through GET → PUT.
        let mut cfg = full_config();
        cfg.auth.token = String::new();
        let redacted = render_settings_json(&cfg, true).unwrap();
        assert!(
            redacted.contains("\"token\":\"\""),
            "empty token should render as empty string: {redacted}"
        );
        // Round-trip via the lossless render to confirm parser side.
        let lossless = render_settings_json(&cfg, false).unwrap();
        let parsed = parse_settings_json(&lossless).unwrap();
        assert_eq!(parsed.auth.token, "");
    }

    #[test]
    fn parser_accepts_redacted_token_for_preserve_round_trip() {
        // GET /settings emits `"token":"***"` for a configured token;
        // a dashboard form that submits unchanged should send the same
        // sentinel back, and the parser must accept it. Merge happens
        // downstream via [`merge_settings_with_current`].
        let body = r#"{
            "wifi":{"ssid":"a","psk":"realkey","country":"US"},
            "mdns":{"hostname":"x"},
            "time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]},
            "auth":{"token":"***"}
        }"#;
        let parsed = parse_settings_json(body).unwrap();
        assert_eq!(parsed.auth.token, TOKEN_REDACTED);
    }

    #[test]
    fn missing_auth_block_defaults_to_empty_token() {
        // Schema-v1 bodies (rendered before auth was added) lack the
        // `auth` field. The parser must fall back to the default empty
        // token so existing operators don't get locked out on first
        // boot of a firmware build that knows about auth.
        let input = r#"{"wifi":{"ssid":"a","psk":"b","country":"US"},"mdns":{"hostname":"x"},"time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]}}"#;
        let parsed = parse_settings_json(input).unwrap();
        assert_eq!(parsed.auth.token, "");
    }

    #[test]
    fn parser_accepts_redacted_psk_for_preserve_round_trip() {
        // The redacted body emitted by `GET /settings` round-trips
        // through the parser; the literal sentinel value flows
        // through to the downstream merge step rather than tripping
        // a 400.
        let body = render_settings_json(&full_config(), true).unwrap();
        let parsed = parse_settings_json(&body).unwrap();
        assert_eq!(parsed.wifi.psk, PSK_REDACTED);
        assert_eq!(parsed.auth.token, TOKEN_REDACTED);
    }

    #[test]
    fn merge_substitutes_redacted_psk_with_current() {
        let current = full_config(); // psk = "secret", token = "shared-secret"
        let new = Config {
            wifi: WifiConfig {
                ssid: "newssid".to_string(),
                psk: PSK_REDACTED.to_string(),
                country: "JP".to_string(),
            },
            ..current.clone()
        };
        let merged = merge_settings_with_current(new, &current);
        assert_eq!(merged.wifi.psk, "secret", "psk preserved from current");
        assert_eq!(
            merged.wifi.ssid, "newssid",
            "non-secret fields pass through"
        );
        assert_eq!(merged.wifi.country, "JP");
    }

    #[test]
    fn merge_substitutes_redacted_token_with_current() {
        let current = full_config(); // token = "shared-secret"
        let new = Config {
            auth: AuthConfig {
                token: TOKEN_REDACTED.to_string(),
            },
            ..current.clone()
        };
        let merged = merge_settings_with_current(new, &current);
        assert_eq!(merged.auth.token, "shared-secret");
    }

    #[test]
    fn merge_overwrites_explicit_value() {
        let current = full_config();
        let new = Config {
            wifi: WifiConfig {
                ssid: current.wifi.ssid.clone(),
                psk: "new-psk".to_string(),
                country: current.wifi.country.clone(),
            },
            auth: AuthConfig {
                token: "new-token".to_string(),
            },
            ..current.clone()
        };
        let merged = merge_settings_with_current(new, &current);
        assert_eq!(merged.wifi.psk, "new-psk");
        assert_eq!(merged.auth.token, "new-token");
    }

    #[test]
    fn merge_clears_to_empty_when_explicitly_empty() {
        // An empty-string PSK or token isn't the redaction sentinel;
        // it's a meaningful value (open AP / auth disabled) and must
        // pass through.
        let current = full_config();
        let new = Config {
            wifi: WifiConfig {
                psk: String::new(),
                ..current.wifi.clone()
            },
            auth: AuthConfig {
                token: String::new(),
            },
            ..current.clone()
        };
        let merged = merge_settings_with_current(new, &current);
        assert!(merged.wifi.psk.is_empty(), "empty PSK clears to empty");
        assert!(merged.auth.token.is_empty(), "empty token disables auth");
    }

    #[test]
    fn render_uses_psk_redacted_constant() {
        // Pin: the sentinel string emitted by render is the same
        // string parse rejects.
        let body = render_settings_json(&full_config(), true).unwrap();
        assert!(body.contains(&format!("\"psk\":\"{PSK_REDACTED}\"")));
    }

    #[test]
    fn rejects_duplicate_top_level_field() {
        let input = r#"{"wifi":{"ssid":"a","psk":"b","country":"US"},"wifi":{"ssid":"x","psk":"y","country":"JP"},"mdns":{"hostname":"x"},"time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]}}"#;
        let err = parse_settings_json(input).unwrap_err();
        match err {
            ConfigError::BareParse(msg) => assert!(
                msg.contains("duplicate top-level field"),
                "expected duplicate-field message, got `{msg}`"
            ),
            other => panic!("expected BareParse, got {other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_nested_field() {
        let input = r#"{"wifi":{"ssid":"a","ssid":"b","psk":"p","country":"US"},"mdns":{"hostname":"x"},"time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]}}"#;
        let err = parse_settings_json(input).unwrap_err();
        match err {
            ConfigError::BareParse(msg) => assert!(
                msg.contains("duplicate wifi field"),
                "expected duplicate-wifi-field message, got `{msg}`"
            ),
            other => panic!("expected BareParse, got {other:?}"),
        }
    }

    #[test]
    fn handles_string_escapes() {
        let input = r#"{"wifi":{"ssid":"a\tb","psk":"\\\"x","country":"US"},"mdns":{"hostname":"x"},"time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]}}"#;
        let parsed = parse_settings_json(input).unwrap();
        assert_eq!(parsed.wifi.ssid, "a\tb");
        assert_eq!(parsed.wifi.psk, "\\\"x");
    }

    #[test]
    fn maximal_payload_fits_under_http_body_cap() {
        // Pin the upper bound on `PUT /settings` payloads. Numbers
        // mirror real-world maxima:
        //   - SSID: 32 chars (IEEE 802.11 limit)
        //   - PSK: 63 chars (WPA2/3 PSK upper bound)
        //   - hostname: 63 chars (RFC-952 / firmware validator)
        //   - tz: long IANA label
        //   - sntp_servers: 4 servers, 16 chars each (typical FQDN)
        // Firmware caps the body at 1024 bytes; this test fails if a
        // schema addition pushes legitimate payloads past that cap so
        // we notice before users do.
        let config = Config {
            wifi: WifiConfig {
                ssid: "x".repeat(32),
                psk: "x".repeat(63),
                country: "US".to_string(),
            },
            mdns: MdnsConfig {
                hostname: "x".repeat(63),
            },
            time: TimeConfig {
                tz: "America/Argentina/Buenos_Aires".to_string(),
                sntp_servers: vec![
                    "time1.google.com".to_string(),
                    "time2.google.com".to_string(),
                    "time3.google.com".to_string(),
                    "time4.google.com".to_string(),
                ],
            },
            auth: AuthConfig {
                token: "x".repeat(64),
            },
            audio: AudioConfig {
                volume_pct: 100,
                muted: false,
            },
            tracker: TrackerSettings::DEFAULT,
        };
        let rendered = render_settings_json(&config, false).unwrap();
        assert!(
            rendered.len() < 1024,
            "maximal payload exceeded firmware MAX_BODY_BYTES: {} bytes",
            rendered.len()
        );
        let parsed = parse_settings_json(&rendered).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn missing_audio_block_defaults_to_50_unmuted() {
        let input = r#"{"wifi":{"ssid":"a","psk":"b","country":"US"},"mdns":{"hostname":"x"},"time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]}}"#;
        let parsed = parse_settings_json(input).unwrap();
        assert_eq!(parsed.audio.volume_pct, 50);
        assert!(!parsed.audio.muted);
    }

    #[test]
    fn parses_audio_block_with_explicit_values() {
        let input = r#"{
            "wifi":{"ssid":"a","psk":"b","country":"US"},
            "mdns":{"hostname":"x"},
            "time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]},
            "audio":{"volume_pct":75,"muted":true}
        }"#;
        let parsed = parse_settings_json(input).unwrap();
        assert_eq!(parsed.audio.volume_pct, 75);
        assert!(parsed.audio.muted);
    }

    #[test]
    fn tracker_fov_rejects_leading_plus() {
        // RFC 8259 §6 disallows a leading `+` on JSON numbers; the
        // shared `scan_f32` helper rejects it for both the RON and
        // JSON parsers. Pin: a `+62.0` lands on `BareParse`, not
        // silently sneaks through to `f32::from_str`.
        let input = r#"{
            "wifi":{"ssid":"a","psk":"b","country":"US"},
            "mdns":{"hostname":"x"},
            "time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]},
            "tracker":{"fov_h_deg":+62.0,"fov_v_deg":49.0,
                       "target_smoothing_alpha":1.0,
                       "flip_x":false,"flip_y":false}
        }"#;
        let err = parse_settings_json(input).unwrap_err();
        assert!(
            matches!(err, ConfigError::BareParse(_)),
            "expected BareParse on leading-plus, got {err:?}"
        );
    }

    #[test]
    fn audio_block_round_trips() {
        let original = full_config();
        let rendered = render_settings_json(&original, false).unwrap();
        let parsed = parse_settings_json(&rendered).unwrap();
        assert_eq!(parsed.audio, original.audio);
    }

    #[test]
    fn audio_volume_above_100_fails_validate() {
        let input = r#"{
            "wifi":{"ssid":"a","psk":"b","country":"US"},
            "mdns":{"hostname":"x"},
            "time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]},
            "audio":{"volume_pct":200,"muted":false}
        }"#;
        let err = parse_settings_json(input).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidVolumePct(200)),
            "got {err:?}"
        );
    }

    #[test]
    fn audio_rejects_duplicate_field() {
        let input = r#"{
            "wifi":{"ssid":"a","psk":"b","country":"US"},
            "mdns":{"hostname":"x"},
            "time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]},
            "audio":{"volume_pct":50,"volume_pct":75,"muted":false}
        }"#;
        let err = parse_settings_json(input).unwrap_err();
        match err {
            ConfigError::BareParse(msg) => assert!(
                msg.contains("duplicate audio field"),
                "expected duplicate-audio-field, got `{msg}`"
            ),
            other => panic!("expected BareParse, got {other:?}"),
        }
    }

    #[test]
    fn merge_passes_audio_through_unchanged() {
        // Audio has no redaction, so merge should always take the
        // `new` value verbatim — operators who tweak hostname via PUT
        // /settings should still get their volume/mute echo correctly.
        let current = full_config(); // volume 33, muted true
        let new = Config {
            audio: AudioConfig {
                volume_pct: 80,
                muted: false,
            },
            ..current.clone()
        };
        let merged = merge_settings_with_current(new, &current);
        assert_eq!(merged.audio.volume_pct, 80);
        assert!(!merged.audio.muted);
    }

    #[test]
    fn ron_parsed_config_round_trips_to_json() {
        // Pin: the bare RON parser and the bare JSON parser produce
        // structurally identical Config values for the same logical
        // schema instance.
        let from_ron = crate::bare::parse_ron_bare(
            r#"
            (
                wifi: ( ssid: "home", psk: "p", country: "US" ),
                mdns: ( hostname: "stackchan" ),
                time: ( tz: "UTC", sntp_servers: [ "pool.ntp.org" ] ),
            )
            "#,
        )
        .unwrap();
        let json = render_settings_json(&from_ron, false).unwrap();
        let from_json = parse_settings_json(&json).unwrap();
        assert_eq!(from_ron, from_json);
    }
}
