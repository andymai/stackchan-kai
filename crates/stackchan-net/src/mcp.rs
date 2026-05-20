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
                if jsonrpc_seen {
                    return Err(ParseError::invalid_request("duplicate 'jsonrpc' field"));
                }
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
                if id.is_some() {
                    return Err(ParseError::invalid_request("duplicate 'id' field"));
                }
                let (val, after) = read_integer(bytes, pos)?;
                id = Some(val);
                pos = after;
            }
            "method" => {
                if method.is_some() {
                    return Err(ParseError::invalid_request("duplicate 'method' field"));
                }
                let (val, after) = read_string(bytes, pos)?;
                method = Some(val);
                pos = after;
            }
            "params" => {
                if params_raw.is_some() {
                    return Err(ParseError::invalid_request("duplicate 'params' field"));
                }
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

/// Static MCP `tools/list` result JSON.
///
/// Each tool maps onto an existing control surface — either an HTTP
/// route on the firmware (`POST /emotion`, `POST /look-at`, …) or a
/// firmware-internal trigger (`enter_thinking` / `exit_thinking` are
/// fired by the sidecar-agent task; the MCP twins let an external
/// orchestrator drive the same state). The catalogue groups by
/// purpose: avatar style (emotion / mood / face geometry), motion
/// (look-at, look-at-point, play-motion), voice (speak, push-toast,
/// start-listen, thinking transitions), peripherals (volume / mute /
/// take-photo), reminders (create / list / cancel), lifecycle (sleep
/// / wake / get-state / reset), and pairing.
///
/// Schemas are minimal — no enum constraints on emotion / mood /
/// phrase strings; the firmware-side tool handler returns
/// `InvalidParams` when an unknown value is passed. The constant
/// body below is the authoritative per-tool name + description +
/// inputSchema; don't duplicate the inventory in this doc comment
/// because it drifts.
pub const TOOLS_LIST_RESULT_JSON: &str = concat!(
    r#"{"tools":["#,
    r#"{"name":"set_emotion","description":"Set the avatar's emotion with an optional hold timer (milliseconds). Vocabulary: neutral, happy, sad, sleepy, surprised, angry, doubt, boring, hi, loved, curious, confused, mad.","inputSchema":{"type":"object","properties":{"emotion":{"type":"string"},"hold_ms":{"type":"integer"}},"required":["emotion"]}},"#,
    r#"{"name":"set_mood","description":"Set the operator-selected mood baseline. Vocabulary: neutral, calm, playful, focus, sleepy.","inputSchema":{"type":"object","properties":{"mood":{"type":"string"}},"required":["mood"]}},"#,
    r#"{"name":"set_face_geometry","description":"Set the avatar's face geometry preset — swaps the eye + mouth baseline silhouette while preserving emotion-driven modulators. Vocabulary: default, chibi, wide, sleepy. Persists across reboots via the runtime store.","inputSchema":{"type":"object","properties":{"geometry":{"type":"string"}},"required":["geometry"]}},"#,
    r#"{"name":"look_at","description":"Aim the avatar's head at a pan/tilt target in degrees, with an optional hold timer.","inputSchema":{"type":"object","properties":{"pan_deg":{"type":"number"},"tilt_deg":{"type":"number"},"hold_ms":{"type":"integer"}},"required":["pan_deg","tilt_deg"]}},"#,
    r#"{"name":"look_at_point","description":"Aim the avatar's head at a 3D world point (right-handed coordinates with +Z forward, +X right, +Y up; units arbitrary, only direction matters). Use when an agent has world coordinates rather than pan/tilt degrees. A target at the origin is rejected as a singularity; use look_at with the neutral pose instead.","inputSchema":{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"},"z":{"type":"number"},"hold_ms":{"type":"integer"}},"required":["x","y","z"]}},"#,
    r#"{"name":"speak","description":"Play a baked phrase or chirp through the speaker.","inputSchema":{"type":"object","properties":{"phrase":{"type":"string"},"locale":{"type":"string"}},"required":["phrase"]}},"#,
    r#"{"name":"play_motion","description":"Play a canonical one-shot motion preset. Vocabulary: greet, nod, shake, laugh. Routed through the same dance-player path as POST /dance.","inputSchema":{"type":"object","properties":{"motion":{"type":"string"}},"required":["motion"]}},"#,
    r#"{"name":"push_toast","description":"Push a short on-screen toast band with warn or error severity. Requires behavior.toast_overlay_enabled in STACKCHAN.RON for the band to actually render. Three-second TTL.","inputSchema":{"type":"object","properties":{"level":{"type":"string","enum":["warn","error"]},"message":{"type":"string"}},"required":["level"]}},"#,
    r#"{"name":"start_listen","description":"Open a listen window: queue an acknowledge chirp, set Attention::Listening, arm the Ear decorator. Default 3000 ms.","inputSchema":{"type":"object","properties":{"duration_ms":{"type":"integer"}}}},"#,
    r#"{"name":"enter_pairing","description":"Open an ESP-NOW pairing window for the configured duration so an external remote can register.","inputSchema":{"type":"object","properties":{"duration_ms":{"type":"integer"}}}},"#,
    r#"{"name":"enter_thinking","description":"Show the thought-bubble decorator on the avatar's face for a hold window. Use when the host is processing on the avatar's behalf and wants the avatar to visibly read as 'thinking'. A subsequent set_emotion call clears the hold as a side effect; alternatively call exit_thinking explicitly. Default 15000 ms.","inputSchema":{"type":"object","properties":{"hold_ms":{"type":"integer"}}}},"#,
    r#"{"name":"exit_thinking","description":"Release any active thinking-bubble hold without changing emotion. Symmetric counterpart to enter_thinking; pairs with set_emotion as the two ways to clear the bubble.","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"set_volume","description":"Set the speaker volume in percent (0..=100). Persists to STACKCHAN.RON; survives reboot.","inputSchema":{"type":"object","properties":{"level":{"type":"integer","minimum":0,"maximum":100}},"required":["level"]}},"#,
    r#"{"name":"set_mute","description":"Mute or unmute the speaker without losing the persisted volume level. Persists to STACKCHAN.RON.","inputSchema":{"type":"object","properties":{"muted":{"type":"boolean"}},"required":["muted"]}},"#,
    r#"{"name":"create_reminder","description":"Schedule a baked phrase to play in N seconds. Returns the reminder id. Runtime-only — does not survive reboot.","inputSchema":{"type":"object","properties":{"fire_in_secs":{"type":"integer","minimum":1,"maximum":432000},"phrase":{"type":"string"}},"required":["fire_in_secs","phrase"]}},"#,
    r#"{"name":"list_reminders","description":"Return the currently-scheduled reminders.","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"cancel_reminder","description":"Cancel a previously-scheduled reminder by id. Returns 404 / not-found if no matching reminder exists.","inputSchema":{"type":"object","properties":{"id":{"type":"integer"}},"required":["id"]}},"#,
    r#"{"name":"take_photo","description":"Trigger the camera to capture the next frame and write it to /sd/CAPTURE.565. The frame is then fetchable via GET /camera/snapshot as raw 320x240 RGB565 big-endian bytes (X-Frame-Format / X-Frame-Width / X-Frame-Height response headers describe the layout). Returns the snapshot URL and dimensions.","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"sleep","description":"Enter sleep mode: eyes shut, head limp, LED ring dark, audio TX paused. Wake via the wake tool, any touch on the screen or body-touch pads, or the side power button. Sleep state resets on reboot.","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"wake","description":"Exit sleep mode and resume the live face / head / LED state.","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"get_state","description":"Return the current avatar snapshot (emotion, mood, head pose, decorator, battery, Wi-Fi, audio).","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"get_health","description":"Return the firmware's liveness snapshot: uptime in milliseconds since boot, build version string, and free PSRAM heap in bytes. Use to confirm kai is up, see how long it has been running, and gauge memory pressure before issuing a memory-allocating tool call (e.g. play_dance with a long keyframe array).","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"get_sensors","description":"Return the latest sensor snapshot: IMU accel/gyro (m/s² and °/s), ambient lux, audio RMS, touch state, body-touch zones, camera tracking observation. Use to ground a follow-up tool call in current physical context.","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"get_tasks","description":"Return watchdog task health: per-task heartbeat ages and channel slot counts. Use to diagnose a misbehaving avatar or to confirm that a subsystem (audio, head, BLE) is healthy before issuing a command that depends on it.","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"get_events","description":"Return the recent operator-visible event ring: lifecycle transitions, control-plane writes, warnings. Each entry carries a timestamp, kind, and message. Use to surface context for what the avatar has been doing.","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"get_crash","description":"Return the most recent panic log captured by the boot path from the persistent RTC crash latch. Returns an empty string when no crash has been recorded since the last clear. Requires an SD card mounted at /sd; an InternalError with detail 'no SD card mounted' is returned otherwise. Use to diagnose a recent reboot.","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"set_palette","description":"Switch the avatar's runtime colour palette. Vocabulary: default, dark, cute, dog. Affects the four 'skin' colours of the avatar while symbolic overlays (heart, sweat, dizzy, etc.) keep their dedicated colours. Persists across reboots via the runtime store.","inputSchema":{"type":"object","properties":{"palette":{"type":"string"}},"required":["palette"]}},"#,
    r#"{"name":"set_face_target","description":"Aim the avatar's head at a normalised 2D frame target (x and y in [-1.0, 1.0]; (0,0) is the camera centre; positive x is right, positive y is down — the screen-space convention every CV face/pose detector emits). So (-1,-1) is top-left and (1,1) is bottom-right. Use when an agent has already mapped a target into camera-frame coordinates. For pan/tilt degrees use look_at instead.","inputSchema":{"type":"object","properties":{"x":{"type":"number","minimum":-1,"maximum":1},"y":{"type":"number","minimum":-1,"maximum":1},"hold_ms":{"type":"integer"}},"required":["x","y"]}},"#,
    r#"{"name":"set_camera_mode","description":"Toggle the LCD's display mode between avatar view (false) and live camera passthrough (true). Display-only — face tracking continues in either mode. Ephemeral: a reboot returns to avatar view regardless.","inputSchema":{"type":"object","properties":{"enabled":{"type":"boolean"}},"required":["enabled"]}},"#,
    r#"{"name":"get_head_offsets","description":"Return the operator-applied head zero-point trim (yaw and tilt, degrees). Use to read back what set_head_offsets persisted, or to diagnose a head that's pointing slightly off-centre.","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"set_head_offsets","description":"Adjust the head zero-point trim in degrees. Applied additively to every commanded pose so an avatar that mounts slightly off-axis can be re-aimed without re-calibrating each motion. Persists across reboots via the runtime store. Typical correction range: ±10 degrees on each axis.","inputSchema":{"type":"object","properties":{"yaw_offset_deg":{"type":"number"},"tilt_offset_deg":{"type":"number"}},"required":["yaw_offset_deg","tilt_offset_deg"]}},"#,
    r#"{"name":"set_behavior_flag","description":"Toggle one runtime-mutable boolean flag persisted in /sd/STACKCHAN.RON. Vocabulary: soliloquy_enabled (idle bubbles), hourly_chime_enabled (top-of-hour chirp), battery_icon_enabled (on-screen battery indicator), toast_overlay_enabled (warn/error band at the bottom of the LCD). Takes effect on the next render tick — no reboot. Wake-word and mDNS flags are NOT included because they're read once at task spawn; use PUT /settings to change those. Requires an SD card mounted at /sd; an InternalError with detail 'no SD card mounted' is returned otherwise.","inputSchema":{"type":"object","properties":{"field":{"type":"string","enum":["soliloquy_enabled","hourly_chime_enabled","battery_icon_enabled","toast_overlay_enabled"]},"value":{"type":"boolean"}},"required":["field","value"]}},"#,
    r#"{"name":"clear_crash","description":"Delete the persistent crash log at /sd/CRASH.LOG so subsequent get_crash calls return an empty result. Idempotent: returns the same success whether a log was actually deleted or none existed — call get_crash first if you need to confirm a real log was cleared. Requires an SD card mounted at /sd; an InternalError with detail 'no SD card mounted' is returned otherwise.","inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"play_dance","description":"Play an arbitrary dance script — a sequence of keyframes that step emotion / head pose / LED colour / decorator at scheduled offsets from the start of playback. Each keyframe carries a required at_ms (milliseconds from script start) plus any subset of optional channels: emotion (string), pan_deg + tilt_deg (numbers, degrees), r + g + b (integers 0-255, the LED ring colour — all three required if any is set), decorator (string). Channels not set on a keyframe inherit the most-recent prior value. Use to compose a one-shot custom gesture; for canonical greet/nod/shake/laugh use play_motion instead.","inputSchema":{"type":"object","properties":{"keyframes":{"type":"array","items":{"type":"object","properties":{"at_ms":{"type":"integer","minimum":0},"emotion":{"type":"string"},"pan_deg":{"type":"number"},"tilt_deg":{"type":"number"},"r":{"type":"integer","minimum":0,"maximum":255},"g":{"type":"integer","minimum":0,"maximum":255},"b":{"type":"integer","minimum":0,"maximum":255},"decorator":{"type":"string"}},"required":["at_ms"]}}},"required":["keyframes"]}},"#,
    r#"{"name":"reset","description":"Release any active emotion / look-at / listening / thinking holds and return to autonomous behavior. Distinct from sleep — the avatar stays awake; it just lets go of operator-set state.","inputSchema":{"type":"object","properties":{}}}"#,
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
    fn parse_request_rejects_duplicate_top_level_keys() {
        // The envelope's four top-level keys each accept one value;
        // a body that shadows them is a sloppy or hostile client.
        // Closed-schema rejection is consistent with the rest of
        // the parser family (parse_set_emotion / parse_toast / ...).
        for (body, label) in [
            (
                r#"{"jsonrpc":"2.0","jsonrpc":"2.0","id":1,"method":"x"}"#,
                "jsonrpc",
            ),
            (r#"{"jsonrpc":"2.0","id":1,"id":2,"method":"x"}"#, "id"),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"x","method":"y"}"#,
                "method",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"x","params":{},"params":{}}"#,
                "params",
            ),
        ] {
            let err = parse_request(body).expect_err(label);
            assert_eq!(
                err.code,
                JsonRpcErrorCode::InvalidRequest,
                "{label}: {err:?}"
            );
        }
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

    // ============================================================
    // JsonRpcErrorCode — `as_i32` / `message` were partially covered;
    // the InvalidRequest / InvalidParams / InternalError arms had no
    // direct tests despite firing in real error paths.
    // ============================================================

    #[test]
    fn jsonrpc_error_code_numeric_values() {
        assert_eq!(JsonRpcErrorCode::ParseError.as_i32(), -32_700);
        assert_eq!(JsonRpcErrorCode::InvalidRequest.as_i32(), -32_600);
        assert_eq!(JsonRpcErrorCode::MethodNotFound.as_i32(), -32_601);
        assert_eq!(JsonRpcErrorCode::InvalidParams.as_i32(), -32_602);
        assert_eq!(JsonRpcErrorCode::InternalError.as_i32(), -32_603);
    }

    #[test]
    fn jsonrpc_error_code_messages() {
        assert_eq!(JsonRpcErrorCode::ParseError.message(), "Parse error");
        assert_eq!(
            JsonRpcErrorCode::InvalidRequest.message(),
            "Invalid Request"
        );
        assert_eq!(
            JsonRpcErrorCode::MethodNotFound.message(),
            "Method not found"
        );
        assert_eq!(JsonRpcErrorCode::InvalidParams.message(), "Invalid params");
        assert_eq!(JsonRpcErrorCode::InternalError.message(), "Internal error");
    }

    // ============================================================
    // parse_request error paths.
    // ============================================================

    #[test]
    fn parse_request_rejects_non_object_top_level() {
        // Bare array, scalar, etc.
        for body in ["[]", "42", "\"x\"", "null"] {
            let err = parse_request(body).expect_err("non-object top-level should reject");
            assert_eq!(err.code, JsonRpcErrorCode::ParseError);
        }
    }

    #[test]
    fn parse_request_rejects_unterminated_object() {
        // No closing `}`. The loop walks past the last key/value and
        // hits EOF.
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"x""#;
        let err = parse_request(body).expect_err("missing `}` should reject");
        assert_eq!(err.code, JsonRpcErrorCode::ParseError);
    }

    #[test]
    fn parse_request_rejects_missing_comma_between_members() {
        // `"id":1` followed by `"method":...` with no comma. The
        // second iteration of the loop expects `,` and bails.
        let body = r#"{"jsonrpc":"2.0" "id":1,"method":"x"}"#;
        let err = parse_request(body).expect_err("missing `,` should reject");
        assert_eq!(err.code, JsonRpcErrorCode::ParseError);
    }

    #[test]
    fn parse_request_rejects_missing_colon_after_key() {
        let body = r#"{"jsonrpc" "2.0","id":1,"method":"x"}"#;
        let err = parse_request(body).expect_err("missing `:` should reject");
        assert_eq!(err.code, JsonRpcErrorCode::ParseError);
    }

    #[test]
    fn parse_request_rejects_unknown_top_level_key() {
        // Anything outside {jsonrpc, id, method, params}.
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"x","extra":"y"}"#;
        let err = parse_request(body).expect_err("unknown key should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidRequest);
    }

    #[test]
    fn parse_request_rejects_wrong_jsonrpc_string_type() {
        // `jsonrpc` must be a string value; an integer doesn't even
        // make it past read_string and lands in ParseError.
        let body = r#"{"jsonrpc":2,"id":1,"method":"x"}"#;
        let err = parse_request(body).expect_err("non-string jsonrpc should reject");
        assert_eq!(err.code, JsonRpcErrorCode::ParseError);
    }

    #[test]
    fn parse_request_rejects_non_integer_id() {
        let body = r#"{"jsonrpc":"2.0","id":"oops","method":"x"}"#;
        let err = parse_request(body).expect_err("string id should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidRequest);
    }

    #[test]
    fn parse_request_rejects_overflowed_id() {
        // `i64::MAX + 1` — out of `i64` range.
        let body = r#"{"jsonrpc":"2.0","id":9223372036854775808,"method":"x"}"#;
        let err = parse_request(body).expect_err("overflow should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidRequest);
    }

    #[test]
    fn parse_request_accepts_array_params() {
        // params can be any JSON value; `take_value` walks until
        // balanced, so an array works as well as an object. The
        // captured slice still re-parses for downstream use.
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"x","params":[1,2,3]}"#;
        let req = parse_request(body).expect("array params should parse");
        assert_eq!(req.params_raw, Some("[1,2,3]"));
    }

    #[test]
    fn parse_request_accepts_scalar_params() {
        // Likewise scalars: tests the `_` arm of `take_value`.
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"x","params":42}"#;
        let req = parse_request(body).expect("scalar params should parse");
        assert_eq!(req.params_raw, Some("42"));
    }

    #[test]
    fn parse_request_rejects_unbalanced_params() {
        // Missing closing `]` on the params array.
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"x","params":[1,2"#;
        let err = parse_request(body).expect_err("unbalanced params should reject");
        assert_eq!(err.code, JsonRpcErrorCode::ParseError);
    }

    // ============================================================
    // read_string error paths via parse_request keys.
    // ============================================================

    #[test]
    fn parse_request_rejects_non_string_key() {
        // Integer where a key string should be. read_string returns
        // "expected string".
        let body = r#"{"jsonrpc":"2.0","id":1,42:"x"}"#;
        let err = parse_request(body).expect_err("non-string key should reject");
        assert_eq!(err.code, JsonRpcErrorCode::ParseError);
    }

    #[test]
    fn parse_request_rejects_dangling_string_escape() {
        // `\` at the end of input — read_string's dangling-escape arm.
        let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"x\\";
        let err = parse_request(body).expect_err("dangling escape should reject");
        assert_eq!(err.code, JsonRpcErrorCode::ParseError);
    }

    #[test]
    fn parse_request_rejects_unsupported_string_escape() {
        // `\u` — we don't support unicode escapes in the MCP parser
        // (the module docs call this out explicitly).
        let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"\\u00C0\"}";
        let err = parse_request(body).expect_err("\\u escape should reject");
        assert_eq!(err.code, JsonRpcErrorCode::ParseError);
    }

    #[test]
    fn parse_request_accepts_supported_string_escapes() {
        // Every escape from the read_string match arm: \" \\ \/ \n \r \t \b \f.
        let body =
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"a\\\"b\\\\c\\/d\\ne\\rf\\tg\\bh\\fi\"}";
        let req = parse_request(body).expect("all supported escapes should parse");
        // The slice is borrowed unescaped — that's fine, the consumer
        // does its own un-escaping when needed.
        assert!(req.method.contains("\\\""));
        assert!(req.method.contains("\\n"));
    }

    #[test]
    fn parse_request_rejects_unterminated_string() {
        // String key without closing `"` — read_string's tail "unterminated string" return.
        let body = "{\"jsonrpc\":\"2.0\",\"id";
        let err = parse_request(body).expect_err("unterminated string should reject");
        assert_eq!(err.code, JsonRpcErrorCode::ParseError);
    }

    // ============================================================
    // find_string_field / find_object_field error paths.
    // ============================================================

    #[test]
    fn find_string_field_rejects_non_object() {
        let err = find_string_field("not an object", "name").expect_err("non-object should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidParams);
    }

    #[test]
    fn find_string_field_rejects_unterminated_object() {
        // Searched key doesn't match the present one, so the loop
        // walks past + hits EOF without finding `}`.
        let err = find_string_field(r#"{"name":"x""#, "missing")
            .expect_err("unterminated object should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidParams);
    }

    #[test]
    fn find_string_field_rejects_missing_comma() {
        let err = find_string_field(r#"{"a":"1" "b":"2"}"#, "b")
            .expect_err("missing comma should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidParams);
    }

    #[test]
    fn find_string_field_rejects_missing_colon() {
        let err = find_string_field(r#"{"a" "1"}"#, "a").expect_err("missing colon should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidParams);
    }

    #[test]
    fn find_string_field_rejects_non_string_value_for_target_key() {
        // Key matches but value isn't a string.
        let err = find_string_field(r#"{"name":42}"#, "name")
            .expect_err("non-string value should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidParams);
    }

    #[test]
    fn find_string_field_rejects_unbalanced_skipped_value() {
        // Key doesn't match; we have to skip the value. If that value
        // is malformed, the InvalidParams skip-fail path fires.
        let err = find_string_field(r#"{"other":[1,2,"name":"x"}"#, "name")
            .expect_err("unbalanced skipped value should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidParams);
    }

    #[test]
    fn find_object_field_returns_none_when_absent() {
        assert_eq!(
            find_object_field(r#"{"other":"x"}"#, "arguments").unwrap(),
            None
        );
    }

    #[test]
    fn find_object_field_rejects_non_object() {
        let err = find_object_field("[1,2]", "arguments").expect_err("non-object should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidParams);
    }

    #[test]
    fn find_object_field_rejects_unterminated_object() {
        let err = find_object_field(r#"{"arguments":{"a":1}"#, "missing")
            .expect_err("unterminated object should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidParams);
    }

    #[test]
    fn find_object_field_rejects_missing_comma() {
        let err =
            find_object_field(r#"{"a":1 "b":2}"#, "b").expect_err("missing comma should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidParams);
    }

    #[test]
    fn find_object_field_rejects_missing_colon() {
        let err = find_object_field(r#"{"a" 1}"#, "a").expect_err("missing colon should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidParams);
    }

    #[test]
    fn find_object_field_rejects_malformed_target_value() {
        // Key matches but value is unbalanced.
        let err = find_object_field(r#"{"arguments":{"a":1"#, "arguments")
            .expect_err("malformed target value should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidParams);
    }

    #[test]
    fn find_object_field_rejects_malformed_skipped_value() {
        // Key doesn't match; the value we're skipping is unbalanced.
        let err = find_object_field(r#"{"other":[1,2,"arguments":{}}"#, "arguments")
            .expect_err("malformed skipped value should reject");
        assert_eq!(err.code, JsonRpcErrorCode::InvalidParams);
    }

    // ============================================================
    // escape_string — `\r`, `\t`, and the control-char branch.
    // ============================================================

    #[test]
    fn escape_string_handles_carriage_return_and_tab() {
        assert_eq!(escape_string("a\rb\tc"), "a\\rb\\tc");
    }

    #[test]
    fn escape_string_emits_unicode_escape_for_control_chars() {
        // U+0001 (SOH) — falls through to the `\\u{:04x}` branch.
        let input = "\u{0001}";
        assert_eq!(escape_string(input), "\\u0001");
    }

    #[test]
    fn escape_string_passes_through_non_ascii() {
        // Non-ASCII UTF-8 bytes (e.g. CJK / emoji) aren't escape-
        // candidates — they pass through unchanged.
        assert_eq!(escape_string("日本😀"), "日本😀");
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
