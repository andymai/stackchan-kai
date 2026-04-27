//! Tiny hand-rolled HTTP/1.1 server for the LAN-scoped control plane.
//!
//! ## Routes
//!
//! - `GET /health` — uptime, firmware version, free heap (handy for
//!   liveness checks and post-flash smoke tests).
//! - `GET /state` — `AvatarSnapshot` JSON read non-destructively
//!   from `super::snapshot`.
//! - `POST /emotion` — JSON `{"emotion": "...", "hold_ms": ...}`.
//!   Sets affect + holds `mind.autonomy` against the autonomous
//!   emotion drivers for `hold_ms` (default
//!   [`super::json::DEFAULT_HOLD_MS`]).
//! - `POST /look-at` — JSON `{"pan_deg": f32, "tilt_deg": f32, "hold_ms": ...}`.
//!   Sets `mind.attention = Tracking { target }` for `hold_ms`,
//!   asserting the operator's target against camera tracking.
//! - `POST /reset` — empty body. Clears any active emotion or
//!   look-at hold and returns the avatar to autonomous behaviour.
//!
//! All write routes funnel through [`REMOTE_COMMAND_SIGNAL`]; the
//! render task drains the signal into
//! `entity.input.remote_command` ahead of `Director::run`, where
//! [`stackchan_core::modifiers::RemoteCommandModifier`] picks it up.
//!
//! No external HTTP crate. The wire format is small, the surface
//! is fixed, and a hand-roll dodges the impl-trait-in-assoc-type
//! requirement that picoserve's `AppBuilder` brings in.

use alloc::format;
use alloc::string::String;

use embassy_net::Stack;
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Duration;
use embedded_io_async::Write as AsyncWrite;
use stackchan_core::RemoteCommand;

use super::json::{self, JsonError};
use super::snapshot::{self, AvatarSnapshot};
use super::wifi::{WIFI_LINK_SIGNAL, WifiLinkState};

/// Listening port. LAN-only; no auth.
const HTTP_PORT: u16 = 80;

/// Maximum request line + headers + body size we'll buffer before
/// responding `400`. POST bodies for the current write surface are
/// at most a few dozen bytes; 1 KiB is generous.
const REQUEST_BUF_BYTES: usize = 1024;

/// Cap on the `Content-Length` header. Anything larger is rejected
/// before any body bytes are read — the write surface only accepts
/// short JSON object bodies.
const MAX_BODY_BYTES: usize = 256;

/// Latest control-plane command.
///
/// Set by the HTTP task on a successful POST; drained by the render
/// task into `entity.input.remote_command` before `Director::run`.
/// Latest-wins semantics — a second POST that lands before the
/// render task drains will overwrite the first.
pub static REMOTE_COMMAND_SIGNAL: Signal<CriticalSectionRawMutex, RemoteCommand> = Signal::new();

/// Embassy task — owns one TCP socket and serves requests one at a time.
/// Re-binds the socket per accept so a misbehaving client can't wedge
/// the listener.
#[embassy_executor::task]
pub async fn http_task(stack: Stack<'static>) -> ! {
    let mut rx_buf = [0u8; 1024];
    let mut tx_buf = [0u8; 2048];

    // Wait for the wifi task to publish a Connected state at least
    // once. After that, every accept loop just keeps trying — embassy-net
    // returns errors quickly when the link is down, no busy spin.
    loop {
        if matches!(WIFI_LINK_SIGNAL.wait().await, WifiLinkState::Connected) {
            break;
        }
    }

    defmt::info!(
        "http: listening on 0.0.0.0:{=u16} (LAN-only, no auth)",
        HTTP_PORT
    );

    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);
        socket.set_timeout(Some(Duration::from_secs(10)));

        if let Err(e) = socket.accept(HTTP_PORT).await {
            defmt::warn!("http: accept failed ({:?})", e);
            continue;
        }

        if let Err(e) = serve_one(&mut socket).await {
            defmt::warn!("http: serve error ({})", defmt::Debug2Format(&e));
        }

        socket.close();
        // Allow the peer time to read the FIN before we re-bind.
        embassy_time::Timer::after(Duration::from_millis(50)).await;
    }
}

/// Lightweight error wrapping for the request handler — ferries
/// socket and parse failures to a single `warn` log line at the
/// accept loop.
#[derive(Debug, defmt::Format)]
enum HttpError {
    /// Socket read returned an error or EOF before the request was
    /// complete.
    Read,
    /// Socket write returned an error mid-response.
    Write,
    /// Header section never closed within `REQUEST_BUF_BYTES`.
    HeadersTooLarge,
    /// `Content-Length` exceeded [`MAX_BODY_BYTES`] or wasn't a valid
    /// non-negative integer.
    BodyTooLarge,
    /// Method + path didn't parse as a valid HTTP request line.
    Malformed,
}

/// Serve a single HTTP/1.1 exchange against an accepted socket.
async fn serve_one(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let mut buf = [0u8; REQUEST_BUF_BYTES];
    let mut filled = 0usize;
    // Read until we see `\r\n\r\n` or hit the cap.
    let header_end = loop {
        if filled >= REQUEST_BUF_BYTES {
            return Err(HttpError::HeadersTooLarge);
        }
        match socket.read(&mut buf[filled..]).await {
            Ok(n) if n > 0 => filled += n,
            // Zero-byte read = EOF before headers landed; same outcome
            // as a transport error from the caller's perspective.
            _ => return Err(HttpError::Read),
        }
        if let Some(idx) = find_subsequence(&buf[..filled], b"\r\n\r\n") {
            break idx;
        }
    };

    // Parse request-line bounds. Capture method/path as `(start, end)`
    // ranges instead of `&str` borrows so the borrow on `buf` ends
    // before the body read needs `&mut buf`.
    let line_end = buf[..filled]
        .windows(2)
        .position(|w| w == b"\r\n")
        .ok_or(HttpError::Malformed)?;
    let first_sp = buf[..line_end]
        .iter()
        .position(|&b| b == b' ')
        .ok_or(HttpError::Malformed)?;
    let path_start = first_sp + 1;
    let second_sp = buf[path_start..line_end]
        .iter()
        .position(|&b| b == b' ')
        .ok_or(HttpError::Malformed)?
        + path_start;
    let body_start = header_end + 4;
    let content_length = parse_content_length(&buf[line_end + 2..header_end])?;
    if content_length > MAX_BODY_BYTES {
        return Err(HttpError::BodyTooLarge);
    }
    while filled < body_start + content_length {
        if filled >= REQUEST_BUF_BYTES {
            return Err(HttpError::BodyTooLarge);
        }
        match socket.read(&mut buf[filled..]).await {
            Ok(n) if n > 0 => filled += n,
            _ => return Err(HttpError::Read),
        }
    }

    let method = core::str::from_utf8(&buf[..first_sp]).map_err(|_| HttpError::Malformed)?;
    let path =
        core::str::from_utf8(&buf[path_start..second_sp]).map_err(|_| HttpError::Malformed)?;
    let body = core::str::from_utf8(&buf[body_start..body_start + content_length])
        .map_err(|_| HttpError::Malformed)?;

    match (method, path) {
        ("GET", "/health") => write_json(socket, 200, &health_body()).await,
        ("GET", "/state") => write_json(socket, 200, &state_body(snapshot::read())).await,
        ("POST", "/emotion") => handle_remote(socket, json::parse_set_emotion(body)).await,
        ("POST", "/look-at") => handle_remote(socket, json::parse_look_at(body)).await,
        ("POST", "/reset") => handle_remote(socket, Ok(RemoteCommand::Reset)).await,
        ("GET" | "POST", _) => write_text(socket, 404, "not found\n").await,
        _ => write_text(socket, 405, "method not allowed\n").await,
    }
}

/// Apply a parsed remote command (or surface the parser error to the
/// client). On success: signal the render task and respond `204 No
/// Content`. On parse failure: respond `400 Bad Request` with a
/// short plain-text reason.
async fn handle_remote(
    socket: &mut TcpSocket<'_>,
    command: Result<RemoteCommand, JsonError>,
) -> Result<(), HttpError> {
    match command {
        Ok(cmd) => {
            defmt::info!("http: remote command {}", defmt::Debug2Format(&cmd));
            REMOTE_COMMAND_SIGNAL.signal(cmd);
            write_text(socket, 204, "").await
        }
        Err(e) => {
            defmt::warn!("http: bad request body ({})", e);
            let body = format!("invalid request body: {e:?}\n");
            write_text(socket, 400, &body).await
        }
    }
}

/// Parse the `Content-Length` header value out of a header block.
/// Returns `0` when absent (correct for GET / 0-body POST). Returns
/// [`HttpError::Malformed`] if the header is present but not a valid
/// non-negative integer.
fn parse_content_length(headers: &[u8]) -> Result<usize, HttpError> {
    for line in headers.split(|&b| b == b'\n') {
        // Trim trailing CR.
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some((name, value)) = split_once(line, b':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case(b"content-length") {
            continue;
        }
        let value = trim_ascii(value);
        let s = core::str::from_utf8(value).map_err(|_| HttpError::Malformed)?;
        return s.parse::<usize>().map_err(|_| HttpError::Malformed);
    }
    Ok(0)
}

/// Index of the first occurrence of `needle` in `haystack`, or `None`.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Split `slice` at the first occurrence of `delim`. Returns `None`
/// if the delimiter is absent.
fn split_once(slice: &[u8], delim: u8) -> Option<(&[u8], &[u8])> {
    let idx = slice.iter().position(|&b| b == delim)?;
    Some((&slice[..idx], &slice[idx + 1..]))
}

/// Strip ASCII whitespace from both ends of `slice`.
fn trim_ascii(slice: &[u8]) -> &[u8] {
    let start = slice
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(slice.len());
    let end = slice
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |i| i + 1);
    &slice[start..end]
}

/// Serialise the `/health` body. Schema is a flat object — no nested
/// types, so a small `format!` keeps the dep surface clean.
fn health_body() -> String {
    let uptime_ms = embassy_time::Instant::now().as_millis();
    let version = env!("CARGO_PKG_VERSION");
    format!(
        "{{\"uptime_ms\":{uptime_ms},\"version\":\"{version}\",\"free_heap_bytes\":{free}}}\n",
        free = esp_alloc::HEAP.free(),
    )
}

/// Serialise the `/state` body from a snapshot read. The HTTP layer
/// owns the JSON shape; downstream consumers can rely on it without
/// pulling stackchan-net into the response path.
fn state_body(s: AvatarSnapshot) -> String {
    let pct = s
        .battery
        .percent
        .map_or_else(|| String::from("null"), |p| format!("{p}"));
    let mv = s
        .battery
        .voltage_mv
        .map_or_else(|| String::from("null"), |m| format!("{m}"));
    let actual = s.head_actual.map_or_else(
        || String::from("null"),
        |p| {
            format!(
                "{{\"pan_deg\":{:.2},\"tilt_deg\":{:.2}}}",
                p.pan_deg, p.tilt_deg
            )
        },
    );
    let ip = s
        .wifi
        .ip
        .map_or_else(|| String::from("null"), |a| format!("\"{a}\""));
    format!(
        "{{\
\"emotion\":\"{emotion:?}\",\
\"head_pose\":{{\"pan_deg\":{pan:.2},\"tilt_deg\":{tilt:.2}}},\
\"head_actual\":{actual},\
\"battery\":{{\"percent\":{pct},\"voltage_mv\":{mv}}},\
\"wifi\":{{\"connected\":{connected},\"ip\":{ip}}}\
}}\n",
        emotion = s.emotion,
        pan = s.head_pose.pan_deg,
        tilt = s.head_pose.tilt_deg,
        connected = s.wifi.connected,
    )
}

/// Write `status` + `body` as `application/json`.
async fn write_json(socket: &mut TcpSocket<'_>, status: u16, body: &str) -> Result<(), HttpError> {
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n",
        reason = status_reason(status),
        len = body.len(),
    );
    socket
        .write_all(header.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;
    socket
        .write_all(body.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;
    socket.flush().await.map_err(|_| HttpError::Write)
}

/// Plain-text response, used for non-JSON paths (errors, `204`).
async fn write_text(socket: &mut TcpSocket<'_>, status: u16, body: &str) -> Result<(), HttpError> {
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n",
        reason = status_reason(status),
        len = body.len(),
    );
    socket
        .write_all(header.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;
    socket
        .write_all(body.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;
    socket.flush().await.map_err(|_| HttpError::Write)
}

/// Mini status-reason table — only the codes this server emits.
const fn status_reason(status: u16) -> &'static str {
    match status {
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        // 200 + everything else fall through; the server only emits
        // codes from this short list, so anything else here is a
        // programming bug.
        _ => "OK",
    }
}
