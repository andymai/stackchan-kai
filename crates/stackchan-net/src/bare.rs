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
    AudioConfig, AuthConfig, BehaviorConfig, Config, EspNowConfig, MdnsConfig, TimeConfig,
    TrackerSettings, WifiConfig, validate_for_disk,
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
/// strict [`validate_for_disk`] gate so out-of-range values surface
/// the same `Invalid*` variants as the host parser, and any literal
/// redaction sentinel (`"***"`) on a secret field surfaces as
/// [`ConfigError::RedactionSentinelOnDisk`].
pub fn parse_ron_bare(input: &str) -> Result<Config, ConfigError> {
    let mut p = Parser::new(input);
    let config = p.parse_config()?;
    validate_for_disk(&config)?;
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
#[allow(clippy::too_many_lines)]
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

    out.push_str("    esp_now: (\n");
    let _ = writeln!(out, "        enabled: {},", config.esp_now.enabled);
    push_field(&mut out, "        pmk_hex", &config.esp_now.pmk_hex);
    push_field(&mut out, "        peer_mac", &config.esp_now.peer_mac);
    push_field(&mut out, "        lmk_hex", &config.esp_now.lmk_hex);
    match config.esp_now.channel {
        Some(ch) => {
            let _ = writeln!(out, "        channel: Some({ch}),");
        }
        None => out.push_str("        channel: None,\n"),
    }
    let _ = writeln!(out, "        tx_rate_hz: {},", config.esp_now.tx_rate_hz);
    out.push_str("    ),\n");

    out.push_str("    behavior: (\n");
    let _ = writeln!(
        out,
        "        soliloquy_enabled: {},",
        config.behavior.soliloquy_enabled
    );
    let _ = writeln!(
        out,
        "        hourly_chime_enabled: {},",
        config.behavior.hourly_chime_enabled
    );
    let _ = writeln!(
        out,
        "        battery_icon_enabled: {},",
        config.behavior.battery_icon_enabled
    );
    let _ = writeln!(
        out,
        "        toast_overlay_enabled: {},",
        config.behavior.toast_overlay_enabled
    );
    let _ = writeln!(
        out,
        "        auto_torque_release_ms: {},",
        config.behavior.auto_torque_release_ms
    );
    push_field(
        &mut out,
        "        audio_debug_udp_target",
        &config.behavior.audio_debug_udp_target,
    );
    push_field(
        &mut out,
        "        agent_sidecar_url",
        &config.behavior.agent_sidecar_url,
    );
    push_field(
        &mut out,
        "        agent_sidecar_token",
        &config.behavior.agent_sidecar_token,
    );
    push_field(
        &mut out,
        "        follower_leader_hostname",
        &config.behavior.follower_leader_hostname,
    );
    let _ = writeln!(
        out,
        "        wake_word_enabled: {},",
        config.behavior.wake_word_enabled
    );
    let _ = writeln!(
        out,
        "        wake_word_threshold: {},",
        config.behavior.wake_word_threshold
    );
    let _ = writeln!(
        out,
        "        wake_word_arena_kib: {},",
        config.behavior.wake_word_arena_kib
    );
    push_field(
        &mut out,
        "        persona_name",
        &config.behavior.persona_name,
    );
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
        let mut esp_now: Option<EspNowConfig> = None;
        let mut behavior: Option<BehaviorConfig> = None;

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
                "esp_now" => {
                    if esp_now.is_some() {
                        return Err(bare_err("duplicate top-level field", "esp_now"));
                    }
                    esp_now = Some(self.parse_esp_now()?);
                }
                "behavior" => {
                    if behavior.is_some() {
                        return Err(bare_err("duplicate top-level field", "behavior"));
                    }
                    behavior = Some(self.parse_behavior()?);
                }
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
            // `auth`, `audio`, `tracker`, and `esp_now` are optional for
            // migration: SD cards written before each block landed
            // lack them, and the defaults match the firmware's prior
            // hard-coded behaviour.
            auth: auth.unwrap_or_default(),
            audio: audio.unwrap_or_default(),
            tracker: tracker.unwrap_or_default(),
            esp_now: esp_now.unwrap_or_default(),
            behavior: behavior.unwrap_or_default(),
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
                "hostname" => {
                    if hostname.is_some() {
                        return Err(bare_err("duplicate mdns field", "hostname"));
                    }
                    hostname = Some(value);
                }
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
                "token" => {
                    if token.is_some() {
                        return Err(bare_err("duplicate auth field", "token"));
                    }
                    token = Some(value);
                }
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

    /// Parse the optional `esp_now: (...)` block. All inner fields
    /// are optional and fall back to [`EspNowConfig::DEFAULT`].
    fn parse_esp_now(&mut self) -> Result<EspNowConfig, ConfigError> {
        self.expect_char('(')?;
        let mut enabled: Option<bool> = None;
        let mut pmk_hex: Option<String> = None;
        let mut peer_mac: Option<String> = None;
        let mut lmk_hex: Option<String> = None;
        let mut channel: Option<Option<u8>> = None;
        let mut tx_rate_hz: Option<u8> = None;
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
                "enabled" => {
                    if enabled.is_some() {
                        return Err(bare_err("duplicate esp_now field", "enabled"));
                    }
                    enabled = Some(self.parse_bool()?);
                }
                "pmk_hex" => {
                    if pmk_hex.is_some() {
                        return Err(bare_err("duplicate esp_now field", "pmk_hex"));
                    }
                    pmk_hex = Some(self.parse_string()?);
                }
                "peer_mac" => {
                    if peer_mac.is_some() {
                        return Err(bare_err("duplicate esp_now field", "peer_mac"));
                    }
                    peer_mac = Some(self.parse_string()?);
                }
                "lmk_hex" => {
                    if lmk_hex.is_some() {
                        return Err(bare_err("duplicate esp_now field", "lmk_hex"));
                    }
                    lmk_hex = Some(self.parse_string()?);
                }
                "channel" => {
                    if channel.is_some() {
                        return Err(bare_err("duplicate esp_now field", "channel"));
                    }
                    channel = Some(self.parse_optional_u8()?);
                }
                "tx_rate_hz" => {
                    if tx_rate_hz.is_some() {
                        return Err(bare_err("duplicate esp_now field", "tx_rate_hz"));
                    }
                    tx_rate_hz = Some(self.parse_u8()?);
                }
                other => return Err(bare_err("unknown esp_now field", other)),
            }
            self.skip_ws_and_comments();
            if !self.try_consume_char(',') && !self.peek_eq(')') {
                return Err(bare_err("expected ',' or ')' in esp_now", ""));
            }
        }
        let defaults = EspNowConfig::DEFAULT;
        Ok(EspNowConfig {
            enabled: enabled.unwrap_or(defaults.enabled),
            pmk_hex: pmk_hex.unwrap_or(defaults.pmk_hex),
            peer_mac: peer_mac.unwrap_or(defaults.peer_mac),
            lmk_hex: lmk_hex.unwrap_or(defaults.lmk_hex),
            channel: channel.unwrap_or(defaults.channel),
            tx_rate_hz: tx_rate_hz.unwrap_or(defaults.tx_rate_hz),
        })
    }

    /// Parse the `behavior: (...)` block. All fields default to
    /// `false` if absent.
    #[allow(clippy::too_many_lines)] // one match arm per flag; splitting helpers wouldn't read clearer
    fn parse_behavior(&mut self) -> Result<BehaviorConfig, ConfigError> {
        self.expect_char('(')?;
        let mut soliloquy_enabled: Option<bool> = None;
        let mut hourly_chime_enabled: Option<bool> = None;
        let mut battery_icon_enabled: Option<bool> = None;
        let mut toast_overlay_enabled: Option<bool> = None;
        let mut auto_torque_release_ms: Option<u32> = None;
        let mut audio_debug_udp_target: Option<String> = None;
        let mut agent_sidecar_url: Option<String> = None;
        let mut agent_sidecar_token: Option<String> = None;
        let mut follower_leader_hostname: Option<String> = None;
        let mut wake_word_enabled: Option<bool> = None;
        let mut wake_word_threshold: Option<i8> = None;
        let mut wake_word_arena_kib: Option<u32> = None;
        let mut persona_name: Option<String> = None;
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
                "soliloquy_enabled" => {
                    if soliloquy_enabled.is_some() {
                        return Err(bare_err("duplicate behavior field", "soliloquy_enabled"));
                    }
                    soliloquy_enabled = Some(self.parse_bool()?);
                }
                "hourly_chime_enabled" => {
                    if hourly_chime_enabled.is_some() {
                        return Err(bare_err("duplicate behavior field", "hourly_chime_enabled"));
                    }
                    hourly_chime_enabled = Some(self.parse_bool()?);
                }
                "battery_icon_enabled" => {
                    if battery_icon_enabled.is_some() {
                        return Err(bare_err("duplicate behavior field", "battery_icon_enabled"));
                    }
                    battery_icon_enabled = Some(self.parse_bool()?);
                }
                "toast_overlay_enabled" => {
                    if toast_overlay_enabled.is_some() {
                        return Err(bare_err(
                            "duplicate behavior field",
                            "toast_overlay_enabled",
                        ));
                    }
                    toast_overlay_enabled = Some(self.parse_bool()?);
                }
                "auto_torque_release_ms" => {
                    if auto_torque_release_ms.is_some() {
                        return Err(bare_err(
                            "duplicate behavior field",
                            "auto_torque_release_ms",
                        ));
                    }
                    auto_torque_release_ms = Some(self.parse_u32()?);
                }
                "audio_debug_udp_target" => {
                    if audio_debug_udp_target.is_some() {
                        return Err(bare_err(
                            "duplicate behavior field",
                            "audio_debug_udp_target",
                        ));
                    }
                    audio_debug_udp_target = Some(self.parse_string()?);
                }
                "agent_sidecar_url" => {
                    if agent_sidecar_url.is_some() {
                        return Err(bare_err("duplicate behavior field", "agent_sidecar_url"));
                    }
                    agent_sidecar_url = Some(self.parse_string()?);
                }
                "agent_sidecar_token" => {
                    if agent_sidecar_token.is_some() {
                        return Err(bare_err("duplicate behavior field", "agent_sidecar_token"));
                    }
                    agent_sidecar_token = Some(self.parse_string()?);
                }
                "follower_leader_hostname" => {
                    if follower_leader_hostname.is_some() {
                        return Err(bare_err(
                            "duplicate behavior field",
                            "follower_leader_hostname",
                        ));
                    }
                    follower_leader_hostname = Some(self.parse_string()?);
                }
                "wake_word_enabled" => {
                    if wake_word_enabled.is_some() {
                        return Err(bare_err("duplicate behavior field", "wake_word_enabled"));
                    }
                    wake_word_enabled = Some(self.parse_bool()?);
                }
                "wake_word_threshold" => {
                    if wake_word_threshold.is_some() {
                        return Err(bare_err("duplicate behavior field", "wake_word_threshold"));
                    }
                    wake_word_threshold = Some(self.parse_i8()?);
                }
                "wake_word_arena_kib" => {
                    if wake_word_arena_kib.is_some() {
                        return Err(bare_err("duplicate behavior field", "wake_word_arena_kib"));
                    }
                    wake_word_arena_kib = Some(self.parse_u32()?);
                }
                "persona_name" => {
                    if persona_name.is_some() {
                        return Err(bare_err("duplicate behavior field", "persona_name"));
                    }
                    persona_name = Some(self.parse_string()?);
                }
                other => return Err(bare_err("unknown behavior field", other)),
            }
            self.skip_ws_and_comments();
            if !self.try_consume_char(',') && !self.peek_eq(')') {
                return Err(bare_err("expected ',' or ')' in behavior", ""));
            }
        }
        Ok(BehaviorConfig {
            soliloquy_enabled: soliloquy_enabled.unwrap_or(false),
            hourly_chime_enabled: hourly_chime_enabled.unwrap_or(false),
            battery_icon_enabled: battery_icon_enabled.unwrap_or(false),
            toast_overlay_enabled: toast_overlay_enabled.unwrap_or(false),
            auto_torque_release_ms: auto_torque_release_ms.unwrap_or(0),
            audio_debug_udp_target: audio_debug_udp_target.unwrap_or_default(),
            agent_sidecar_url: agent_sidecar_url.unwrap_or_default(),
            agent_sidecar_token: agent_sidecar_token.unwrap_or_default(),
            follower_leader_hostname: follower_leader_hostname.unwrap_or_default(),
            wake_word_enabled: wake_word_enabled.unwrap_or(false),
            wake_word_threshold: wake_word_threshold.unwrap_or(100),
            wake_word_arena_kib: wake_word_arena_kib.unwrap_or(64),
            persona_name: persona_name.unwrap_or_default(),
        })
    }

    /// Parse `Some(<u8>)` or `None` — the RON encoding for an
    /// `Option<u8>` in our renderer.
    fn parse_optional_u8(&mut self) -> Result<Option<u8>, ConfigError> {
        if self.input.starts_with("None") {
            self.advance("None".len());
            return Ok(None);
        }
        if self.input.starts_with("Some") {
            self.advance("Some".len());
            self.skip_ws_and_comments();
            self.expect_char('(')?;
            self.skip_ws_and_comments();
            let v = self.parse_u8()?;
            self.skip_ws_and_comments();
            self.expect_char(')')?;
            return Ok(Some(v));
        }
        Err(bare_err("expected Some(...) or None", ""))
    }

    /// Parse a contiguous run of decimal digits as a `u32`. Used for
    /// `behavior.auto_torque_release_ms`. Any digits past `u32::MAX`
    /// land on `BareParse` rather than wrapping silently.
    fn parse_u32(&mut self) -> Result<u32, ConfigError> {
        let bytes = self.input.as_bytes();
        let mut end = 0;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == 0 {
            return Err(bare_err("expected unsigned integer", ""));
        }
        let (digits, rest) = self.input.split_at(end);
        let parsed: u32 = digits
            .parse()
            .map_err(|_| bare_err("not a u32 literal", digits))?;
        self.input = rest;
        Ok(parsed)
    }

    /// Parse a signed decimal literal as an `i8`. Used for
    /// `behavior.wake_word_threshold`. Accepts an optional leading
    /// `-`; out-of-range values land on `BareParse` rather than
    /// wrapping silently.
    fn parse_i8(&mut self) -> Result<i8, ConfigError> {
        let negative = self.try_consume_char('-');
        let bytes = self.input.as_bytes();
        let mut end = 0;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == 0 {
            return Err(bare_err("expected signed integer", ""));
        }
        let (digits, rest) = self.input.split_at(end);
        let magnitude: i16 = digits
            .parse()
            .map_err(|_| bare_err("i8 literal out of range", digits))?;
        let signed = if negative { -magnitude } else { magnitude };
        if !(i16::from(i8::MIN)..=i16::from(i8::MAX)).contains(&signed) {
            return Err(bare_err("i8 literal out of range", digits));
        }
        self.input = rest;
        #[allow(clippy::cast_possible_truncation)]
        Ok(signed as i8)
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
    /// `target_smoothing_alpha`. Delegates to [`scan_f32`] so the
    /// JSON parser in [`crate::bare_json`] consumes the identical
    /// grammar — single source of truth for number-shape recognition.
    fn parse_f32(&mut self) -> Result<f32, ConfigError> {
        scan_f32(&mut self.input)
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

/// Read a contiguous run of number-shaped bytes off the front of
/// `input` and parse it as `f32`.
///
/// Accepted grammar: optional leading `-`, decimal digits, optional
/// fractional part (`.digits`), optional exponent (`e[-]?digits` or
/// `E[-]?digits`). A leading `+` is **rejected** — neither RFC 8259
/// (JSON) nor the RON subset this crate emits ever produces one, so
/// accepting it would let inputs round-trip through to f32 in
/// shapes the rest of the pipeline never expects.
///
/// On success the consumed prefix is sliced off `*input` and the
/// parsed value is returned. Non-finite parses (`inf`, `NaN`) are
/// rejected even though they're not normally producible by the
/// grammar, as belt-and-braces.
///
/// Shared between [`Parser::parse_f32`] (RON) and
/// [`crate::bare_json::Parser::parse_f32`] (JSON) so the two wire
/// formats can never disagree on which byte sequences are numbers.
pub(crate) fn scan_f32(input: &mut &str) -> Result<f32, ConfigError> {
    let bytes = input.as_bytes();
    let mut end = 0;
    while end < bytes.len() {
        let b = bytes[end];
        if b == b'-' || b == b'.' || b == b'e' || b == b'E' || b.is_ascii_digit() {
            end += 1;
        } else {
            break;
        }
    }
    if end == 0 {
        return Err(bare_err("expected float literal", ""));
    }
    let (digits, rest) = input.split_at(end);
    let parsed: f32 = digits
        .parse()
        .map_err(|_| bare_err("not an f32 literal", digits))?;
    if !parsed.is_finite() {
        return Err(bare_err("f32 literal is non-finite", digits));
    }
    *input = rest;
    Ok(parsed)
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
    fn round_trips_with_agent_sidecar_token() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                behavior: ( agent_sidecar_token: "sk-sidecar-1234" ),
            )
        "#;
        let cfg = parse_ron_bare(s).unwrap();
        let rendered = render_ron_bare(&cfg).unwrap();
        let reparsed = parse_ron_bare(&rendered).unwrap();
        assert_eq!(cfg, reparsed);
        assert_eq!(reparsed.behavior.agent_sidecar_token, "sk-sidecar-1234");
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

    // ============================================================
    // Per-block error-path coverage. Every block's "unknown field"
    // arm and "expected ',' or ')' in <block>" arm were 0-execution.
    // ============================================================

    /// Shared base: just enough of the schema to satisfy required
    /// fields. Append more block text and pass through `parse_ron_bare`.
    fn with_base(extra: &str) -> String {
        format!(
            r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                {extra}
            )
            "#
        )
    }

    fn assert_bare_parse_err(input: &str) -> String {
        match parse_ron_bare(input).unwrap_err() {
            ConfigError::BareParse(msg) => msg,
            other => panic!("expected BareParse, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["a"] ),
                surprise: "no",
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(msg.contains("unknown top-level field"), "got {msg}");
    }

    #[test]
    fn rejects_missing_comma_between_top_level_fields() {
        // No comma between mdns and time blocks.
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" )
                time: ( tz: "UTC", sntp_servers: ["a"] ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(msg.contains("expected ',' or ')'"), "got {msg}");
    }

    #[test]
    fn rejects_unknown_wifi_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US", what: "x" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["a"] ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(msg.contains("unknown wifi field"), "got {msg}");
    }

    #[test]
    fn rejects_unknown_mdns_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h", what: "x" ),
                time: ( tz: "UTC", sntp_servers: ["a"] ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(msg.contains("unknown mdns field"), "got {msg}");
    }

    #[test]
    fn rejects_unknown_time_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["a"], oops: "no" ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(msg.contains("unknown time field"), "got {msg}");
    }

    #[test]
    fn rejects_unknown_auth_field() {
        let s = with_base(r#"auth: ( token: "t", oops: "no" ),"#);
        let msg = assert_bare_parse_err(&s);
        assert!(msg.contains("unknown auth field"), "got {msg}");
    }

    #[test]
    fn rejects_redacted_auth_token() {
        // The bare parser refuses the redacted sentinel — the
        // round-trip path that emits "***" expects the operator to
        // type the actual token back. Pin that protection.
        let redacted = TOKEN_REDACTED;
        let s = format!(
            r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["a"] ),
                auth: ( token: "{redacted}" ),
            )
            "#
        );
        let msg = assert_bare_parse_err(&s);
        assert!(msg.contains("redacted sentinel"), "got {msg}");
    }

    #[test]
    fn rejects_unknown_audio_field() {
        let s = with_base("audio: ( volume_pct: 50, muted: false, mystery: 1 ),");
        let msg = assert_bare_parse_err(&s);
        assert!(msg.contains("unknown audio field"), "got {msg}");
    }

    #[test]
    fn rejects_unknown_tracker_field() {
        let s = with_base("tracker: ( fov_h_deg: 60.0, oops: 1 ),");
        let msg = assert_bare_parse_err(&s);
        assert!(msg.contains("unknown tracker field"), "got {msg}");
    }

    #[test]
    fn rejects_unknown_esp_now_field() {
        let s = with_base("esp_now: ( enabled: false, oops: 1 ),");
        let msg = assert_bare_parse_err(&s);
        assert!(msg.contains("unknown esp_now field"), "got {msg}");
    }

    #[test]
    fn rejects_unknown_behavior_field() {
        let s = with_base("behavior: ( soliloquy_enabled: false, oops: 1 ),");
        let msg = assert_bare_parse_err(&s);
        assert!(msg.contains("unknown behavior field"), "got {msg}");
    }

    // ============================================================
    // Optional<u8> + literal parser error paths.
    // ============================================================

    #[test]
    fn parses_esp_now_channel_some() {
        let s = with_base(
            r#"esp_now: ( enabled: true, pmk_hex: "00000000000000000000000000000000", peer_mac: "00:00:00:00:00:00", lmk_hex: "00000000000000000000000000000000", channel: Some(11), tx_rate_hz: 10 ),"#,
        );
        let cfg = parse_ron_bare(&s).unwrap();
        assert_eq!(cfg.esp_now.channel, Some(11));
    }

    #[test]
    fn parses_esp_now_channel_none() {
        let s = with_base(
            r#"esp_now: ( enabled: true, pmk_hex: "00000000000000000000000000000000", peer_mac: "00:00:00:00:00:00", lmk_hex: "00000000000000000000000000000000", channel: None, tx_rate_hz: 10 ),"#,
        );
        let cfg = parse_ron_bare(&s).unwrap();
        assert_eq!(cfg.esp_now.channel, None);
    }

    #[test]
    fn rejects_optional_u8_other_token() {
        // Anything that's not `Some(...)` or `None` errors clearly.
        let s = with_base(
            r#"esp_now: ( enabled: true, pmk_hex: "00000000000000000000000000000000", peer_mac: "00:00:00:00:00:00", lmk_hex: "00000000000000000000000000000000", channel: Maybe(7), tx_rate_hz: 10 ),"#,
        );
        let msg = assert_bare_parse_err(&s);
        assert!(msg.contains("expected Some(...) or None"), "got {msg}");
    }

    #[test]
    fn rejects_esp_now_channel_value_out_of_u8_range() {
        let s = with_base(
            r#"esp_now: ( enabled: true, pmk_hex: "00000000000000000000000000000000", peer_mac: "00:00:00:00:00:00", lmk_hex: "00000000000000000000000000000000", channel: Some(999), tx_rate_hz: 10 ),"#,
        );
        let msg = assert_bare_parse_err(&s);
        assert!(msg.contains("u8 literal out of range"), "got {msg}");
    }

    #[test]
    fn rejects_u32_non_digit() {
        let s = with_base("behavior: ( auto_torque_release_ms: notanumber ),");
        let msg = assert_bare_parse_err(&s);
        assert!(msg.contains("expected unsigned integer"), "got {msg}");
    }

    #[test]
    fn rejects_i8_out_of_range() {
        let s = with_base("behavior: ( wake_word_threshold: 999 ),");
        let msg = assert_bare_parse_err(&s);
        assert!(msg.contains("i8 literal out of range"), "got {msg}");
    }

    #[test]
    fn rejects_i8_non_digit() {
        let s = with_base("behavior: ( wake_word_threshold: oops ),");
        let msg = assert_bare_parse_err(&s);
        assert!(msg.contains("expected signed integer"), "got {msg}");
    }

    #[test]
    fn rejects_non_bool_for_bool_field() {
        let s = with_base("behavior: ( soliloquy_enabled: yes ),");
        let msg = assert_bare_parse_err(&s);
        assert!(msg.contains("expected boolean literal"), "got {msg}");
    }

    #[test]
    fn rejects_string_list_missing_separator() {
        // sntp_servers has two elements with no comma between.
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["a" "b"] ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(msg.contains("expected ',' or ']' in list"), "got {msg}");
    }

    #[test]
    fn rejects_unterminated_string_literal() {
        // String literal that never closes.
        let s = "( wifi: ( ssid: \"unterminated";
        let msg = assert_bare_parse_err(s);
        assert!(msg.contains("unterminated string literal"), "got {msg}");
    }

    #[test]
    fn rejects_dangling_backslash() {
        let s = "( wifi: ( ssid: \"oops\\";
        let msg = assert_bare_parse_err(s);
        assert!(msg.contains("dangling backslash"), "got {msg}");
    }

    #[test]
    fn rejects_unsupported_escape() {
        let s = r#"
            (
                wifi: ( ssid: "bad\xescape", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["a"] ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(msg.contains("unsupported escape"), "got {msg}");
    }

    #[test]
    fn supports_newline_and_tab_escapes() {
        // \n and \t are the supported escapes; pin that they decode.
        let s = r#"
            (
                wifi: ( ssid: "line\none\ttab", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["a"] ),
            )
        "#;
        let cfg = parse_ron_bare(s).unwrap();
        assert_eq!(cfg.wifi.ssid, "line\none\ttab");
    }

    #[test]
    fn rejects_identifier_starting_with_digit() {
        let s = "( 1bad: 0 )";
        let msg = assert_bare_parse_err(s);
        assert!(msg.contains("expected identifier"), "got {msg}");
    }

    #[test]
    fn rejects_eof_when_identifier_expected() {
        let s = "(";
        let msg = assert_bare_parse_err(s);
        assert!(
            msg.contains("expected identifier, got EOF") || msg.contains("expected identifier"),
            "got {msg}"
        );
    }

    #[test]
    fn rejects_missing_open_paren() {
        // Top-level isn't a tuple struct.
        let s = "wifi: ()";
        let msg = assert_bare_parse_err(s);
        assert!(msg.contains("expected char"), "got {msg}");
    }

    // ============================================================
    // Renderer: esp_now Some(channel) emission path.
    // ============================================================

    #[test]
    fn renders_esp_now_some_channel() {
        // The Some-channel renderer arm was uncovered. Build a config
        // with channel = Some(7) and confirm the rendered string
        // includes `channel: Some(7),`.
        let mut cfg = parse_ron_bare(FIXTURE).unwrap();
        cfg.esp_now.channel = Some(7);
        let rendered = render_ron_bare(&cfg).unwrap();
        assert!(
            rendered.contains("channel: Some(7),"),
            "rendered = {rendered}"
        );
    }

    #[test]
    fn renders_escapes_in_string_literals() {
        // push_string_literal's backslash + quote arms — neither was
        // exercised by FIXTURE values. Round-trip a config whose
        // ssid contains both kinds of escape.
        let mut cfg = parse_ron_bare(FIXTURE).unwrap();
        cfg.wifi.ssid = "back\\slash and \"quote\"".to_string();
        let rendered = render_ron_bare(&cfg).unwrap();
        assert!(rendered.contains(r#""back\\slash and \"quote\"""#));
        let reparsed = parse_ron_bare(&rendered).unwrap();
        assert_eq!(reparsed.wifi.ssid, "back\\slash and \"quote\"");
    }

    // ============================================================
    // Duplicate-key rejection — schema parity with bare_json.rs.
    // A hand-edited RON with shadowed fields would otherwise last-wins
    // silently; the canonical hazard is `wifi: ( psk: "real", psk: "x" )`
    // where the renderer-side redaction string `"***"` ends up
    // overwriting a real PSK on a round-trip through `GET /settings`.
    // ============================================================

    #[test]
    fn rejects_duplicate_top_level_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                wifi: ( ssid: "n2", psk: "p2", country: "JP" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(
            msg.contains("duplicate top-level field") && msg.contains("wifi"),
            "got {msg}"
        );
    }

    #[test]
    fn rejects_duplicate_wifi_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "real", psk: "x", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(
            msg.contains("duplicate wifi field") && msg.contains("psk"),
            "got {msg}"
        );
    }

    #[test]
    fn rejects_duplicate_mdns_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "a", hostname: "b" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(
            msg.contains("duplicate mdns field") && msg.contains("hostname"),
            "got {msg}"
        );
    }

    #[test]
    fn rejects_duplicate_time_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", tz: "JST", sntp_servers: ["pool.ntp.org"] ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(
            msg.contains("duplicate time field") && msg.contains("tz"),
            "got {msg}"
        );
    }

    #[test]
    fn rejects_duplicate_auth_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                auth: ( token: "real-token-aaaaaaaaaaaaaaaa", token: "shadow-aaaaaaaaaaaaaaaaa" ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(
            msg.contains("duplicate auth field") && msg.contains("token"),
            "got {msg}"
        );
    }

    #[test]
    fn rejects_duplicate_audio_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                audio: ( volume_pct: 50, volume_pct: 60, muted: false ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(
            msg.contains("duplicate audio field") && msg.contains("volume_pct"),
            "got {msg}"
        );
    }

    #[test]
    fn rejects_duplicate_tracker_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                tracker: ( fov_h_deg: 60, fov_h_deg: 50 ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(
            msg.contains("duplicate tracker field") && msg.contains("fov_h_deg"),
            "got {msg}"
        );
    }

    #[test]
    fn rejects_duplicate_esp_now_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                esp_now: ( enabled: true, enabled: false ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(
            msg.contains("duplicate esp_now field") && msg.contains("enabled"),
            "got {msg}"
        );
    }

    // ============================================================
    // Disk-load redaction-sentinel rejection (validate_for_disk).
    // The "***" sentinel exists as a "preserve current value" marker
    // on the HTTP `PUT /settings` merge path. On disk it means
    // operator copy-paste from a redacted GET; rejecting fast beats
    // a Wi-Fi auth-fail buried deep in the boot log.
    // ============================================================

    fn assert_redaction_sentinel(input: &str) -> &'static str {
        match parse_ron_bare(input).unwrap_err() {
            ConfigError::RedactionSentinelOnDisk(field) => field,
            other => panic!("expected RedactionSentinelOnDisk, got {other:?}"),
        }
    }

    #[test]
    fn rejects_redacted_psk_on_disk() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "***", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
            )
        "#;
        assert_eq!(assert_redaction_sentinel(s), "wifi.psk");
    }

    #[test]
    fn rejects_redacted_pmk_hex_on_disk() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                esp_now: ( enabled: false, pmk_hex: "***" ),
            )
        "#;
        assert_eq!(assert_redaction_sentinel(s), "esp_now.pmk_hex");
    }

    #[test]
    fn rejects_redacted_lmk_hex_on_disk() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                esp_now: ( enabled: false, lmk_hex: "***" ),
            )
        "#;
        assert_eq!(assert_redaction_sentinel(s), "esp_now.lmk_hex");
    }

    #[test]
    fn rejects_redacted_agent_sidecar_token_on_disk() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                behavior: ( agent_sidecar_token: "***" ),
            )
        "#;
        assert_eq!(assert_redaction_sentinel(s), "behavior.agent_sidecar_token");
    }

    #[test]
    fn rejects_duplicate_behavior_field() {
        let s = r#"
            (
                wifi: ( ssid: "n", psk: "p", country: "US" ),
                mdns: ( hostname: "h" ),
                time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
                behavior: ( wake_word_enabled: true, wake_word_enabled: false ),
            )
        "#;
        let msg = assert_bare_parse_err(s);
        assert!(
            msg.contains("duplicate behavior field") && msg.contains("wake_word_enabled"),
            "got {msg}"
        );
    }
}
