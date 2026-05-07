//! Minimal Model Context Protocol (MCP) server transport.
//!
//! Implements just enough of MCP for an external assistant to drive
//! Stack-chan: the JSON-RPC 2.0 envelope, the three required MCP
//! methods (`initialize`, `tools/list`, `tools/call`), and a small
//! tool catalogue covering emotion, mood, look-at, speak, and state
//! reads. The firmware [`POST /mcp`](https://modelcontextprotocol.io/)
//! route reads a request body, hands it to [`parse_request`], maps
//! the method onto the existing control-plane primitives, and renders
//! the response with one of the `render_*_response` helpers.
//!
//! ## Why hand-rolled
//!
//! Same rationale as [`crate::bare_json`]: pulling `serde_json` onto
//! `xtensa-esp32s3-none-elf` cascades into `serde/std + base64/std`,
//! which doesn't compile on the firmware target. The MCP wire surface
//! is small enough that a hand-rolled parser is simpler than feature-
//! gating a serde-based one.
//!
//! ## What's covered
//!
//! - JSON-RPC 2.0 envelope (`jsonrpc`, `id`, `method`, `params`) and
//!   the standard error codes (`ParseError = -32700`,
//!   `InvalidRequest = -32600`, `MethodNotFound = -32601`,
//!   `InvalidParams = -32602`, `InternalError = -32603`).
//! - MCP `initialize` (capability negotiation; returns only the
//!   `tools` capability since this server has no resources or prompts).
//! - MCP `tools/list` (returns the static [`TOOLS_LIST_RESULT_JSON`]
//!   catalogue).
//! - MCP `tools/call` parameter parsing — extracts `name` and the
//!   `arguments` object slice; dispatch happens in the firmware so
//!   the per-tool handler can call into the appropriate control-plane
//!   primitive.
//!
//! ## What's not
//!
//! - Notifications (requests with no `id`): MCP doesn't currently use
//!   them outside of `cancelled` and progress, neither of which the
//!   firmware needs.
//! - Streaming HTTP / SSE: an MCP client polls `POST /mcp` per
//!   request. Adding SSE would compose the existing
//!   [`embassy_sync::pubsub::PubSubChannel`] pattern from the avatar
//!   snapshot stream — out of scope for this skeleton.
//! - Resources / prompts / roots / sampling: the server returns no
//!   capability for these, so MCP clients will skip them.
//!
//! [`crate::bare_json`]: crate::bare_json
//! [`embassy_sync::pubsub::PubSubChannel`]: https://docs.rs/embassy-sync/

use alloc::format;
use alloc::string::String;

/// JSON-RPC 2.0 protocol-level error codes.
///
/// Numeric values match the spec — see
/// <https://www.jsonrpc.org/specification#error_object>. The firmware
/// maps them onto HTTP status codes when serialising the outer
/// envelope: parse / invalid-request return `400`; everything else
/// returns `200` with the error encoded in the JSON body, since a
/// `tools/call` failure is part of the normal protocol flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JsonRpcErrorCode {
    /// `-32700` — body wasn't valid JSON.
    ParseError,
    /// `-32600` — JSON parsed but isn't a valid JSON-RPC request
    /// (missing `jsonrpc`, missing `method`, wrong types, …).
    InvalidRequest,
    /// `-32601` — `method` doesn't match any handler.
    MethodNotFound,
    /// `-32602` — `params` is the wrong shape for the requested method.
    InvalidParams,
    /// `-32603` — server-side failure mapping the call onto a control
    /// plane primitive (e.g. enqueue fail, no SD card).
    InternalError,
}

impl JsonRpcErrorCode {
    /// Spec-defined integer code.
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::ParseError => -32_700,
            Self::InvalidRequest => -32_600,
            Self::MethodNotFound => -32_601,
            Self::InvalidParams => -32_602,
            Self::InternalError => -32_603,
        }
    }

    /// Short human-readable label for the `error.message` field.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::ParseError => "Parse error",
            Self::InvalidRequest => "Invalid Request",
            Self::MethodNotFound => "Method not found",
            Self::InvalidParams => "Invalid params",
            Self::InternalError => "Internal error",
        }
    }
}

/// JSON-RPC request id.
///
/// The spec allows any of `null` / number / string; this server
/// requires a numeric id and rejects anything else as
/// `InvalidRequest`. MCP clients all use numeric ids in practice.
pub type RequestId = i64;

/// Parsed JSON-RPC request — keeps the params slice borrowed from
/// the input so the per-method handler can re-parse the inner object
/// against its own schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRequest<'body> {
    /// Numeric request id. The firmware echoes this on the response.
    pub id: RequestId,
    /// Method name (e.g. `"initialize"`, `"tools/list"`, `"tools/call"`).
    pub method: &'body str,
    /// Slice of the original body covering the `params` value, or
    /// `None` if the request had no `params` field. Callers re-parse
    /// this against the method-specific schema; this layer doesn't
    /// understand individual method shapes.
    pub params_raw: Option<&'body str>,
}

/// Errors produced by [`parse_request`]. Each variant carries a
/// `JsonRpcErrorCode` so the caller can map directly onto the JSON-RPC
/// error envelope without a second translation step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError {
    /// JSON-RPC error code to surface in the response.
    pub code: JsonRpcErrorCode,
    /// One-line human-readable diagnostic.
    pub detail: &'static str,
}

impl ParseError {
    /// Construct a parse-error variant.
    #[must_use]
    pub const fn parse(detail: &'static str) -> Self {
        Self {
            code: JsonRpcErrorCode::ParseError,
            detail,
        }
    }

    /// Construct an invalid-request variant.
    #[must_use]
    pub const fn invalid_request(detail: &'static str) -> Self {
        Self {
            code: JsonRpcErrorCode::InvalidRequest,
            detail,
        }
    }
}

/// Parse a JSON-RPC 2.0 request envelope.
///
/// Returns method, numeric id, and a slice of the params object.
/// Validates `jsonrpc == "2.0"` and rejects anything that doesn't
/// match the closed top-level schema (`jsonrpc`, `id`, `method`,
/// optional `params`). Strict on the envelope; permissive inside
/// `params` (the slice is returned unparsed).
///
/// # Errors
///
/// - [`JsonRpcErrorCode::ParseError`] on malformed JSON.
/// - [`JsonRpcErrorCode::InvalidRequest`] on missing required fields,
///   wrong types, or unknown top-level keys.
pub fn parse_request(body: &str) -> Result<ParsedRequest<'_>, ParseError> {
    let bytes = body.as_bytes();
    let mut pos = skip_ws(bytes, 0);
    if pos >= bytes.len() || bytes[pos] != b'{' {
        return Err(ParseError::parse("expected JSON object"));
    }
    pos += 1;

    let mut jsonrpc_seen = false;
    let mut id: Option<RequestId> = None;
    let mut method: Option<&str> = None;
    let mut params_raw: Option<&str> = None;
    // `first` rather than the byte position because leading whitespace
    // before `{`, or whitespace between `{` and the first key, would
    // otherwise push `pos > 1` past the first iteration and trip the
    // comma-required guard before any key has been read. Same pattern
    // as `find_string_field` / `find_object_field` below.
    let mut first = true;

    loop {
        pos = skip_ws(bytes, pos);
        if pos >= bytes.len() {
            return Err(ParseError::parse("unterminated object"));
        }
        if bytes[pos] == b'}' {
            break;
        }
        if !first {
            // Expect a comma between members. Tolerate whitespace.
            if bytes[pos] != b',' {
                return Err(ParseError::parse("expected ',' between members"));
            }
            pos += 1;
            pos = skip_ws(bytes, pos);
        }
        first = false;
        let (key, after_key) = read_string(bytes, pos)?;
        pos = skip_ws(bytes, after_key);
        if pos >= bytes.len() || bytes[pos] != b':' {
            return Err(ParseError::parse("expected ':' after key"));
        }
        pos = skip_ws(bytes, pos + 1);

        match key {
            "jsonrpc" => {
                let (val, after) = read_string(bytes, pos)?;
                if val != "2.0" {
                    return Err(ParseError::invalid_request(
                        "jsonrpc field must equal \"2.0\"",
                    ));
                }
                jsonrpc_seen = true;
                pos = after;
            }
            "id" => {
                let (val, after) = read_integer(bytes, pos)?;
                id = Some(val);
                pos = after;
            }
            "method" => {
                let (val, after) = read_string(bytes, pos)?;
                let start = bytes
                    .get(after.saturating_sub(val.len() + 1))
                    .map_or(after, |_| after - val.len() - 1);
                let _ = start; // suppress unused; method slice already produced
                method = Some(val);
                pos = after;
            }
            "params" => {
                // Capture the whole params value (object/array/literal)
                // as a slice so the dispatcher can re-parse it against
                // the method-specific schema.
                let (slice, after) = take_value(body, bytes, pos)?;
                params_raw = Some(slice);
                pos = after;
            }
            _ => return Err(ParseError::invalid_request("unknown top-level key")),
        }
    }

    if !jsonrpc_seen {
        return Err(ParseError::invalid_request("missing 'jsonrpc' field"));
    }
    let Some(id) = id else {
        return Err(ParseError::invalid_request("missing 'id' field"));
    };
    let Some(method) = method else {
        return Err(ParseError::invalid_request("missing 'method' field"));
    };

    Ok(ParsedRequest {
        id,
        method,
        params_raw,
    })
}

/// Skip ASCII whitespace from position `start`.
fn skip_ws(bytes: &[u8], start: usize) -> usize {
    let mut p = start;
    while p < bytes.len() && matches!(bytes[p], b' ' | b'\t' | b'\n' | b'\r') {
        p += 1;
    }
    p
}

/// Read a JSON string starting at position `start` (which must point
/// at a `"`). Supports `\\`, `\"`, `\n`, `\r`, `\t`, `\b`, `\f`, `\/`;
/// `\uXXXX` Unicode escapes are *not* supported and reject as a
/// `ParseError`. The MCP wire surface in practice doesn't carry
/// Unicode-escaped strings — clients write CJK / emoji as raw UTF-8 —
/// but an LLM-generated body that uses `ja` for `"ja"`
/// would be rejected here. Document the gap explicitly so a future
/// schema change is aware before adding a field where the limitation
/// would matter (a free-text `phrase` argument, for example).
///
/// The slice is borrowed from the input — no allocation, no escape
/// processing for the consumer side. Method names and tool names
/// don't carry escapes in practice; if the caller needs a fully
/// unescaped string it'll have to do that itself.
fn read_string(bytes: &[u8], start: usize) -> Result<(&str, usize), ParseError> {
    if start >= bytes.len() || bytes[start] != b'"' {
        return Err(ParseError::parse("expected string"));
    }
    let body_start = start + 1;
    let mut p = body_start;
    while p < bytes.len() {
        match bytes[p] {
            b'"' => {
                let raw = core::str::from_utf8(&bytes[body_start..p])
                    .map_err(|_| ParseError::parse("string contained invalid UTF-8"))?;
                return Ok((raw, p + 1));
            }
            b'\\' => {
                if p + 1 >= bytes.len() {
                    return Err(ParseError::parse("dangling escape in string"));
                }
                if !matches!(
                    bytes[p + 1],
                    b'"' | b'\\' | b'/' | b'n' | b'r' | b't' | b'b' | b'f'
                ) {
                    return Err(ParseError::parse("unsupported string escape"));
                }
                p += 2;
            }
            _ => p += 1,
        }
    }
    Err(ParseError::parse("unterminated string"))
}

/// Read a signed integer at `start` (whitespace-skipped).
fn read_integer(bytes: &[u8], start: usize) -> Result<(i64, usize), ParseError> {
    let mut p = start;
    let neg = if p < bytes.len() && bytes[p] == b'-' {
        p += 1;
        true
    } else {
        false
    };
    let digit_start = p;
    while p < bytes.len() && bytes[p].is_ascii_digit() {
        p += 1;
    }
    if p == digit_start {
        return Err(ParseError::invalid_request("expected integer"));
    }
    let raw = core::str::from_utf8(&bytes[digit_start..p])
        .map_err(|_| ParseError::parse("integer contained invalid UTF-8"))?;
    let parsed: i64 = raw
        .parse()
        .map_err(|_| ParseError::invalid_request("integer out of i64 range"))?;
    Ok((if neg { -parsed } else { parsed }, p))
}

/// Take a complete JSON value starting at `start` and return it as a
/// borrowed slice. Doesn't validate the value's contents; just walks
/// until balanced. Used for capturing the `params` object so the
/// per-method handler can re-parse it.
///
/// Handles nested objects, arrays, strings (with escapes), and
/// scalar literals (numbers, `true`, `false`, `null`).
fn take_value<'b>(
    body: &'b str,
    bytes: &[u8],
    start: usize,
) -> Result<(&'b str, usize), ParseError> {
    let mut p = start;
    if p >= bytes.len() {
        return Err(ParseError::parse("expected value"));
    }
    let begin = p;
    match bytes[p] {
        b'{' | b'[' => {
            let open = bytes[p];
            let close = if open == b'{' { b'}' } else { b']' };
            let mut depth = 1;
            p += 1;
            while p < bytes.len() && depth > 0 {
                match bytes[p] {
                    b'"' => {
                        let (_, after) = read_string(bytes, p)?;
                        p = after;
                        continue;
                    }
                    c if c == open => depth += 1,
                    c if c == close => depth -= 1,
                    _ => {}
                }
                p += 1;
            }
            if depth != 0 {
                return Err(ParseError::parse("unbalanced JSON value"));
            }
        }
        b'"' => {
            let (_, after) = read_string(bytes, p)?;
            p = after;
        }
        _ => {
            // Scalar: read until comma, closing brace, or whitespace.
            while p < bytes.len()
                && !matches!(bytes[p], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
            {
                p += 1;
            }
        }
    }
    Ok((&body[begin..p], p))
}

/// Find the value of a top-level string field inside `params_raw`.
///
/// Helper for per-method dispatch: a tool handler that wants the
/// `name` field of a `tools/call` payload can call
/// `find_string_field(params, "name")` instead of writing a full
/// nested parser. Whitespace tolerant; rejects escaped strings (the
/// fields MCP carries — tool names, emotion strings — are plain
/// ASCII identifiers).
///
/// Returns `Ok(None)` for missing keys, `Ok(Some(_))` for found keys
/// with simple string values, and `Err` on malformed JSON.
///
/// # Errors
///
/// [`JsonRpcErrorCode::InvalidParams`] on a malformed object.
pub fn find_string_field<'a>(
    params_raw: &'a str,
    key: &str,
) -> Result<Option<&'a str>, ParseError> {
    let bytes = params_raw.as_bytes();
    let mut pos = skip_ws(bytes, 0);
    if pos >= bytes.len() || bytes[pos] != b'{' {
        return Err(ParseError {
            code: JsonRpcErrorCode::InvalidParams,
            detail: "params is not an object",
        });
    }
    pos += 1;
    let mut first = true;
    loop {
        pos = skip_ws(bytes, pos);
        if pos >= bytes.len() {
            return Err(ParseError {
                code: JsonRpcErrorCode::InvalidParams,
                detail: "unterminated params object",
            });
        }
        if bytes[pos] == b'}' {
            return Ok(None);
        }
        if !first {
            if bytes[pos] != b',' {
                return Err(ParseError {
                    code: JsonRpcErrorCode::InvalidParams,
                    detail: "expected ',' between members",
                });
            }
            pos += 1;
            pos = skip_ws(bytes, pos);
        }
        first = false;
        let (k, after_k) = read_string(bytes, pos).map_err(|e| ParseError {
            code: JsonRpcErrorCode::InvalidParams,
            detail: e.detail,
        })?;
        pos = skip_ws(bytes, after_k);
        if pos >= bytes.len() || bytes[pos] != b':' {
            return Err(ParseError {
                code: JsonRpcErrorCode::InvalidParams,
                detail: "expected ':' after key",
            });
        }
        pos = skip_ws(bytes, pos + 1);
        if k == key {
            let (val, _) = read_string(bytes, pos).map_err(|e| ParseError {
                code: JsonRpcErrorCode::InvalidParams,
                detail: e.detail,
            })?;
            return Ok(Some(val));
        }
        // Skip the value to the next member.
        let (_, after_val) = take_value(params_raw, bytes, pos).map_err(|e| ParseError {
            code: JsonRpcErrorCode::InvalidParams,
            detail: e.detail,
        })?;
        pos = after_val;
    }
}

/// Return the slice of `params_raw` covering the `arguments` field
/// of a `tools/call` request, or `None` when absent. Object slice is
/// returned including the surrounding `{}`.
///
/// # Errors
///
/// [`JsonRpcErrorCode::InvalidParams`] on a malformed object.
pub fn find_object_field<'a>(
    params_raw: &'a str,
    key: &str,
) -> Result<Option<&'a str>, ParseError> {
    let bytes = params_raw.as_bytes();
    let mut pos = skip_ws(bytes, 0);
    if pos >= bytes.len() || bytes[pos] != b'{' {
        return Err(ParseError {
            code: JsonRpcErrorCode::InvalidParams,
            detail: "params is not an object",
        });
    }
    pos += 1;
    let mut first = true;
    loop {
        pos = skip_ws(bytes, pos);
        if pos >= bytes.len() {
            return Err(ParseError {
                code: JsonRpcErrorCode::InvalidParams,
                detail: "unterminated params object",
            });
        }
        if bytes[pos] == b'}' {
            return Ok(None);
        }
        if !first {
            if bytes[pos] != b',' {
                return Err(ParseError {
                    code: JsonRpcErrorCode::InvalidParams,
                    detail: "expected ',' between members",
                });
            }
            pos += 1;
            pos = skip_ws(bytes, pos);
        }
        first = false;
        let (k, after_k) = read_string(bytes, pos).map_err(|e| ParseError {
            code: JsonRpcErrorCode::InvalidParams,
            detail: e.detail,
        })?;
        pos = skip_ws(bytes, after_k);
        if pos >= bytes.len() || bytes[pos] != b':' {
            return Err(ParseError {
                code: JsonRpcErrorCode::InvalidParams,
                detail: "expected ':' after key",
            });
        }
        pos = skip_ws(bytes, pos + 1);
        if k == key {
            let (val, _) = take_value(params_raw, bytes, pos).map_err(|e| ParseError {
                code: JsonRpcErrorCode::InvalidParams,
                detail: e.detail,
            })?;
            return Ok(Some(val));
        }
        let (_, after_val) = take_value(params_raw, bytes, pos).map_err(|e| ParseError {
            code: JsonRpcErrorCode::InvalidParams,
            detail: e.detail,
        })?;
        pos = after_val;
    }
}

/// Render a JSON-RPC success response with a pre-rendered `result`
/// JSON value (string, object, etc.). Caller is responsible for
/// `result_json` being valid JSON.
#[must_use]
pub fn render_success(id: RequestId, result_json: &str) -> String {
    format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{result_json}}}")
}

/// Render a JSON-RPC error response. `detail` becomes the
/// `error.data.detail` string field; `error.message` is the standard
/// label for `code`.
#[must_use]
pub fn render_error(id: Option<RequestId>, code: JsonRpcErrorCode, detail: &str) -> String {
    let id_part = id.map_or_else(|| String::from("null"), |i| format!("{i}"));
    let escaped_detail = escape_string(detail);
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id_part},\"error\":{{\"code\":{c},\"message\":\"{m}\",\"data\":{{\"detail\":\"{escaped_detail}\"}}}}}}",
        c = code.as_i32(),
        m = code.message(),
    )
}

/// Escape a string for safe embedding inside a JSON string literal.
/// Handles the ASCII subset MCP error messages need; non-ASCII bytes
/// pass through unchanged (they're already valid UTF-8 by `&str`
/// invariant).
fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = core::fmt::Write::write_fmt(&mut out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Static MCP `initialize` result JSON. Hard-coded protocol version,
/// no resources / prompts / sampling capabilities, just `tools`.
///
/// `protocolVersion` matches the version the spec was at when this
/// firmware was written; clients that negotiate down still work
/// because the reply only declares capabilities the server actually
/// implements.
pub const INITIALIZE_RESULT_JSON: &str = concat!(
    "{",
    "\"protocolVersion\":\"2025-06-18\",",
    "\"capabilities\":{\"tools\":{}},",
    "\"serverInfo\":{\"name\":\"stackchan-kai\",\"version\":\"0.1\"}",
    "}",
);

/// Static MCP `tools/list` result JSON. Tools map onto the existing
/// HTTP control plane:
///
/// - `set_emotion(emotion: string, hold_ms?: integer)`
/// - `set_mood(mood: string)`
/// - `look_at(pan_deg: number, tilt_deg: number, hold_ms?: integer)`
/// - `speak(phrase: string, locale?: string)`
/// - `start_listen(duration_ms?: integer)`
/// - `enter_pairing(duration_ms?: integer)`
/// - `set_volume(level: integer)`
/// - `set_mute(muted: bool)`
/// - `create_reminder(fire_in_secs: integer, phrase: string) -> { id }`
/// - `list_reminders() -> { reminders: [...] }`
/// - `cancel_reminder(id: integer)`
/// - `get_state()`
///
/// Schemas are minimal — no enum constraints on emotion / mood /
/// phrase strings; the firmware-side tool handler returns
/// `InvalidParams` when an unknown value is passed.
pub const TOOLS_LIST_RESULT_JSON: &str = concat!(
    r#"{"tools":["#,
    r#"{"name":"set_emotion","description":"Set the avatar's emotion with an optional hold timer (milliseconds). Vocabulary: neutral, happy, sad, sleepy, surprised, angry, doubt, boring, hi, loved, curious, confused, mad.","inputSchema":{"type":"object","properties":{"emotion":{"type":"string"},"hold_ms":{"type":"integer"}},"required":["emotion"]}},"#,
    r#"{"name":"set_mood","description":"Set the operator-selected mood baseline. Vocabulary: neutral, calm, playful, focus, sleepy.","inputSchema":{"type":"object","properties":{"mood":{"type":"string"}},"required":["mood"]}},"#,
    r#"{"name":"look_at","description":"Aim the avatar's head at a pan/tilt target in degrees, with an optional hold timer.","inputSchema":{"type":"object","properties":{"pan_deg":{"type":"number"},"tilt_deg":{"type":"number"},"hold_ms":{"type":"integer"}},"required":["pan_deg","tilt_deg"]}},"#,
    r#"{"name":"speak","description":"Play a baked phrase or chirp through the speaker.","inputSchema":{"type":"object","properties":{"phrase":{"type":"string"},"locale":{"type":"string"}},"required":["phrase"]}},"#,
    r#"{"name":"start_listen","description":"Open a listen window: queue an acknowledge chirp, set Attention::Listening, arm the Ear decorator. Default 3000 ms.","inputSchema":{"type":"object","properties":{"duration_ms":{"type":"integer"}}}},"#,
    r#"{"name":"enter_pairing","description":"Open an ESP-NOW pairing window for the configured duration so an external remote can register.","inputSchema":{"type":"object","properties":{"duration_ms":{"type":"integer"}}}},"#,
    r#"{"name":"set_volume","description":"Set the speaker volume in percent (0..=100). Persists to STACKCHAN.RON; survives reboot.","inputSchema":{"type":"object","properties":{"level":{"type":"integer","minimum":0,"maximum":100}},"required":["level"]}},"#,
    r#"{"name":"set_mute","description":"Mute or unmute the speaker without losing the persisted volume level. Persists to STACKCHAN.RON.","inputSchema":{"type":"object","properties":{"muted":{"type":"boolean"}},"required":["muted"]}},"#,
    r#"{"name":"create_reminder","description":"Schedule a baked phrase to play in N seconds. Returns the reminder id. Runtime-only — does not survive reboot.","inputSchema":{"type":"object","properties":{"fire_in_secs":{"type":"integer","minimum":1,"maximum":432000},"phrase":{"type":"string"}},"required":["fire_in_secs","phrase"]}},"#,
    r#"{"name":"list_reminders","description":"Return the currently-scheduled reminders.","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"cancel_reminder","description":"Cancel a previously-scheduled reminder by id. Returns 404 / not-found if no matching reminder exists.","inputSchema":{"type":"object","properties":{"id":{"type":"integer"}},"required":["id"]}},"#,
    r#"{"name":"get_state","description":"Return the current avatar snapshot (emotion, mood, head pose, decorator, battery, Wi-Fi, audio).","inputSchema":{"type":"object","properties":{}}}"#,
    "]}",
);

/// Wrap an arbitrary tool-result string as MCP `tools/call` content.
///
/// MCP wraps every tool result in `{ "content": [...] }` where each
/// content item has a `type` and a payload. This helper picks the
/// `text` content type and embeds the raw text as a JSON-escaped
/// string.
#[must_use]
pub fn render_tool_text_result(text: &str) -> String {
    let escaped = escape_string(text);
    format!("{{\"content\":[{{\"type\":\"text\",\"text\":\"{escaped}\"}}]}}")
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::missing_docs_in_private_items,
    reason = "test-only: parser fixtures are well-formed by construction; \
              .unwrap() / .expect() on a fixture-derived Result is fine"
)]
mod tests {
    use super::*;

    #[test]
    fn parse_request_extracts_method_id_and_params() {
        let body = r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"set_emotion","arguments":{"emotion":"happy"}}}"#;
        let req = parse_request(body).expect("valid request should parse");
        assert_eq!(req.id, 42);
        assert_eq!(req.method, "tools/call");
        let params = req.params_raw.expect("params present");
        assert!(params.starts_with('{'));
        assert!(params.contains("\"name\":\"set_emotion\""));
    }

    #[test]
    fn parse_request_handles_missing_params() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let req = parse_request(body).expect("valid request");
        assert_eq!(req.method, "tools/list");
        assert_eq!(req.params_raw, None);
    }

    #[test]
    fn parse_request_rejects_wrong_jsonrpc_version() {
        let body = r#"{"jsonrpc":"1.0","id":1,"method":"x"}"#;
        let err = parse_request(body).expect_err("should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidRequest);
    }

    #[test]
    fn parse_request_rejects_missing_id() {
        let body = r#"{"jsonrpc":"2.0","method":"x"}"#;
        let err = parse_request(body).expect_err("should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidRequest);
    }

    #[test]
    fn parse_request_rejects_missing_method() {
        let body = r#"{"jsonrpc":"2.0","id":1}"#;
        let err = parse_request(body).expect_err("should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidRequest);
    }

    #[test]
    fn parse_request_rejects_garbage() {
        let err = parse_request("not json").expect_err("should reject");
        assert_eq!(err.code, JsonRpcErrorCode::ParseError);
    }

    #[test]
    fn parse_request_handles_leading_whitespace_before_brace() {
        // Regression: the comma guard used to key off byte position
        // and reject any input where `pos > 1` at the first key —
        // which fires for any HTTP body with a leading newline.
        let body = "\n  \t{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"x\"}";
        let req = parse_request(body).expect("leading whitespace should be tolerated");
        assert_eq!(req.method, "x");
    }

    #[test]
    fn parse_request_handles_whitespace_after_opening_brace() {
        // Regression: same byte-position bug. `{ "jsonrpc": ... }`
        // (one space after `{`) would have failed.
        let body = r#"{ "jsonrpc": "2.0", "id": 1, "method": "x" }"#;
        let req = parse_request(body).expect("inner whitespace should be tolerated");
        assert_eq!(req.method, "x");
    }

    #[test]
    fn parse_request_negative_id_works() {
        let body = r#"{"jsonrpc":"2.0","id":-7,"method":"x"}"#;
        let req = parse_request(body).expect("valid request");
        assert_eq!(req.id, -7);
    }

    #[test]
    fn find_string_field_returns_value() {
        let params = r#"{"name":"set_emotion","arguments":{"emotion":"happy"}}"#;
        assert_eq!(
            find_string_field(params, "name").unwrap(),
            Some("set_emotion")
        );
    }

    #[test]
    fn find_string_field_returns_none_when_absent() {
        let params = r#"{"name":"x"}"#;
        assert_eq!(find_string_field(params, "missing").unwrap(), None);
    }

    #[test]
    fn find_string_field_skips_object_values() {
        // The value of `arguments` is an object; the helper should
        // walk past it without choking.
        let params = r#"{"arguments":{"a":1,"b":"c"},"name":"x"}"#;
        assert_eq!(find_string_field(params, "name").unwrap(), Some("x"));
    }

    #[test]
    fn find_object_field_returns_object_slice() {
        let params = r#"{"name":"x","arguments":{"emotion":"happy","hold_ms":5000}}"#;
        let args = find_object_field(params, "arguments").unwrap().unwrap();
        assert!(args.starts_with('{'));
        assert!(args.ends_with('}'));
        assert!(args.contains("\"emotion\":\"happy\""));
    }

    #[test]
    fn render_success_emits_well_formed_envelope() {
        let s = render_success(7, r#"{"ok":true}"#);
        assert_eq!(s, r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#);
    }

    #[test]
    fn render_error_emits_well_formed_envelope() {
        let s = render_error(Some(3), JsonRpcErrorCode::MethodNotFound, "no such method");
        assert!(s.contains(r#""id":3"#));
        assert!(s.contains(r#""code":-32601"#));
        assert!(s.contains(r#""message":"Method not found""#));
        assert!(s.contains(r#""detail":"no such method""#));
    }

    #[test]
    fn render_error_handles_null_id_for_parse_failures() {
        // When parsing fails before we can read the id, the spec says
        // respond with id = null. Pin that the renderer emits the
        // bare token.
        let s = render_error(None, JsonRpcErrorCode::ParseError, "garbage");
        assert!(s.contains(r#""id":null"#));
    }

    #[test]
    fn escape_string_handles_control_chars() {
        assert_eq!(escape_string("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }

    #[test]
    fn render_tool_text_result_wraps_correctly() {
        let s = render_tool_text_result("hello");
        assert_eq!(s, r#"{"content":[{"type":"text","text":"hello"}]}"#);
    }

    #[test]
    fn tools_list_json_is_valid_round_trip_against_parser() {
        // Sanity: the static catalogue is itself valid JSON. Round-
        // trip through `take_value` to confirm it parses end-to-end.
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{TOOLS_LIST_RESULT_JSON}}}"#
        );
        parse_request(&body).expect("tools list catalogue should re-parse");
    }
}
