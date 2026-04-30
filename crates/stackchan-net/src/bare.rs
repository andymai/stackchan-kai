//! Hand-rolled RON-subset parser + renderer for [`crate::Config`].
//!
//! The default `parse` feature in this crate uses `ron 0.10`, which
//! pulls `serde/std + base64/std`. Both break on
//! `xtensa-esp32s3-none-elf` (no std). This module is the firmware's
//! escape hatch: a tiny RON-subset parser that handles exactly the
//! schema v1 shape — top-level tuple struct, three nested tuple
//! structs, string fields, and one `Vec<String>` — and nothing else.
//!
//! It's symmetric with [`crate::parse_ron`] / [`crate::render_ron`]:
//! anything either side renders the other side parses, so SD round
//! trips and `PUT /settings` bodies stay lossless.
//!
//! The parser is deliberately minimal — no expression evaluation,
//! no enums, no maps, no unsigned/signed/float literals. Any schema
//! growth beyond v1 must extend this module in lockstep with the
//! serde derives.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write as _;

use crate::bare_json::TOKEN_REDACTED;
use crate::config::{
    AudioConfig, AuthConfig, Config, MdnsConfig, TimeConfig, TrackerSettings, WifiConfig, validate,
};
use crate::error::ConfigError;

/// Parse a schema-v1 RON document into a [`Config`] without using `serde` or `ron`.
///
/// The accepted grammar is the exact subset that [`crate::render_ron`]
/// emits when called with `PrettyConfig::new`, plus some tolerance for
/// hand-edits (whitespace, line comments, trailing commas).
///
/// # Errors
///
/// Returns [`ConfigError::BareParse`] on any structural mismatch
/// (missing field, unexpected token, runaway string), then runs the
/// shared [`validate`] gate so out-of-range values surface the same
/// `Invalid*` variants as the host parser.
pub fn parse_ron_bare(input: &str) -> Result<Config, ConfigError> {
    let mut p = Parser::new(input);
    let config = p.parse_config()?;
    validate(&config)?;
    Ok(config)
}

/// Render a [`Config`] to RON. Output matches what
/// [`crate::render_ron`] (host-side, serde + ron) emits, so a config
/// written by either side parses cleanly through the other.
///
/// # Errors
///
/// Currently infallible — kept as `Result` for symmetry with the
/// host renderer, which can fail under serde edge cases.
pub fn render_ron_bare(config: &Config) -> Result<String, ConfigError> {
    let mut out = String::new();
    out.push_str("(\n");
    out.push_str("    wifi: (\n");
    push_field(&mut out, "        ssid", &config.wifi.ssid);
    push_field(&mut out, "        psk", &config.wifi.psk);
    push_field(&mut out, "        country", &config.wifi.country);
    out.push_str("    ),\n");

    out.push_str("    mdns: (\n");
    push_field(&mut out, "        hostname", &config.mdns.hostname);
    out.push_str("    ),\n");

    out.push_str("    time: (\n");
    push_field(&mut out, "        tz", &config.time.tz);
    out.push_str("        sntp_servers: [\n");
    for s in &config.time.sntp_servers {
        out.push_str("            ");
        push_string_literal(&mut out, s);
        out.push_str(",\n");
    }
    out.push_str("        ],\n");
    out.push_str("    ),\n");

    out.push_str("    auth: (\n");
    push_field(&mut out, "        token", &config.auth.token);
    out.push_str("    ),\n");

    out.push_str("    audio: (\n");
    let _ = writeln!(out, "        volume_pct: {},", config.audio.volume_pct);
    let _ = writeln!(out, "        muted: {},", config.audio.muted);
    out.push_str("    ),\n");

    out.push_str("    tracker: (\n");
    let _ = writeln!(out, "        fov_h_deg: {},", config.tracker.fov_h_deg);
    let _ = writeln!(out, "        fov_v_deg: {},", config.tracker.fov_v_deg);
    let _ = writeln!(
        out,
        "        target_smoothing_alpha: {},",
        config.tracker.target_smoothing_alpha
    );
    let _ = writeln!(out, "        flip_x: {},", config.tracker.flip_x);
    let _ = writeln!(out, "        flip_y: {},", config.tracker.flip_y);
    out.push_str("    ),\n");

    out.push_str(")\n");
    Ok(out)
}

/// Helper: emit `        name: "value",\n`.
fn push_field(out: &mut String, indented_name: &str, value: &str) {
    out.push_str(indented_name);
    out.push_str(": ");
    push_string_literal(out, value);
    out.push_str(",\n");
}

/// Helper: emit a quoted RON string with `\\` and `\"` escapes.
fn push_string_literal(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            other => out.push(other),
        }
    }
    out.push('"');
}

/// Recursive-descent parser over a `&str` cursor. Slow but sized for
/// a config-file workload — schema v1 is well under 1 KiB.
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

    /// Top-level grammar: parse the schema-v1 outer tuple struct.
    fn parse_config(&mut self) -> Result<Config, ConfigError> {
        self.skip_ws_and_comments();
        self.expect_char('(')?;
        let mut wifi: Option<WifiConfig> = None;
        let mut mdns: Option<MdnsConfig> = None;
        let mut time: Option<TimeConfig> = None;
        let mut auth: Option<AuthConfig> = None;
        let mut audio: Option<AudioConfig> = None;
        let mut tracker: Option<TrackerSettings> = None;

        loop {
            self.skip_ws_and_comments();
            if self.try_consume_char(')') {
                break;
            }
            let key = self.read_ident()?;
            self.skip_ws_and_comments();
            self.expect_char(':')?;
            self.skip_ws_and_comments();
            match key {
                "wifi" => wifi = Some(self.parse_wifi()?),
                "mdns" => mdns = Some(self.parse_mdns()?),
                "time" => time = Some(self.parse_time()?),
                "auth" => auth = Some(self.parse_auth()?),
                "audio" => audio = Some(self.parse_audio()?),
                "tracker" => tracker = Some(self.parse_tracker()?),
                other => return Err(bare_err("unknown top-level field", other)),
            }
            self.skip_ws_and_comments();
            // Trailing comma optional; closing `)` also OK.
            if !self.try_consume_char(',') && !self.peek_eq(')') {
                return Err(bare_err("expected ',' or ')'", ""));
            }
        }

        Ok(Config {
            wifi: wifi.ok_or_else(|| bare_err("missing field 'wifi'", ""))?,
            mdns: mdns.ok_or_else(|| bare_err("missing field 'mdns'", ""))?,
            time: time.ok_or_else(|| bare_err("missing field 'time'", ""))?,
            // `auth`, `audio`, and `tracker` are optional for
            // migration: SD cards written before each block landed
            // lack them, and the defaults match the firmware's prior
            // hard-coded behaviour.
            auth: auth.unwrap_or_default(),
            audio: audio.unwrap_or_default(),
            tracker: tracker.unwrap_or_default(),
        })
    }

    /// Parse the `wifi: (ssid, psk, country)` block.
    fn parse_wifi(&mut self) -> Result<WifiConfig, ConfigError> {
        self.expect_char('(')?;
        let mut ssid: Option<String> = None;
        let mut psk: Option<String> = None;
        let mut country: Option<String> = None;
        loop {
            self.skip_ws_and_comments();
            if self.try_consume_char(')') {
                break;
            }
            let key = self.read_ident()?;
            self.skip_ws_and_comments();
            self.expect_char(':')?;
            self.skip_ws_and_comments();
            let value = self.parse_string()?;
            match key {
                "ssid" => ssid = Some(value),
                "psk" => psk = Some(value),
                "country" => country = Some(value),
                other => return Err(bare_err("unknown wifi field", other)),
            }
            self.skip_ws_and_comments();
            if !self.try_consume_char(',') && !self.peek_eq(')') {
                return Err(bare_err("expected ',' or ')' in wifi", ""));
            }
        }
        Ok(WifiConfig {
            ssid: ssid.ok_or_else(|| bare_err("missing wifi.ssid", ""))?,
            psk: psk.ok_or_else(|| bare_err("missing wifi.psk", ""))?,
            country: country.ok_or_else(|| bare_err("missing wifi.country", ""))?,
        })
    }

    /// Parse the `mdns: (hostname)` block.
    fn parse_mdns(&mut self) -> Result<MdnsConfig, ConfigError> {
        self.expect_char('(')?;
        let mut hostname: Option<String> = None;
        loop {
            self.skip_ws_and_comments();
            if self.try_consume_char(')') {
                break;
            }
            let key = self.read_ident()?;
            self.skip_ws_and_comments();
            self.expect_char(':')?;
            self.skip_ws_and_comments();
            let value = self.parse_string()?;
            match key {
                "hostname" => hostname = Some(value),
                other => return Err(bare_err("unknown mdns field", other)),
            }
            self.skip_ws_and_comments();
            if !self.try_consume_char(',') && !self.peek_eq(')') {
                return Err(bare_err("expected ',' or ')' in mdns", ""));
            }
        }
        Ok(MdnsConfig {
            hostname: hostname.ok_or_else(|| bare_err("missing mdns.hostname", ""))?,
        })
    }

    /// Parse the `time: (tz, sntp_servers)` block.
    fn parse_time(&mut self) -> Result<TimeConfig, ConfigError> {
        self.expect_char('(')?;
        let mut tz: Option<String> = None;
        let mut sntp_servers: Option<Vec<String>> = None;
        loop {
            self.skip_ws_and_comments();
            if self.try_consume_char(')') {
                break;
            }
            let key = self.read_ident()?;
            self.skip_ws_and_comments();
            self.expect_char(':')?;
            self.skip_ws_and_comments();
            match key {
                "tz" => tz = Some(self.parse_string()?),
                "sntp_servers" => sntp_servers = Some(self.parse_string_list()?),
                other => return Err(bare_err("unknown time field", other)),
            }
            self.skip_ws_and_comments();
            if !self.try_consume_char(',') && !self.peek_eq(')') {
                return Err(bare_err("expected ',' or ')' in time", ""));
            }
        }
        Ok(TimeConfig {
            tz: tz.ok_or_else(|| bare_err("missing time.tz", ""))?,
            sntp_servers: sntp_servers.ok_or_else(|| bare_err("missing time.sntp_servers", ""))?,
        })
    }

    /// Parse the `auth: (token)` block. An empty block is permitted
    /// and yields [`AuthConfig::default`] — operators who haven't
    /// configured auth keep the LAN-open behaviour.
    fn parse_auth(&mut self) -> Result<AuthConfig, ConfigError> {
        self.expect_char('(')?;
        let mut token: Option<String> = None;
        loop {
            self.skip_ws_and_comments();
            if self.try_consume_char(')') {
                break;
            }
            let key = self.read_ident()?;
            self.skip_ws_and_comments();
            self.expect_char(':')?;
            self.skip_ws_and_comments();
            let value = self.parse_string()?;
            match key {
                "token" => token = Some(value),
                other => return Err(bare_err("unknown auth field", other)),
            }
            self.skip_ws_and_comments();
            if !self.try_consume_char(',') && !self.peek_eq(')') {
                return Err(bare_err("expected ',' or ')' in auth", ""));
            }
        }
        let token = token.unwrap_or_default();
        // Symmetric with the JSON parser: a literal `***` on disk is
        // almost certainly a copy-paste from `GET /settings`, not an
        // intentional value. Catch it here so the operator gets a
        // clear error instead of a silently-locked-out device.
        if token == TOKEN_REDACTED {
            return Err(bare_err(
                "auth.token is the redacted sentinel — supply the real token",
                "",
            ));
        }
        Ok(AuthConfig { token })
    }

    /// Parse the `audio: (volume_pct, muted)` block. Volume is an
    /// integer literal (RON `u8`), mute is a bare `true` / `false`.
    fn parse_audio(&mut self) -> Result<AudioConfig, ConfigError> {
        self.expect_char('(')?;
        let mut volume_pct: Option<u8> = None;
        let mut muted: Option<bool> = None;
        loop {
            self.skip_ws_and_comments();
            if self.try_consume_char(')') {
                break;
            }
            let key = self.read_ident()?;
            self.skip_ws_and_comments();
            self.expect_char(':')?;
            self.skip_ws_and_comments();
            match key {
                "volume_pct" => volume_pct = Some(self.parse_u8()?),
                "muted" => muted = Some(self.parse_bool()?),
                other => return Err(bare_err("unknown audio field", other)),
            }
            self.skip_ws_and_comments();
            if !self.try_consume_char(',') && !self.peek_eq(')') {
                return Err(bare_err("expected ',' or ')' in audio", ""));
            }
        }
        let defaults = AudioConfig::default();
        Ok(AudioConfig {
            volume_pct: volume_pct.unwrap_or(defaults.volume_pct),
            muted: muted.unwrap_or(defaults.muted),
        })
    }

    /// Parse the `tracker: (...)` block. Five fields, all optional;
    /// missing fields fall back to [`TrackerSettings::DEFAULT`] so a
    /// SD card written before this block existed reproduces the
    /// pre-runtime-config tracker behaviour exactly.
    fn parse_tracker(&mut self) -> Result<TrackerSettings, ConfigError> {
        self.expect_char('(')?;
        // `pan_fov` / `tilt_fov` rather than `fov_h_deg` / `fov_v_deg`
        // for the locals — clippy's `similar_names` flags the latter
        // as too close. The struct fields keep the on-the-wire names.
        let mut pan_fov: Option<f32> = None;
        let mut tilt_fov: Option<f32> = None;
        let mut alpha: Option<f32> = None;
        let mut flip_x: Option<bool> = None;
        let mut flip_y: Option<bool> = None;
        loop {
            self.skip_ws_and_comments();
            if self.try_consume_char(')') {
                break;
            }
            let key = self.read_ident()?;
            self.skip_ws_and_comments();
            self.expect_char(':')?;
            self.skip_ws_and_comments();
            match key {
                "fov_h_deg" => pan_fov = Some(self.parse_f32()?),
                "fov_v_deg" => tilt_fov = Some(self.parse_f32()?),
                "target_smoothing_alpha" => {
                    alpha = Some(self.parse_f32()?);
                }
                "flip_x" => flip_x = Some(self.parse_bool()?),
                "flip_y" => flip_y = Some(self.parse_bool()?),
                other => return Err(bare_err("unknown tracker field", other)),
            }
            self.skip_ws_and_comments();
            if !self.try_consume_char(',') && !self.peek_eq(')') {
                return Err(bare_err("expected ',' or ')' in tracker", ""));
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

    /// Parse a contiguous run of decimal digits as a `u8`. Used for
    /// `audio.volume_pct`. Range gating is left to [`validate`] so the
    /// out-of-range surface lands on `ConfigError::InvalidVolumePct`
    /// (with the offending value) rather than a generic `BareParse`.
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
        // Parse as u16 first so 0..=255 + a few extra digits land on
        // BareParse cleanly rather than wrapping silently. Then cast
        // down — the validator catches > 100 on the audio path.
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

    /// Parse a contiguous run of number-shaped bytes as `f32`. Used
    /// for the tracker block's `fov_h_deg` / `fov_v_deg` /
    /// `target_smoothing_alpha`. Accepts a leading `-`, decimal point,
    /// and exponent — delegated to `f32::from_str` for the heavy
    /// lifting. Range checks live in [`validate`].
    fn parse_f32(&mut self) -> Result<f32, ConfigError> {
        let bytes = self.input.as_bytes();
        let mut end = 0;
        while end < bytes.len() {
            let b = bytes[end];
            if b == b'-' || b == b'+' || b == b'.' || b == b'e' || b == b'E' || b.is_ascii_digit() {
                end += 1;
            } else {
                break;
            }
        }
        if end == 0 {
            return Err(bare_err("expected float literal", ""));
        }
        let (digits, rest) = self.input.split_at(end);
        let parsed: f32 = digits
            .parse()
            .map_err(|_| bare_err("not an f32 literal", digits))?;
        if !parsed.is_finite() {
            return Err(bare_err("f32 literal is non-finite", digits));
        }
        self.input = rest;
        Ok(parsed)
    }

    /// Parse a bare `true` or `false` literal.
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

    /// Parse `[ "...", "...", ]` into a `Vec<String>`.
    fn parse_string_list(&mut self) -> Result<Vec<String>, ConfigError> {
        self.expect_char('[')?;
        let mut out: Vec<String> = Vec::new();
        loop {
            self.skip_ws_and_comments();
            if self.try_consume_char(']') {
                break;
            }
            let s = self.parse_string()?;
            out.push(s);
            self.skip_ws_and_comments();
            if !self.try_consume_char(',') && !self.peek_eq(']') {
                return Err(bare_err("expected ',' or ']' in list", ""));
            }
        }
        Ok(out)
    }

    /// Parse a `"..."` string literal with `\\` and `\"` escapes.
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
                    other => return Err(bare_err("unsupported escape", &other.to_string())),
                }
                self.advance(esc.len_utf8());
            } else {
                out.push(ch);
                self.advance(ch.len_utf8());
            }
        }
    }

    /// Read a bare identifier `[a-zA-Z_][a-zA-Z0-9_]*`. Returns a
    /// borrowed slice into `input` that's valid only until the next
    /// `advance` past it; callers must `match` on it before
    /// continuing.
    fn read_ident(&mut self) -> Result<&'a str, ConfigError> {
        let bytes = self.input.as_bytes();
        if bytes.is_empty() {
            return Err(bare_err("expected identifier, got EOF", ""));
        }
        let first = bytes[0];
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return Err(bare_err("expected identifier", ""));
        }
        let mut end = 1;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        let (ident, rest) = self.input.split_at(end);
        self.input = rest;
        Ok(ident)
    }

    /// Skip ASCII whitespace and `// line comments` until a token.
    fn skip_ws_and_comments(&mut self) {
        loop {
            let bytes = self.input.as_bytes();
            // Skip whitespace.
            let mut i = 0;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i > 0 {
                self.advance(i);
                continue;
            }
            // Try line comment.
            if self.input.starts_with("//") {
                let bytes = self.input.as_bytes();
                let mut j = 2;
                while j < bytes.len() && bytes[j] != b'\n' {
                    j += 1;
                }
                self.advance(j);
                continue;
            }
            return;
        }
    }

    /// Slide the cursor forward `n` bytes. Caller guarantees `n` is
    /// at a UTF-8 char boundary (we only call this with `len_utf8()`
    /// or after byte-only matches like `'('`).
    fn advance(&mut self, n: usize) {
        self.input = &self.input[n..];
    }

    /// Peek the next char without consuming.
    fn peek_char(&self) -> Option<char> {
        self.input.chars().next()
    }

    /// True iff the next byte is `c`.
    fn peek_eq(&self, c: char) -> bool {
        self.input.as_bytes().first().copied() == Some(c as u8)
    }

    /// Consume `c` if it's next; otherwise leave the cursor put.
    fn try_consume_char(&mut self, c: char) -> bool {
        if self.peek_eq(c) {
            self.advance(1);
            true
        } else {
            false
        }
    }

    /// Consume `c` or return a parse error.
    fn expect_char(&mut self, c: char) -> Result<(), ConfigError> {
        if self.try_consume_char(c) {
            Ok(())
        } else {
            Err(bare_err("expected char", &c.to_string()))
        }
    }
}

/// Build a `BareParse` error. The format-arg pattern keeps the
/// firmware-side `defmt::Debug2Format` log line readable.
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
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
(
    wifi: (
        ssid: "home",
        psk: "redacted",
        country: "US",
    ),
    mdns: (
        hostname: "stackchan",
    ),
    time: (
        tz: "UTC",
        sntp_servers: ["pool.ntp.org"],
    ),
)
"#;

    #[test]
    fn parses_minimal_fixture() {
        let cfg = parse_ron_bare(FIXTURE).unwrap();
        assert_eq!(cfg.wifi.ssid, "home");
        assert_eq!(cfg.wifi.psk, "redacted");
        assert_eq!(cfg.wifi.country, "US");
        assert_eq!(cfg.mdns.hostname, "stackchan");
        assert_eq!(cfg.time.tz, "UTC");
        assert_eq!(cfg.time.sntp_servers, vec!["pool.ntp.org".to_string()]);
    }

    #[test]
    fn handles_line_comments_and_trailing_commas() {
        let s = r#"
            // top comment
            (
                wifi: ( ssid: "n", psk: "p", country: "JP", ),
                mdns: ( hostname: "h" ), // trailing
                time: ( tz: "UTC", sntp_servers: ["a","b",], ),
            )
        "#;
        let cfg = parse_ron_bare(s).unwrap();
        assert_eq!(
            cfg.time.sntp_servers,
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn handles_string_escapes() {
        let s = r#"
            (
                wifi: ( ssid: "foo\"bar", psk: "back\\slash", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
            )
        "#;
        let cfg = parse_ron_bare(s).unwrap();
        assert_eq!(cfg.wifi.ssid, "foo\"bar");
        assert_eq!(cfg.wifi.psk, "back\\slash");
    }

    #[test]
    fn renders_then_re_parses() {
        let original = parse_ron_bare(FIXTURE).unwrap();
        let rendered = render_ron_bare(&original).unwrap();
        let re_parsed = parse_ron_bare(&rendered).unwrap();
        assert_eq!(original, re_parsed);
    }

    #[test]
    fn render_output_round_trips_through_serde_path() {
        // Sanity: anything our renderer emits, the serde-side parser
        // (gated behind feature `parse`) should also accept. Only
        // exercised when running the host test suite, which has the
        // feature on by default.
        #[cfg(feature = "parse")]
        {
            let original = parse_ron_bare(FIXTURE).unwrap();
            let rendered = render_ron_bare(&original).unwrap();
            let via_serde = crate::parse_ron(&rendered).unwrap();
            assert_eq!(original, via_serde);
        }
    }

    #[test]
    fn rejects_missing_field() {
        let err = parse_ron_bare("(wifi: (ssid: \"x\", psk: \"y\", country: \"US\"))").unwrap_err();
        assert!(matches!(err, ConfigError::BareParse(_)), "got {err:?}");
    }

    #[test]
    fn missing_auth_block_defaults_to_empty_token() {
        // Schema-v1 SD cards (written before the auth block landed)
        // omit `auth:` entirely. The parser must accept that and fall
        // back to the default empty token so a firmware bump doesn't
        // brick existing kits.
        let cfg = parse_ron_bare(FIXTURE).unwrap();
        assert_eq!(cfg.auth.token, "");
    }

    #[test]
    fn parses_auth_block_with_token() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                auth: ( token: "shared-secret" ),
            )
        "#;
        let cfg = parse_ron_bare(s).unwrap();
        assert_eq!(cfg.auth.token, "shared-secret");
    }

    #[test]
    fn round_trips_with_token() {
        // The renderer always emits an auth block; pin that a token
        // round-trips losslessly through render → re-parse.
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                auth: ( token: "abc-123" ),
            )
        "#;
        let cfg = parse_ron_bare(s).unwrap();
        let rendered = render_ron_bare(&cfg).unwrap();
        let reparsed = parse_ron_bare(&rendered).unwrap();
        assert_eq!(cfg, reparsed);
        assert_eq!(reparsed.auth.token, "abc-123");
    }

    #[test]
    fn missing_audio_block_defaults_to_50_unmuted() {
        let cfg = parse_ron_bare(FIXTURE).unwrap();
        assert_eq!(cfg.audio.volume_pct, 50);
        assert!(!cfg.audio.muted);
    }

    #[test]
    fn parses_audio_block_with_explicit_values() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                audio: ( volume_pct: 75, muted: true ),
            )
        "#;
        let cfg = parse_ron_bare(s).unwrap();
        assert_eq!(cfg.audio.volume_pct, 75);
        assert!(cfg.audio.muted);
    }

    #[test]
    fn round_trips_with_audio_block() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                audio: ( volume_pct: 33, muted: true ),
            )
        "#;
        let cfg = parse_ron_bare(s).unwrap();
        let rendered = render_ron_bare(&cfg).unwrap();
        let reparsed = parse_ron_bare(&rendered).unwrap();
        assert_eq!(cfg, reparsed);
        assert_eq!(reparsed.audio.volume_pct, 33);
        assert!(reparsed.audio.muted);
    }

    #[test]
    fn audio_volume_above_100_fails_validate() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                audio: ( volume_pct: 200, muted: false ),
            )
        "#;
        let err = parse_ron_bare(s).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidVolumePct(200)),
            "got {err:?}"
        );
    }

    #[test]
    fn validates_after_parse() {
        // Lowercase country slips through bare parse but fails the
        // shared validate gate.
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "us" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["a"] ),
            )
        "#;
        let err = parse_ron_bare(s).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidCountry(_)), "got {err:?}");
    }
}
