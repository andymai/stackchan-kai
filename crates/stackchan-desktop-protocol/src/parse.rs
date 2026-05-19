//! Bytes → [`Inbound`].
//!
//! Strategy: parse each line into a small generic JSON value tree
//! (alloc'd once), then dispatch on the top-level keys. This is
//! simpler than streaming dispatch because the desktop protocol's
//! discriminator (`cmd` vs `evt` vs `time` vs none) is
//! content-based rather than position-based; a few hundred extra
//! bytes of allocation per line is negligible given the
//! at-most-10-Hz heartbeat rate and the firmware's PSRAM heap.
//!
//! Unknown top-level keys parse fine but are skipped by the typed
//! extractors — desktop versions can add fields without bricking
//! firmware that pre-dates them.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write as _;
use core::str;

use crate::error::ProtoError;
use crate::types::{
    Ack, BatteryStatus, Cmd, ContentBlock, Decision, Inbound, Outbound, Prompt, Snapshot,
    StatusData, SysStatus, Turn, UserStats,
};

/// Parse one newline-delimited desktop message.
///
/// `line` must be a single JSON object (no trailing newline; the
/// framer strips it). Empty inputs return [`ProtoError::MalformedJson`].
///
/// # Errors
///
/// - [`ProtoError::InvalidUtf8`] if `line` isn't UTF-8.
/// - [`ProtoError::MalformedJson`] for any structural fault.
/// - [`ProtoError::MissingField`] when a required field for the
///   inferred kind isn't present.
/// - [`ProtoError::BadValue`] when a field is the wrong shape.
/// - [`ProtoError::UnknownKind`] when the message has a `cmd` field
///   we don't recognize (forward compat: just ignore at the consumer).
/// - [`ProtoError::InvalidBase64`] for malformed `chunk` payloads.
pub fn parse_inbound(line: &[u8]) -> Result<Inbound, ProtoError> {
    let text = str::from_utf8(line).map_err(|_| ProtoError::InvalidUtf8)?;
    let value = JsonParser::new(text).parse_value()?;
    let Value::Object(fields) = value else {
        return Err(ProtoError::MalformedJson("top-level must be an object"));
    };
    dispatch(&fields)
}

/// Dispatch a parsed top-level object to the right [`Inbound`]
/// variant based on its discriminator key.
fn dispatch(fields: &[(String, Value)]) -> Result<Inbound, ProtoError> {
    if let Some(cmd) = lookup(fields, "cmd") {
        let cmd_name = expect_str(cmd, "cmd")?;
        return parse_cmd(cmd_name, fields).map(Inbound::Cmd);
    }
    if let Some(evt) = lookup(fields, "evt") {
        let evt_name = expect_str(evt, "evt")?;
        if evt_name == "turn" {
            return parse_turn(fields).map(Inbound::Turn);
        }
        return Err(ProtoError::UnknownKind(evt_name.to_string()));
    }
    if let Some(time) = lookup(fields, "time") {
        return parse_time_sync(time);
    }
    parse_snapshot(fields).map(Inbound::Snapshot)
}

/// Extract a [`Snapshot`] from the top-level object's fields. All
/// fields are optional — a keepalive `{}` parses to
/// [`Snapshot::default`].
fn parse_snapshot(fields: &[(String, Value)]) -> Result<Snapshot, ProtoError> {
    let mut snap = Snapshot::default();
    for (k, v) in fields {
        match k.as_str() {
            "total" => snap.total = expect_u32(v, "total")?,
            "running" => snap.running = expect_u32(v, "running")?,
            "waiting" => snap.waiting = expect_u32(v, "waiting")?,
            "msg" => snap.msg = expect_str_or_null(v, "msg")?.unwrap_or_default(),
            "entries" => snap.entries = expect_string_array(v, "entries")?,
            "tokens" => snap.tokens = expect_u64(v, "tokens")?,
            "tokens_today" => snap.tokens_today = expect_u64(v, "tokens_today")?,
            "prompt" => snap.prompt = parse_prompt(v)?,
            _ => {} // forward compat
        }
    }
    Ok(snap)
}

/// Extract a [`Prompt`] from a `prompt` field's value. The
/// desktop encodes "no prompt" as either field-absent (handled by
/// the caller) or `null`.
fn parse_prompt(v: &Value) -> Result<Option<Prompt>, ProtoError> {
    match v {
        Value::Null => Ok(None),
        Value::Object(fields) => {
            let id = field_str(fields, "id", "prompt.id")?;
            let tool = field_str_opt(fields, "tool").unwrap_or_default();
            let hint = field_str_opt(fields, "hint").unwrap_or_default();
            if id.is_empty() {
                return Err(ProtoError::MissingField("prompt.id"));
            }
            Ok(Some(Prompt { id, tool, hint }))
        }
        _ => Err(ProtoError::BadValue {
            field: "prompt",
            reason: "expected object or null",
        }),
    }
}

/// Extract a [`Turn`] from a `{"evt":"turn",...}` envelope.
/// `role` defaults to `"assistant"` when absent (the spec's only
/// documented value today).
fn parse_turn(fields: &[(String, Value)]) -> Result<Turn, ProtoError> {
    let role = field_str_opt(fields, "role").unwrap_or_else(|| "assistant".into());
    let content_val = lookup(fields, "content").ok_or(ProtoError::MissingField("content"))?;
    let Value::Array(blocks_raw) = content_val else {
        return Err(ProtoError::BadValue {
            field: "content",
            reason: "expected array",
        });
    };
    let mut content = Vec::with_capacity(blocks_raw.len());
    for block in blocks_raw {
        let Value::Object(block_fields) = block else {
            return Err(ProtoError::BadValue {
                field: "content[]",
                reason: "expected object",
            });
        };
        let kind = field_str(block_fields, "type", "content[].type")?;
        let text = field_str_opt(block_fields, "text");
        let raw_json = render_value(block);
        content.push(ContentBlock {
            kind,
            text,
            raw_json,
        });
    }
    Ok(Turn { role, content })
}

/// Extract an [`Inbound::TimeSync`] from a `time` field's value.
/// The desktop encodes time as a two-element array
/// `[epoch_secs, tz_offset_secs]`.
fn parse_time_sync(v: &Value) -> Result<Inbound, ProtoError> {
    let Value::Array(items) = v else {
        return Err(ProtoError::BadValue {
            field: "time",
            reason: "expected [epoch, tz_offset] array",
        });
    };
    if items.len() != 2 {
        return Err(ProtoError::BadValue {
            field: "time",
            reason: "expected two-element array",
        });
    }
    let epoch_secs = expect_i64(&items[0], "time[0]")?;
    let tz_offset_secs =
        i32::try_from(expect_i64(&items[1], "time[1]")?).map_err(|_| ProtoError::BadValue {
            field: "time[1]",
            reason: "tz offset out of i32 range",
        })?;
    Ok(Inbound::TimeSync {
        epoch_secs,
        tz_offset_secs,
    })
}

/// Dispatch a `cmd`-tagged inbound to the right [`Cmd`] variant.
/// Unknown command names surface as [`ProtoError::UnknownKind`] so
/// the caller can log + drop without crashing.
fn parse_cmd(name: &str, fields: &[(String, Value)]) -> Result<Cmd, ProtoError> {
    match name {
        "owner" => Ok(Cmd::Owner {
            name: field_str(fields, "name", "owner.name")?,
        }),
        "status" => Ok(Cmd::Status),
        "name" => Ok(Cmd::SetName {
            name: field_str(fields, "name", "name.name")?,
        }),
        "unpair" => Ok(Cmd::Unpair),
        "char_begin" => Ok(Cmd::CharBegin {
            name: field_str(fields, "name", "char_begin.name")?,
            total: field_u32(fields, "total", "char_begin.total")?,
        }),
        "file" => Ok(Cmd::File {
            path: field_str(fields, "path", "file.path")?,
            size: field_u32(fields, "size", "file.size")?,
        }),
        "chunk" => {
            let b64 = field_str(fields, "d", "chunk.d")?;
            let data = base64_decode(&b64)?;
            Ok(Cmd::Chunk { data })
        }
        "file_end" => Ok(Cmd::FileEnd),
        "char_end" => Ok(Cmd::CharEnd),
        other => Err(ProtoError::UnknownKind(other.to_string())),
    }
}

// ============================================================
// Outbound parsing — only meaningful for tests / loopback fixtures,
// but it's symmetric and cheap so we expose it.
// ============================================================

/// Parse an outbound JSON line (the device-side wire format).
///
/// Symmetric with [`crate::render_outbound`]; provided so host
/// tests can round-trip without re-implementing the parser.
///
/// # Errors
///
/// Same shape as [`parse_inbound`].
pub fn parse_outbound(line: &[u8]) -> Result<Outbound, ProtoError> {
    let text = str::from_utf8(line).map_err(|_| ProtoError::InvalidUtf8)?;
    let value = JsonParser::new(text).parse_value()?;
    let Value::Object(fields) = value else {
        return Err(ProtoError::MalformedJson("top-level must be an object"));
    };
    if let Some(ack_val) = lookup(&fields, "ack") {
        let ack_name = expect_str(ack_val, "ack")?.to_string();
        if ack_name == "status" {
            let data_val = lookup(&fields, "data").ok_or(ProtoError::MissingField("data"))?;
            let Value::Object(data_fields) = data_val else {
                return Err(ProtoError::BadValue {
                    field: "data",
                    reason: "expected object",
                });
            };
            return Ok(Outbound::StatusAck(parse_status_data(data_fields)?));
        }
        let ok = field_bool(&fields, "ok", "ok")?;
        let n = field_u32_opt(&fields, "n").unwrap_or(0);
        let error = field_str_opt(&fields, "error");
        return Ok(Outbound::Ack(Ack {
            cmd: ack_name,
            ok,
            n,
            error,
        }));
    }
    if let Some(cmd_val) = lookup(&fields, "cmd") {
        let cmd_name = expect_str(cmd_val, "cmd")?;
        if cmd_name == "permission" {
            let id = field_str(&fields, "id", "id")?;
            let decision_str = field_str(&fields, "decision", "decision")?;
            let decision = match decision_str.as_str() {
                "once" => Decision::Once,
                "deny" => Decision::Deny,
                _ => {
                    return Err(ProtoError::BadValue {
                        field: "decision",
                        reason: "expected `once` or `deny`",
                    });
                }
            };
            return Ok(Outbound::Permission { id, decision });
        }
        return Err(ProtoError::UnknownKind(cmd_name.to_string()));
    }
    Err(ProtoError::MalformedJson("outbound needs `ack` or `cmd`"))
}

/// Extract a [`StatusData`] from a status ack's `data` payload.
/// All sub-objects are optional — the spec says "you can omit
/// fields you don't have."
fn parse_status_data(fields: &[(String, Value)]) -> Result<StatusData, ProtoError> {
    let mut data = StatusData::default();
    for (k, v) in fields {
        match k.as_str() {
            "name" => data.name = expect_str(v, "name")?.to_string(),
            "sec" => data.sec = expect_bool(v, "sec")?,
            "bat" => data.battery = Some(parse_battery(v)?),
            "sys" => data.sys = Some(parse_sys(v)?),
            "stats" => data.stats = Some(parse_user_stats(v)?),
            _ => {}
        }
    }
    Ok(data)
}

/// Extract a [`BatteryStatus`] from a status ack's `bat` sub-object.
fn parse_battery(v: &Value) -> Result<BatteryStatus, ProtoError> {
    let Value::Object(fields) = v else {
        return Err(ProtoError::BadValue {
            field: "bat",
            reason: "expected object",
        });
    };
    Ok(BatteryStatus {
        pct: u8::try_from(field_u32(fields, "pct", "bat.pct")?).map_err(|_| {
            ProtoError::BadValue {
                field: "bat.pct",
                reason: "out of u8 range",
            }
        })?,
        mv: u16::try_from(field_u32(fields, "mV", "bat.mV")?).map_err(|_| {
            ProtoError::BadValue {
                field: "bat.mV",
                reason: "out of u16 range",
            }
        })?,
        ma: i16::try_from(field_i64(fields, "mA", "bat.mA")?).map_err(|_| {
            ProtoError::BadValue {
                field: "bat.mA",
                reason: "out of i16 range",
            }
        })?,
        usb: field_bool(fields, "usb", "bat.usb")?,
    })
}

/// Extract a [`SysStatus`] from a status ack's `sys` sub-object.
fn parse_sys(v: &Value) -> Result<SysStatus, ProtoError> {
    let Value::Object(fields) = v else {
        return Err(ProtoError::BadValue {
            field: "sys",
            reason: "expected object",
        });
    };
    Ok(SysStatus {
        uptime_secs: field_u32(fields, "up", "sys.up")?,
        heap_free_bytes: field_u32(fields, "heap", "sys.heap")?,
    })
}

/// Extract a [`UserStats`] from a status ack's `stats` sub-object.
/// All counters default to 0 when absent so a device that doesn't
/// track a particular counter can simply omit it.
fn parse_user_stats(v: &Value) -> Result<UserStats, ProtoError> {
    let Value::Object(fields) = v else {
        return Err(ProtoError::BadValue {
            field: "stats",
            reason: "expected object",
        });
    };
    Ok(UserStats {
        approvals: field_u32_opt(fields, "appr").unwrap_or(0),
        denies: field_u32_opt(fields, "deny").unwrap_or(0),
        velocity: field_u32_opt(fields, "vel").unwrap_or(0),
        naps: field_u32_opt(fields, "nap").unwrap_or(0),
        level: field_u32_opt(fields, "lvl").unwrap_or(0),
    })
}

// ============================================================
// Field extractors
// ============================================================

/// Look up a key in a flat object-field list. `O(n)` linear scan;
/// our objects have at most ~10 keys so a `HashMap` would just
/// add allocation overhead.
fn lookup<'a>(fields: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Pull a required string field and clone it. `error_path` shows
/// up in the error message so missing-field errors point at the
/// nested path (e.g. `prompt.id`) rather than just the leaf key.
fn field_str(
    fields: &[(String, Value)],
    key: &str,
    error_path: &'static str,
) -> Result<String, ProtoError> {
    let v = lookup(fields, key).ok_or(ProtoError::MissingField(error_path))?;
    Ok(expect_str(v, error_path)?.to_string())
}

/// Pull an optional string field. Missing or non-string returns
/// `None`; the caller decides whether that's an error.
fn field_str_opt(fields: &[(String, Value)], key: &str) -> Option<String> {
    match lookup(fields, key)? {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// Pull a required u32 field.
fn field_u32(
    fields: &[(String, Value)],
    key: &str,
    error_path: &'static str,
) -> Result<u32, ProtoError> {
    let v = lookup(fields, key).ok_or(ProtoError::MissingField(error_path))?;
    expect_u32(v, error_path)
}

/// Pull an optional u32 field. Missing, non-integer, or out-of-range
/// returns `None`. Used for status-ack counter fields that may be
/// individually omitted.
fn field_u32_opt(fields: &[(String, Value)], key: &str) -> Option<u32> {
    match lookup(fields, key)? {
        Value::Int(n) => u32::try_from(*n).ok(),
        _ => None,
    }
}

/// Pull a required i64 field.
fn field_i64(
    fields: &[(String, Value)],
    key: &str,
    error_path: &'static str,
) -> Result<i64, ProtoError> {
    let v = lookup(fields, key).ok_or(ProtoError::MissingField(error_path))?;
    expect_i64(v, error_path)
}

/// Pull a required bool field.
fn field_bool(
    fields: &[(String, Value)],
    key: &str,
    error_path: &'static str,
) -> Result<bool, ProtoError> {
    let v = lookup(fields, key).ok_or(ProtoError::MissingField(error_path))?;
    expect_bool(v, error_path)
}

/// Coerce a [`Value`] to `&str` or fail with [`ProtoError::BadValue`].
const fn expect_str<'a>(v: &'a Value, field: &'static str) -> Result<&'a str, ProtoError> {
    if let Value::Str(s) = v {
        Ok(s.as_str())
    } else {
        Err(ProtoError::BadValue {
            field,
            reason: "expected string",
        })
    }
}

/// Coerce a [`Value`] to `Option<String>`. Distinguishes
/// JSON `null` (returns `Ok(None)`) from a wrong-type value (returns
/// [`ProtoError::BadValue`]).
fn expect_str_or_null(v: &Value, field: &'static str) -> Result<Option<String>, ProtoError> {
    match v {
        Value::Null => Ok(None),
        Value::Str(s) => Ok(Some(s.clone())),
        _ => Err(ProtoError::BadValue {
            field,
            reason: "expected string or null",
        }),
    }
}

/// Coerce a [`Value`] to `u32` (negatives and `> u32::MAX` rejected).
fn expect_u32(v: &Value, field: &'static str) -> Result<u32, ProtoError> {
    let n = expect_i64(v, field)?;
    u32::try_from(n).map_err(|_| ProtoError::BadValue {
        field,
        reason: "out of u32 range",
    })
}

/// Coerce a [`Value`] to `u64` (negatives rejected).
fn expect_u64(v: &Value, field: &'static str) -> Result<u64, ProtoError> {
    let n = expect_i64(v, field)?;
    u64::try_from(n).map_err(|_| ProtoError::BadValue {
        field,
        reason: "out of u64 range",
    })
}

/// Coerce a [`Value`] to `i64`.
const fn expect_i64(v: &Value, field: &'static str) -> Result<i64, ProtoError> {
    if let Value::Int(n) = v {
        Ok(*n)
    } else {
        Err(ProtoError::BadValue {
            field,
            reason: "expected integer",
        })
    }
}

/// Coerce a [`Value`] to `bool`.
const fn expect_bool(v: &Value, field: &'static str) -> Result<bool, ProtoError> {
    if let Value::Bool(b) = v {
        Ok(*b)
    } else {
        Err(ProtoError::BadValue {
            field,
            reason: "expected bool",
        })
    }
}

/// Coerce a [`Value`] to `Vec<String>`, rejecting non-string array
/// elements.
fn expect_string_array(v: &Value, field: &'static str) -> Result<Vec<String>, ProtoError> {
    let Value::Array(items) = v else {
        return Err(ProtoError::BadValue {
            field,
            reason: "expected array",
        });
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(expect_str(item, field)?.to_string());
    }
    Ok(out)
}

// ============================================================
// JSON tree + parser
// ============================================================

/// Generic JSON value tree. Owned, allocates per node — fine at
/// the at-most-10-Hz heartbeat rate the protocol runs at, and lets
/// the dispatcher look at any top-level key without re-tokenising.
/// Object fields are a `Vec` (not a `HashMap`) so insertion order
/// survives [`render_value`] for raw-block forwarding.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    /// JSON `null`.
    Null,
    /// JSON `true` / `false`.
    Bool(bool),
    /// JSON integer literal. Floats and exponents are rejected by
    /// the parser — the protocol uses ints only.
    Int(i64),
    /// JSON string literal (escapes already decoded).
    Str(String),
    /// JSON array.
    Array(Vec<Self>),
    /// JSON object, key insertion order preserved.
    Object(Vec<(String, Self)>),
}

/// Streaming JSON parser. Owns no scratch buffer — `input` slides
/// as tokens are consumed.
struct JsonParser<'a> {
    /// Unconsumed remainder of the input string.
    input: &'a str,
}

impl<'a> JsonParser<'a> {
    /// Construct a parser over `input`.
    const fn new(input: &'a str) -> Self {
        Self { input }
    }

    /// Parse a single JSON value (object, array, string, number,
    /// bool, or null).
    fn parse_value(&mut self) -> Result<Value, ProtoError> {
        self.skip_ws();
        let Some(ch) = self.peek() else {
            return Err(ProtoError::MalformedJson("unexpected end of input"));
        };
        let v = match ch {
            '{' => self.parse_object()?,
            '[' => self.parse_array()?,
            '"' => Value::Str(self.parse_string()?),
            't' | 'f' => self.parse_bool()?,
            'n' => self.parse_null()?,
            '-' | '0'..='9' => self.parse_number()?,
            _ => return Err(ProtoError::MalformedJson("unexpected token")),
        };
        Ok(v)
    }

    /// Parse `{ "key": value, ... }` into a [`Value::Object`].
    fn parse_object(&mut self) -> Result<Value, ProtoError> {
        self.expect('{')?;
        let mut fields = Vec::new();
        loop {
            self.skip_ws();
            if self.try_consume('}') {
                return Ok(Value::Object(fields));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(':')?;
            let value = self.parse_value()?;
            fields.push((key, value));
            self.skip_ws();
            if self.try_consume(',') {
                continue;
            }
            self.skip_ws();
            self.expect('}')?;
            return Ok(Value::Object(fields));
        }
    }

    /// Parse `[ value, value, ... ]` into a [`Value::Array`].
    fn parse_array(&mut self) -> Result<Value, ProtoError> {
        self.expect('[')?;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.try_consume(']') {
                return Ok(Value::Array(items));
            }
            items.push(self.parse_value()?);
            self.skip_ws();
            if self.try_consume(',') {
                continue;
            }
            self.skip_ws();
            self.expect(']')?;
            return Ok(Value::Array(items));
        }
    }

    /// Parse `"..."` into a [`String`] with all JSON escapes
    /// (`\\`, `\"`, `\/`, `\n`, `\t`, `\r`, `\b`, `\f`, `\uXXXX`,
    /// surrogate pairs) decoded.
    fn parse_string(&mut self) -> Result<String, ProtoError> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            let Some(ch) = self.peek() else {
                return Err(ProtoError::MalformedJson("unterminated string"));
            };
            if ch == '"' {
                self.advance(1);
                return Ok(out);
            }
            if ch == '\\' {
                self.advance(1);
                let Some(esc) = self.peek() else {
                    return Err(ProtoError::MalformedJson("dangling backslash"));
                };
                match esc {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{0008}'),
                    'f' => out.push('\u{000c}'),
                    'u' => {
                        self.advance(1);
                        let cp = self.parse_unicode_escape()?;
                        out.push(cp);
                        continue;
                    }
                    _ => return Err(ProtoError::MalformedJson("unsupported escape")),
                }
                self.advance(esc.len_utf8());
            } else {
                out.push(ch);
                self.advance(ch.len_utf8());
            }
        }
    }

    /// Decode a `\uXXXX` escape (already past the `\u`). Handles
    /// UTF-16 surrogate pairs for code points outside the BMP.
    fn parse_unicode_escape(&mut self) -> Result<char, ProtoError> {
        if self.input.len() < 4 {
            return Err(ProtoError::MalformedJson("short \\u escape"));
        }
        let (hex, rest) = self.input.split_at(4);
        let cp =
            u32::from_str_radix(hex, 16).map_err(|_| ProtoError::MalformedJson("bad \\u hex"))?;
        self.input = rest;
        if (0xD800..=0xDBFF).contains(&cp) {
            if !self.input.starts_with("\\u") {
                return Err(ProtoError::MalformedJson("lone high surrogate"));
            }
            self.advance(2);
            if self.input.len() < 4 {
                return Err(ProtoError::MalformedJson("short \\u escape"));
            }
            let (low_hex, rest) = self.input.split_at(4);
            let low = u32::from_str_radix(low_hex, 16)
                .map_err(|_| ProtoError::MalformedJson("bad low surrogate hex"))?;
            self.input = rest;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return Err(ProtoError::MalformedJson("bad low surrogate"));
            }
            let combined = 0x1_0000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
            char::from_u32(combined).ok_or(ProtoError::MalformedJson("bad surrogate pair"))
        } else if (0xDC00..=0xDFFF).contains(&cp) {
            Err(ProtoError::MalformedJson("lone low surrogate"))
        } else {
            char::from_u32(cp).ok_or(ProtoError::MalformedJson("bad code point"))
        }
    }

    /// Parse a `true` / `false` literal into [`Value::Bool`].
    fn parse_bool(&mut self) -> Result<Value, ProtoError> {
        if self.input.starts_with("true") {
            self.advance(4);
            Ok(Value::Bool(true))
        } else if self.input.starts_with("false") {
            self.advance(5);
            Ok(Value::Bool(false))
        } else {
            Err(ProtoError::MalformedJson("expected true|false"))
        }
    }

    /// Parse a `null` literal into [`Value::Null`].
    fn parse_null(&mut self) -> Result<Value, ProtoError> {
        if self.input.starts_with("null") {
            self.advance(4);
            Ok(Value::Null)
        } else {
            Err(ProtoError::MalformedJson("expected null"))
        }
    }

    /// Parse an optionally-signed integer literal into [`Value::Int`].
    /// Floats and exponents are rejected — the protocol uses ints
    /// only; accepting them would silently truncate operator-visible
    /// numbers if a future desktop version sent them.
    fn parse_number(&mut self) -> Result<Value, ProtoError> {
        let bytes = self.input.as_bytes();
        let mut end = 0;
        if bytes.first() == Some(&b'-') {
            end += 1;
        }
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        // Reject decimal points and exponents — protocol uses ints only.
        if end < bytes.len() && (bytes[end] == b'.' || bytes[end] == b'e' || bytes[end] == b'E') {
            return Err(ProtoError::MalformedJson("non-integer number"));
        }
        if end == 0 || (end == 1 && bytes[0] == b'-') {
            return Err(ProtoError::MalformedJson("empty number"));
        }
        let (digits, rest) = self.input.split_at(end);
        let parsed: i64 = digits
            .parse()
            .map_err(|_| ProtoError::MalformedJson("integer out of range"))?;
        self.input = rest;
        Ok(Value::Int(parsed))
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

    /// Peek the next char without advancing.
    fn peek(&self) -> Option<char> {
        self.input.chars().next()
    }

    /// Consume the next char if it equals `c`; return whether it did.
    fn try_consume(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.advance(c.len_utf8());
            true
        } else {
            false
        }
    }

    /// Require the next char to equal `c`; advance past it on success.
    fn expect(&mut self, c: char) -> Result<(), ProtoError> {
        if self.try_consume(c) {
            Ok(())
        } else {
            Err(ProtoError::MalformedJson("missing required char"))
        }
    }

    /// Drop the first `n` bytes from [`Self::input`]. Callers must
    /// guarantee `n` lies on a UTF-8 boundary (all call sites do —
    /// they advance after a peek + length check).
    const fn advance(&mut self, n: usize) {
        self.input = self.input.split_at(n).1;
    }
}

// ============================================================
// Value → JSON string (used to preserve raw turn content blocks).
// ============================================================

/// Re-render a [`Value`] tree back to compact JSON. Preserves
/// object key insertion order so [`ContentBlock::raw_json`] tracks
/// the wire order of fields, but whitespace and number formatting
/// may differ from the original input (JSON is whitespace-agnostic
/// and integers always render in canonical form). Output is
/// suitable for re-forwarding but not for byte-identical round-trip.
fn render_value(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v);
    out
}

/// Append the JSON serialization of `v` to `out`.
fn write_value(out: &mut String, v: &Value) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Int(n) => {
            // write! to a String can't fail.
            let _ = write!(out, "{n}");
        }
        Value::Str(s) => write_string(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Value::Object(fields) => {
            out.push('{');
            for (i, (k, val)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, k);
                out.push(':');
                write_value(out, val);
            }
            out.push('}');
        }
    }
}

/// Append a JSON string literal (with required escapes) to `out`.
fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ============================================================
// Base64 decoder (RFC 4648 standard alphabet, padding optional).
// ============================================================

/// Decode a base64 string (RFC 4648 standard alphabet) into bytes.
/// Whitespace inside the input is ignored; non-alphabet bytes
/// (including non-padding `=` after data) return
/// [`ProtoError::InvalidBase64`]. Pulled inline rather than via a
/// crate dep because the workspace's `base64` versions are already
/// in conflict and the decoder is ~40 lines.
fn base64_decode(input: &str) -> Result<Vec<u8>, ProtoError> {
    const INVALID: u8 = 0xFF;
    const PAD: u8 = 0xFE;
    let mut table = [INVALID; 256];
    for (i, b) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        .iter()
        .enumerate()
    {
        // SAFETY-of-correctness: cast is exact; alphabet is 64 chars.
        #[allow(clippy::cast_possible_truncation)]
        let idx = i as u8;
        table[*b as usize] = idx;
    }
    table[b'=' as usize] = PAD;

    let cleaned: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(cleaned.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    let mut padding = 0u32;

    for byte in cleaned {
        let v = table[byte as usize];
        if v == INVALID {
            return Err(ProtoError::InvalidBase64);
        }
        if v == PAD {
            padding += 1;
            continue;
        }
        if padding > 0 {
            return Err(ProtoError::InvalidBase64);
        }
        buf = (buf << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            #[allow(clippy::cast_possible_truncation)]
            let byte = (buf >> bits) as u8;
            out.push(byte);
        }
    }
    // Final group must satisfy the data/padding parity from RFC
    // 4648 §4: every 4 input chars decode to 3 bytes. A short final
    // group of 2 data chars (`bits == 4`) requires `==`; 3 data
    // chars (`bits == 2`) requires `=`; an aligned group needs no
    // padding. A 1-char tail (`bits == 6`) is always invalid.
    // Without this strict check, `"Zg="` decodes to the same byte
    // as `"Zg=="` and a truncated transmission goes undetected.
    let expected_padding = match bits {
        0 => 0,
        4 => 2,
        2 => 1,
        _ => return Err(ProtoError::InvalidBase64),
    };
    if padding != expected_padding {
        return Err(ProtoError::InvalidBase64);
    }
    Ok(out)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "tests assert structural invariants; .expect / .unwrap / panic! are the standard test idiom"
)]
mod tests {
    use super::*;

    #[test]
    fn base64_decodes_canonical_vectors() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYg==").unwrap(), b"foob");
        assert_eq!(base64_decode("Zm9vYmE=").unwrap(), b"fooba");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
    }

    #[test]
    fn base64_rejects_non_alphabet() {
        assert!(matches!(
            base64_decode("Zm9*"),
            Err(ProtoError::InvalidBase64)
        ));
    }

    #[test]
    fn base64_rejects_wrong_padding_count() {
        // `Zg=` decodes to the same `b"f"` as the correctly-padded
        // `Zg==`, but accepting it lets a truncated transmission
        // pass undetected. RFC 4648 §4 requires exactly 2 padding
        // chars for a 2-char residual.
        assert!(matches!(
            base64_decode("Zg="),
            Err(ProtoError::InvalidBase64)
        ));
        // Same for a 3-char residual missing its single padding char.
        assert!(matches!(
            base64_decode("Zm8"),
            Err(ProtoError::InvalidBase64)
        ));
        // And rejected: an aligned group with stray padding.
        assert!(matches!(
            base64_decode("Zm9v="),
            Err(ProtoError::InvalidBase64)
        ));
    }

    #[test]
    fn base64_rejects_data_after_padding() {
        assert!(matches!(
            base64_decode("Zg==Zg=="),
            Err(ProtoError::InvalidBase64)
        ));
    }

    #[test]
    fn base64_tolerates_whitespace() {
        assert_eq!(base64_decode("Zm9v\n YmFy").unwrap(), b"foobar");
    }

    // ============================================================
    // JSON string escapes — every backslash escape, surrogate pair,
    // and the lone/short surrogate error paths.
    // ============================================================

    fn parse_str(input: &str) -> Result<String, ProtoError> {
        let mut p = JsonParser::new(input);
        p.parse_string()
    }

    #[test]
    fn string_escapes_decode() {
        // RFC 8259 §7 short escapes. Each one is its own branch in
        // `parse_string`; cover them together so the table is unbroken.
        let cases = [
            (r#""\"""#, "\""),
            (r#""\\""#, "\\"),
            (r#""\/""#, "/"),
            (r#""\n""#, "\n"),
            (r#""\t""#, "\t"),
            (r#""\r""#, "\r"),
            (r#""\b""#, "\u{0008}"),
            (r#""\f""#, "\u{000c}"),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_str(input).unwrap(), expected, "input: {input}");
        }
    }

    #[test]
    fn string_unicode_escapes_decode() {
        // BMP code point.
        assert_eq!(parse_str(r#""é""#).unwrap(), "é");
        // Astral plane code point via UTF-16 surrogate pair (😀 = U+1F600).
        assert_eq!(parse_str(r#""😀""#).unwrap(), "😀");
    }

    #[test]
    fn string_unterminated_rejected() {
        assert!(matches!(
            parse_str(r#""no closing quote"#),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn string_dangling_backslash_rejected() {
        assert!(matches!(
            parse_str("\"\\"),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn string_unsupported_escape_rejected() {
        // `\x` isn't a JSON-defined escape.
        assert!(matches!(
            parse_str(r#""\x41""#),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn unicode_escape_short_hex_rejected() {
        assert!(matches!(
            parse_str(r#""\u00""#),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn unicode_escape_bad_hex_rejected() {
        assert!(matches!(
            parse_str(r#""\uZZZZ""#),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn unicode_lone_high_surrogate_rejected() {
        // High surrogate not followed by `\u`.
        assert!(matches!(
            parse_str(r#""\ud83dABCD""#),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn unicode_high_surrogate_truncated_low_rejected() {
        // High surrogate followed by `\u` but fewer than 4 hex chars.
        assert!(matches!(
            parse_str(r#""\ud83d\u00""#),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn unicode_bad_low_surrogate_hex_rejected() {
        assert!(matches!(
            parse_str(r#""\ud83d\uZZZZ""#),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn unicode_low_surrogate_out_of_range_rejected() {
        // High surrogate followed by `\u` where the second value
        // isn't in the low-surrogate range.
        assert!(matches!(
            parse_str(r#""\ud83dA""#),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn unicode_lone_low_surrogate_rejected() {
        // Low surrogate without a preceding high surrogate.
        assert!(matches!(
            parse_str(r#""\udc00""#),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    // ============================================================
    // Primitive parser branches: bool / null / number.
    // ============================================================

    #[test]
    fn bool_typo_rejected() {
        // `true`/`false` are the only valid starts; `tru` fails
        // the `starts_with("true")` and `starts_with("false")` arms.
        assert!(matches!(
            JsonParser::new("tru").parse_value(),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn null_typo_rejected() {
        assert!(matches!(
            JsonParser::new("nul").parse_value(),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn number_lone_minus_rejected() {
        assert!(matches!(
            JsonParser::new("-").parse_value(),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn number_negative_parses() {
        // The `-` branch in `parse_number` is otherwise unexercised.
        let v = JsonParser::new("-42").parse_value().unwrap();
        assert_eq!(v, Value::Int(-42));
    }

    #[test]
    fn unexpected_top_level_token_rejected() {
        assert!(matches!(
            parse_inbound(b"$bogus"),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    // ============================================================
    // parse_inbound dispatch error paths.
    // ============================================================

    #[test]
    fn inbound_invalid_utf8_rejected() {
        // Lone continuation byte never starts a valid UTF-8 sequence.
        assert!(matches!(
            parse_inbound(&[0xff, 0xff, 0xff]),
            Err(ProtoError::InvalidUtf8)
        ));
    }

    #[test]
    fn inbound_top_level_non_object_rejected() {
        assert!(matches!(
            parse_inbound(b"[1, 2, 3]"),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn inbound_unknown_evt_rejected() {
        let line = br#"{"evt":"mystery"}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::UnknownKind(ref k)) if k == "mystery"
        ));
    }

    #[test]
    fn inbound_unknown_cmd_rejected() {
        let line = br#"{"cmd":"mystery"}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::UnknownKind(ref k)) if k == "mystery"
        ));
    }

    #[test]
    fn inbound_prompt_null_decodes_to_none() {
        let line = br#"{"prompt":null}"#;
        let Inbound::Snapshot(snap) = parse_inbound(line).unwrap() else {
            panic!("expected snapshot");
        };
        assert_eq!(snap.prompt, None);
    }

    #[test]
    fn inbound_prompt_missing_id_rejected() {
        // `id` defaults to empty string via `field_str_opt` fallback;
        // an explicit empty string also trips the empty-id guard.
        let line = br#"{"prompt":{"id":""}}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::MissingField("prompt.id"))
        ));
    }

    #[test]
    fn inbound_prompt_wrong_shape_rejected() {
        let line = br#"{"prompt":42}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::BadValue {
                field: "prompt",
                ..
            })
        ));
    }

    #[test]
    fn inbound_turn_content_must_be_array() {
        let line = br#"{"evt":"turn","content":42}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::BadValue {
                field: "content",
                ..
            })
        ));
    }

    #[test]
    fn inbound_turn_content_block_must_be_object() {
        let line = br#"{"evt":"turn","content":[42]}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::BadValue {
                field: "content[]",
                ..
            })
        ));
    }

    #[test]
    fn inbound_turn_roundtrip_preserves_raw_json() {
        // Exercises `render_value` / `write_value` / `write_string`
        // across every Value variant (null, bool, int, str, array,
        // object) plus control-char escape in `write_string`.
        let line = br#"{"evt":"turn","content":[{"type":"text","text":"hi"},{"type":"tool_use","name":"Bash","input":{"cmd":"ls","args":["-l"],"silent":true,"timeout":null,"sep":"\t","raw":""}}]}"#;
        let Inbound::Turn(turn) = parse_inbound(line).unwrap() else {
            panic!("expected turn");
        };
        assert_eq!(turn.role, "assistant");
        assert_eq!(turn.content.len(), 2);
        assert_eq!(turn.content[0].kind, "text");
        assert_eq!(turn.content[0].text.as_deref(), Some("hi"));
        // raw_json for the tool_use block must include every nested
        // type the serializer can emit.
        let raw = &turn.content[1].raw_json;
        assert!(raw.contains(r#""type":"tool_use""#));
        assert!(raw.contains(r#""silent":true"#));
        assert!(raw.contains(r#""timeout":null"#));
        assert!(raw.contains(r#""args":["-l"]"#));
        assert!(raw.contains(r#""sep":"\t""#));
        assert!(raw.contains("\"raw\":\"\""));
    }

    #[test]
    fn inbound_turn_serializer_escapes_control_chars() {
        // Hits the `c if (c as u32) < 0x20` branch of `write_string`.
        // Feed a `SOH` escape (legal JSON); the re-serialized block
        // must encode the same control char back as `SOH`.
        let line = b"{\"evt\":\"turn\",\"content\":[{\"type\":\"x\",\"ctrl\":\"\\u0001\"}]}";
        let Inbound::Turn(turn) = parse_inbound(line).unwrap() else {
            panic!("expected turn");
        };
        assert!(turn.content[0].raw_json.contains("\"ctrl\":\"\\u0001\""));
    }

    #[test]
    fn inbound_turn_serializer_emits_false_and_negative_int() {
        // Hits `Value::Bool(false)` + negative-int branches of
        // `write_value` that the preceding test doesn't.
        let line = br#"{"evt":"turn","content":[{"type":"x","flag":false,"n":-3}]}"#;
        let Inbound::Turn(turn) = parse_inbound(line).unwrap() else {
            panic!("expected turn");
        };
        let raw = &turn.content[0].raw_json;
        assert!(raw.contains(r#""flag":false"#));
        assert!(raw.contains(r#""n":-3"#));
    }

    #[test]
    fn inbound_time_must_be_array() {
        let line = br#"{"time":42}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::BadValue { field: "time", .. })
        ));
    }

    #[test]
    fn inbound_time_array_wrong_length() {
        let line = br#"{"time":[123]}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::BadValue { field: "time", .. })
        ));
    }

    #[test]
    fn inbound_time_tz_offset_out_of_i32_range() {
        // `tz_offset_secs` is an `i32`. Pass a value that fits in
        // i64 but overflows i32.
        let line = br#"{"time":[100,9999999999]}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::BadValue {
                field: "time[1]",
                ..
            })
        ));
    }

    #[test]
    fn inbound_time_sync_decodes() {
        let line = br#"{"time":[1700000000,-28800]}"#;
        let Inbound::TimeSync {
            epoch_secs,
            tz_offset_secs,
        } = parse_inbound(line).unwrap()
        else {
            panic!("expected time sync");
        };
        assert_eq!(epoch_secs, 1_700_000_000);
        assert_eq!(tz_offset_secs, -28_800);
    }

    #[test]
    fn inbound_chunk_invalid_base64_rejected() {
        let line = br#"{"cmd":"chunk","d":"Zg=="X"}"#;
        // Outer parse fails on the malformed JSON first; isolate the
        // base64 path with a syntactically valid envelope.
        assert!(parse_inbound(line).is_err());

        let line = br#"{"cmd":"chunk","d":"Zg*=="}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::InvalidBase64)
        ));
    }

    #[test]
    fn inbound_snapshot_keepalive_decodes_to_default() {
        let line = br"{}";
        let Inbound::Snapshot(snap) = parse_inbound(line).unwrap() else {
            panic!("expected snapshot");
        };
        assert_eq!(snap, Snapshot::default());
    }

    #[test]
    fn inbound_snapshot_msg_null_yields_empty_string() {
        // Exercises `expect_str_or_null`'s `Null` arm via `msg`.
        let line = br#"{"msg":null,"entries":["a","b"],"tokens":7,"tokens_today":3,"total":1,"running":0,"waiting":0}"#;
        let Inbound::Snapshot(snap) = parse_inbound(line).unwrap() else {
            panic!("expected snapshot");
        };
        assert_eq!(snap.msg, "");
        assert_eq!(snap.entries, alloc::vec!["a", "b"]);
        assert_eq!(snap.tokens, 7);
        assert_eq!(snap.tokens_today, 3);
    }

    #[test]
    fn inbound_snapshot_msg_wrong_type_rejected() {
        let line = br#"{"msg":42}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::BadValue { field: "msg", .. })
        ));
    }

    #[test]
    fn inbound_snapshot_entries_non_array_rejected() {
        let line = br#"{"entries":"oops"}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::BadValue {
                field: "entries",
                ..
            })
        ));
    }

    #[test]
    fn inbound_snapshot_entries_non_string_element_rejected() {
        let line = br#"{"entries":[1,2]}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::BadValue {
                field: "entries",
                ..
            })
        ));
    }

    #[test]
    fn inbound_snapshot_total_out_of_u32_range_rejected() {
        // Negative trips `u32::try_from` in `expect_u32`.
        let line = br#"{"total":-1}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::BadValue { field: "total", .. })
        ));
    }

    #[test]
    fn inbound_snapshot_tokens_negative_rejected() {
        // Negative trips `u64::try_from` in `expect_u64`.
        let line = br#"{"tokens":-1}"#;
        assert!(matches!(
            parse_inbound(line),
            Err(ProtoError::BadValue {
                field: "tokens",
                ..
            })
        ));
    }

    // ============================================================
    // parse_outbound dispatch — wholly uncovered before this PR.
    // ============================================================

    #[test]
    fn outbound_invalid_utf8_rejected() {
        assert!(matches!(
            parse_outbound(&[0xff, 0xff]),
            Err(ProtoError::InvalidUtf8)
        ));
    }

    #[test]
    fn outbound_top_level_non_object_rejected() {
        assert!(matches!(
            parse_outbound(b"42"),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn outbound_neither_ack_nor_cmd_rejected() {
        assert!(matches!(
            parse_outbound(b"{}"),
            Err(ProtoError::MalformedJson(_))
        ));
    }

    #[test]
    fn outbound_generic_ack_decodes() {
        let line = br#"{"ack":"name","ok":true,"n":3,"error":null}"#;
        let Outbound::Ack(ack) = parse_outbound(line).unwrap() else {
            panic!("expected ack");
        };
        assert_eq!(ack.cmd, "name");
        assert!(ack.ok);
        assert_eq!(ack.n, 3);
        assert_eq!(ack.error, None);
    }

    #[test]
    fn outbound_ack_with_error_string_decodes() {
        let line = br#"{"ack":"file","ok":false,"error":"disk full"}"#;
        let Outbound::Ack(ack) = parse_outbound(line).unwrap() else {
            panic!("expected ack");
        };
        assert!(!ack.ok);
        assert_eq!(ack.n, 0); // default when absent
        assert_eq!(ack.error.as_deref(), Some("disk full"));
    }

    #[test]
    fn outbound_status_ack_decodes_all_subobjects() {
        // Exercises parse_status_data + parse_battery + parse_sys
        // + parse_user_stats end-to-end.
        let line = br#"{"ack":"status","data":{"name":"Clawd","sec":true,"bat":{"pct":85,"mV":4100,"mA":-120,"usb":true},"sys":{"up":600,"heap":150000},"stats":{"appr":12,"deny":3,"vel":2,"nap":1,"lvl":5}}}"#;
        let Outbound::StatusAck(data) = parse_outbound(line).unwrap() else {
            panic!("expected status ack");
        };
        assert_eq!(data.name, "Clawd");
        assert!(data.sec);
        let bat = data.battery.expect("battery present");
        assert_eq!(bat.pct, 85);
        assert_eq!(bat.mv, 4100);
        assert_eq!(bat.ma, -120);
        assert!(bat.usb);
        let sys = data.sys.expect("sys present");
        assert_eq!(sys.uptime_secs, 600);
        assert_eq!(sys.heap_free_bytes, 150_000);
        let stats = data.stats.expect("stats present");
        assert_eq!(stats.approvals, 12);
        assert_eq!(stats.denies, 3);
        assert_eq!(stats.velocity, 2);
        assert_eq!(stats.naps, 1);
        assert_eq!(stats.level, 5);
    }

    #[test]
    fn outbound_status_ack_missing_data_rejected() {
        let line = br#"{"ack":"status"}"#;
        assert!(matches!(
            parse_outbound(line),
            Err(ProtoError::MissingField("data"))
        ));
    }

    #[test]
    fn outbound_status_ack_data_wrong_shape_rejected() {
        let line = br#"{"ack":"status","data":42}"#;
        assert!(matches!(
            parse_outbound(line),
            Err(ProtoError::BadValue { field: "data", .. })
        ));
    }

    #[test]
    fn outbound_status_ack_bat_wrong_shape_rejected() {
        let line = br#"{"ack":"status","data":{"bat":42}}"#;
        assert!(matches!(
            parse_outbound(line),
            Err(ProtoError::BadValue { field: "bat", .. })
        ));
    }

    #[test]
    fn outbound_status_ack_sys_wrong_shape_rejected() {
        let line = br#"{"ack":"status","data":{"sys":42}}"#;
        assert!(matches!(
            parse_outbound(line),
            Err(ProtoError::BadValue { field: "sys", .. })
        ));
    }

    #[test]
    fn outbound_status_ack_stats_wrong_shape_rejected() {
        let line = br#"{"ack":"status","data":{"stats":42}}"#;
        assert!(matches!(
            parse_outbound(line),
            Err(ProtoError::BadValue { field: "stats", .. })
        ));
    }

    #[test]
    fn outbound_status_ack_battery_pct_out_of_u8_range_rejected() {
        let line = br#"{"ack":"status","data":{"bat":{"pct":300,"mV":4100,"mA":0,"usb":false}}}"#;
        assert!(matches!(
            parse_outbound(line),
            Err(ProtoError::BadValue {
                field: "bat.pct",
                ..
            })
        ));
    }

    #[test]
    fn outbound_status_ack_battery_mv_out_of_u16_range_rejected() {
        let line = br#"{"ack":"status","data":{"bat":{"pct":50,"mV":99999,"mA":0,"usb":false}}}"#;
        assert!(matches!(
            parse_outbound(line),
            Err(ProtoError::BadValue {
                field: "bat.mV",
                ..
            })
        ));
    }

    #[test]
    fn outbound_status_ack_battery_ma_out_of_i16_range_rejected() {
        let line =
            br#"{"ack":"status","data":{"bat":{"pct":50,"mV":4100,"mA":99999,"usb":false}}}"#;
        assert!(matches!(
            parse_outbound(line),
            Err(ProtoError::BadValue {
                field: "bat.mA",
                ..
            })
        ));
    }

    #[test]
    fn outbound_permission_decodes_both_decisions() {
        for (raw, expected) in [
            (
                br#"{"cmd":"permission","id":"abc","decision":"once"}"#.as_slice(),
                Decision::Once,
            ),
            (
                br#"{"cmd":"permission","id":"abc","decision":"deny"}"#.as_slice(),
                Decision::Deny,
            ),
        ] {
            let Outbound::Permission { id, decision } = parse_outbound(raw).unwrap() else {
                panic!("expected permission");
            };
            assert_eq!(id, "abc");
            assert_eq!(decision, expected);
        }
    }

    #[test]
    fn outbound_permission_unknown_decision_rejected() {
        let line = br#"{"cmd":"permission","id":"abc","decision":"maybe"}"#;
        assert!(matches!(
            parse_outbound(line),
            Err(ProtoError::BadValue {
                field: "decision",
                ..
            })
        ));
    }

    #[test]
    fn outbound_unknown_cmd_rejected() {
        let line = br#"{"cmd":"reboot"}"#;
        assert!(matches!(
            parse_outbound(line),
            Err(ProtoError::UnknownKind(ref k)) if k == "reboot"
        ));
    }
}
