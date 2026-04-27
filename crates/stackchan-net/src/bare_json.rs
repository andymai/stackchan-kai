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

use crate::config::{Config, MdnsConfig, TimeConfig, WifiConfig, validate};
use crate::error::ConfigError;

/// Sentinel emitted by [`render_settings_json`] when `redact_psk = true`.
///
/// Rejected as a PSK value by [`parse_settings_json`] so a `GET → PUT`
/// round trip can't accidentally persist the redacted placeholder as
/// the real key (which would silently break Wi-Fi on next reboot).
pub const PSK_REDACTED: &str = "***";

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

/// Render a [`Config`] to a compact JSON string.
///
/// `redact_psk = true` replaces `wifi.psk` with `"***"` so the
/// output is safe to expose to unauthed callers (HTTP `GET /settings`
/// passes `true`). `redact_psk = false` round-trips losslessly with
/// [`parse_settings_json`].
///
/// # Errors
///
/// Currently infallible — kept as `Result` for symmetry with
/// [`crate::render_ron`].
pub fn render_settings_json(config: &Config, redact_psk: bool) -> Result<String, ConfigError> {
    let mut out = String::new();
    out.push('{');
    out.push_str("\"wifi\":{");
    push_string_field(&mut out, "ssid", &config.wifi.ssid);
    out.push(',');
    push_string_field(
        &mut out,
        "psk",
        if redact_psk {
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
    out.push_str("]}}");
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
                "wifi" => wifi = Some(self.parse_wifi()?),
                "mdns" => mdns = Some(self.parse_mdns()?),
                "time" => time = Some(self.parse_time()?),
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
                "ssid" => ssid = Some(value),
                "psk" => psk = Some(value),
                "country" => country = Some(value),
                other => return Err(bare_err("unknown wifi field", other)),
            }
            self.skip_ws();
            if !self.try_consume_char(',') && !self.peek_eq('}') {
                return Err(bare_err("expected ',' or '}' in wifi", ""));
            }
        }
        let psk = psk.ok_or_else(|| bare_err("missing wifi.psk", ""))?;
        if psk == PSK_REDACTED {
            return Err(bare_err(
                "wifi.psk is the redacted sentinel — supply the real key",
                "",
            ));
        }
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
                "hostname" => hostname = Some(value),
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
                "tz" => tz = Some(self.parse_string()?),
                "sntp_servers" => sntp_servers = Some(self.parse_string_list()?),
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
    fn rejects_redacted_psk_sentinel() {
        // GET /settings emits "psk":"***"; if an operator round-trips
        // that through PUT /settings unchanged, the firmware would
        // persist "***" as the real key and break Wi-Fi at next
        // boot. parse_settings_json must catch it.
        let body = render_settings_json(&full_config(), true).unwrap();
        let err = parse_settings_json(&body).unwrap_err();
        match err {
            ConfigError::BareParse(msg) => assert!(
                msg.contains("redacted"),
                "expected redacted-sentinel message, got `{msg}`"
            ),
            other => panic!("expected BareParse, got {other:?}"),
        }
    }

    #[test]
    fn render_uses_psk_redacted_constant() {
        // Pin: the sentinel string emitted by render is the same
        // string parse rejects.
        let body = render_settings_json(&full_config(), true).unwrap();
        assert!(body.contains(&format!("\"psk\":\"{PSK_REDACTED}\"")));
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
