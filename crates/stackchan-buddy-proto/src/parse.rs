//! Bytes → [`Inbound`].
//!
//! Strategy: parse each line into a small generic JSON value tree
//! (alloc'd once), then dispatch on the top-level keys. This is
//! simpler than streaming dispatch because the buddy protocol's
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

/// Parse one newline-delimited buddy message.
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
    if bits >= 6 || padding > 2 {
        return Err(ProtoError::InvalidBase64);
    }
    Ok(out)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests assert structural invariants; .expect / .unwrap are the standard test idiom"
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
}
