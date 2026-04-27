//! Tiny hand-rolled HTTP/1.1 server for the LAN-scoped control plane.
//!
//! ## Routes
//!
//! - `GET /` — operator dashboard. Self-contained HTML + JS embedded
//!   at compile time via `include_bytes!`. Drives the live state via
//!   `/state/stream` and POSTs to the control routes.
//! - `GET /health` — uptime, firmware version, free heap (handy for
//!   liveness checks and post-flash smoke tests).
//! - `GET /state` — `AvatarSnapshot` JSON read non-destructively
//!   from `super::snapshot`.
//! - `GET /state/stream` — Server-Sent Events stream of
//!   `AvatarSnapshot` updates. Sends an initial event on connect,
//!   then one event per change (throttled at the producer to ~10 Hz),
//!   plus a `: heartbeat` SSE comment every 15 s.
//! - `POST /emotion` — JSON `{"emotion": "...", "hold_ms": ...}`.
//!   Sets affect + holds `mind.autonomy` against the autonomous
//!   emotion drivers for `hold_ms` (default
//!   [`stackchan_net::http_command::DEFAULT_HOLD_MS`]).
//! - `POST /look-at` — JSON `{"pan_deg": f32, "tilt_deg": f32, "hold_ms": ...}`.
//!   Sets `mind.attention = Tracking { target }` for `hold_ms`,
//!   asserting the operator's target against camera tracking.
//! - `POST /reset` — empty body. Clears any active emotion or
//!   look-at hold and returns the avatar to autonomous behaviour.
//! - `POST /speak` — JSON `{"phrase": "...", "locale": "..."}`.
//!   Renders a [`stackchan_core::voice::PhraseId`] from the baked
//!   catalog and queues it on the audio TX path. Fire-and-forget;
//!   no avatar-state hold timer.
//! - `GET /settings` — current persisted [`stackchan_net::Config`]
//!   as JSON, with `wifi.psk` and `auth.token` redacted.
//! - `PUT /settings` — full-replace [`stackchan_net::Config`] body.
//!   Validates, writes back atomically to `/sd/STACKCHAN.RON`, and
//!   responds `{"reboot_required": true}`. Wi-Fi keeps using the
//!   boot-time config; reboot to apply.
//!
//! ## Auth
//!
//! `PUT` and `POST` routes are gated by `auth.token` from the
//! persisted config. Empty token (default) leaves the LAN open;
//! a non-empty token requires `Authorization: Bearer <token>` and
//! returns `401` on mismatch. Read routes stay unauthenticated.
//!
//! Avatar-state writes (POST /emotion, /look-at, /reset) funnel
//! through [`REMOTE_COMMAND_SIGNAL`]; the render task drains it
//! into `entity.input.remote_command` ahead of `Director::run`,
//! where [`stackchan_core::modifiers::RemoteCommandModifier`] picks
//! it up. PUT /settings goes through
//! [`crate::storage::with_storage`] for the atomic SD writeback;
//! the new value is mirrored into [`crate::storage::CONFIG_SNAPSHOT`]
//! so subsequent GETs see it without a re-read.
//!
//! No external HTTP crate. The wire format is small, the surface
//! is fixed, and a hand-roll dodges the impl-trait-in-assoc-type
//! requirement that picoserve's `AppBuilder` brings in.

use alloc::format;
use alloc::string::String;

use embassy_futures::select::{Either, select};
use embassy_net::Stack;
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::WaitResult;
use embassy_sync::signal::Signal;
use embassy_time::Duration;
use embedded_io_async::Write as AsyncWrite;
use stackchan_core::RemoteCommand;
use stackchan_net::http_command::{self as json, JsonError};
use stackchan_net::http_parse::{
    ct_eq, find_subsequence, parse_bearer_token, parse_content_length,
};

use super::snapshot::{self, AvatarSnapshot};
use super::wifi::LINK_READY;

/// Listening port. LAN-only; write routes are gated on the
/// configured `auth.token` (empty = no auth).
const HTTP_PORT: u16 = 80;

/// Maximum request line + headers + body size we'll buffer before
/// responding `400`. Headers and body share this buffer, so the
/// late-stage `filled >= REQUEST_BUF_BYTES` guard doubles as a
/// header-overflow check.
const REQUEST_BUF_BYTES: usize = 1024;

/// Cap on the `Content-Length` header. Bodies of this size or
/// larger are rejected before any body bytes are read.
///
/// Equal to [`REQUEST_BUF_BYTES`] on purpose: the buffer holds
/// headers + body together, so any `content_length` that hits the
/// cap can't physically fit alongside the request line. Sized for
/// `PUT /settings`: the full schema-v1 body with a 32-char SSID,
/// 63-char WPA2 PSK, an `America/…` IANA tz label, and a few SNTP
/// servers lands around 320 bytes; the 1024 ceiling leaves room
/// for future fields without forcing every operator update through
/// a re-cap.
const MAX_BODY_BYTES: usize = 1024;

/// Self-contained operator dashboard, embedded at compile time.
/// Loaded by `GET /` at the device root; uses the existing
/// SSE / POST / PUT routes for live state and control.
const DASHBOARD_HTML: &[u8] = include_bytes!("dashboard.html");

/// Latest control-plane command.
///
/// Set by the HTTP task on a successful POST; drained by the render
/// task into `entity.input.remote_command` before `Director::run`.
/// Latest-wins semantics — a second POST that lands before the
/// render task drains will overwrite the first.
pub static REMOTE_COMMAND_SIGNAL: Signal<CriticalSectionRawMutex, RemoteCommand> = Signal::new();

/// Number of concurrent HTTP worker tasks. Each worker holds its own
/// rx/tx buffers and accepts one connection at a time.
///
/// Sized for: one long-lived `GET /state/stream` SSE client + a few
/// short-lived requests in parallel. Bumping this requires a matching
/// bump to [`super::snapshot::SSE_MAX_SUBSCRIBERS`] (each worker can
/// hold one SSE subscriber at a time).
pub const HTTP_WORKER_COUNT: usize = 4;

/// Embassy worker task — one TCP socket per worker, accepts a
/// connection, serves it, then loops back for the next accept.
/// `pool_size` provides [`HTTP_WORKER_COUNT`] independent instances
/// so multiple clients (including a long-lived SSE stream) can run
/// in parallel.
///
/// Each worker's rx/tx buffers live on its own task stack — bumping
/// `HTTP_WORKER_COUNT` linearly grows total buffer usage.
#[embassy_executor::task(pool_size = HTTP_WORKER_COUNT)]
pub async fn http_worker(stack: Stack<'static>) -> ! {
    let mut rx_buf = [0u8; 1024];
    let mut tx_buf = [0u8; 2048];

    // Gate on the latched LINK_READY flag — Signal::wait would race
    // between workers (single stored waker), so we poll the atomic
    // every 100 ms until the wifi task latches it on first connect.
    // After that, every accept loop just keeps trying — embassy-net
    // returns errors quickly when the link is down, no busy spin.
    while !LINK_READY.load(core::sync::atomic::Ordering::Acquire) {
        embassy_time::Timer::after(Duration::from_millis(100)).await;
    }

    defmt::info!(
        "http: worker listening on 0.0.0.0:{=u16} (LAN-only; auth gate: token-driven)",
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
            // Best-effort status reply for parse-side failures so
            // operators see `400`/`413`/`431` from curl instead of a
            // bare connection reset. `Read`/`Write` skip — the socket
            // is already broken.
            write_status_for_error(&mut socket, &e).await;
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
    /// Write route required a bearer token; the request didn't carry
    /// one or it didn't match the configured value.
    Unauthorized,
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
    let content_length =
        parse_content_length(&buf[line_end + 2..header_end]).map_err(|_| HttpError::Malformed)?;
    if content_length >= MAX_BODY_BYTES {
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

    // Gate write routes on the configured bearer token. An empty
    // token (or missing config snapshot during the brief boot
    // window) is treated as "auth disabled" — preserves the LAN-
    // open behaviour for operators who haven't opted in.
    if matches!(method, "PUT" | "POST") {
        let provided = parse_bearer_token(&buf[line_end + 2..header_end]);
        let snapshot = crate::storage::CONFIG_SNAPSHOT.lock().await;
        let authorized = match snapshot.as_ref() {
            Some(cfg) if !cfg.auth.token.is_empty() => {
                provided.is_some_and(|t| ct_eq(t.as_bytes(), cfg.auth.token.as_bytes()))
            }
            _ => true,
        };
        drop(snapshot);
        if !authorized {
            return Err(HttpError::Unauthorized);
        }
    }

    match (method, path) {
        ("GET", "/" | "/index.html") => write_dashboard(socket).await,
        ("GET", "/health") => write_json(socket, 200, &health_body()).await,
        ("GET", "/state") => write_json(socket, 200, &state_body(snapshot::read())).await,
        ("GET", "/state/stream") => handle_state_stream(socket).await,
        ("GET", "/settings") => handle_get_settings(socket).await,
        ("PUT", "/settings") => handle_put_settings(socket, body).await,
        ("POST", "/emotion") => handle_remote(socket, json::parse_set_emotion(body)).await,
        ("POST", "/look-at") => handle_remote(socket, json::parse_look_at(body)).await,
        ("POST", "/reset") => handle_remote(socket, Ok(RemoteCommand::Reset)).await,
        ("POST", "/speak") => handle_remote(socket, json::parse_speak(body)).await,
        ("GET" | "POST" | "PUT", _) => write_text(socket, 404, "not found\n").await,
        _ => write_text(socket, 405, "method not allowed\n").await,
    }
}

/// `GET /state/stream` — open an SSE stream of [`AvatarSnapshot`]
/// events. The render task publishes throttled snapshots via
/// [`super::snapshot::SNAPSHOT_PUBSUB`]; this handler subscribes,
/// emits each new snapshot as `data: {json}\n\n`, and sends a
/// `: heartbeat\n\n` SSE comment line every
/// [`SSE_HEARTBEAT_SECS`] seconds so proxies and NAT idle timers
/// don't tear the connection down.
///
/// Runs until the client disconnects or the socket times out.
/// Returns an error from the loop when the write fails — the
/// outer accept loop logs and re-binds.
async fn handle_state_stream(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let Ok(mut subscriber) = snapshot::SNAPSHOT_PUBSUB.subscriber() else {
        // All subscriber slots taken — every other worker is also
        // streaming. Refuse politely.
        return write_text(socket, 503, "stream slots exhausted\n").await;
    };

    // Disable the per-request inactivity timeout: SSE traffic is
    // server→client only, and the client doesn't speak after the
    // initial GET.
    socket.set_timeout(None);

    let header = "HTTP/1.1 200 OK\r\n\
                  Content-Type: text/event-stream\r\n\
                  Cache-Control: no-cache\r\n\
                  Connection: keep-alive\r\n\r\n";
    socket
        .write_all(header.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;

    // Initial event: send the current snapshot immediately so
    // freshly-connected clients don't have to wait for the next
    // render-tick change.
    write_event(socket, &snapshot::read()).await?;

    loop {
        match select(
            subscriber.next_message(),
            embassy_time::Timer::after(Duration::from_secs(SSE_HEARTBEAT_SECS)),
        )
        .await
        {
            Either::First(WaitResult::Message(snap)) => write_event(socket, &snap).await?,
            // `Lagged` means we missed N publishes. Skip them and
            // wait for the next — a current snapshot is more useful
            // than backfilling stale ones.
            Either::First(WaitResult::Lagged(_)) => {}
            Either::Second(()) => write_heartbeat(socket).await?,
        }
    }
}

/// SSE heartbeat interval. 15 s is a common default that keeps most
/// reverse-proxy / NAT idle timers happy without bloating LAN
/// traffic.
const SSE_HEARTBEAT_SECS: u64 = 15;

/// Write a single SSE `data: ...\n\n` event carrying the snapshot's
/// JSON encoding.
async fn write_event(socket: &mut TcpSocket<'_>, snap: &AvatarSnapshot) -> Result<(), HttpError> {
    // `state_body` returns a JSON object terminated with `\n`; SSE
    // wants a single `data: <line>` followed by a blank line, so we
    // strip the trailing newline before formatting.
    let body = state_body(*snap);
    let trimmed = body.trim_end_matches('\n');
    let event = format!("data: {trimmed}\n\n");
    socket
        .write_all(event.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;
    socket.flush().await.map_err(|_| HttpError::Write)
}

/// Write an SSE comment line (`: heartbeat\n\n`) to keep the
/// connection alive across idle stretches. Comment lines are
/// ignored by `EventSource` clients.
async fn write_heartbeat(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    socket
        .write_all(b": heartbeat\n\n")
        .await
        .map_err(|_| HttpError::Write)?;
    socket.flush().await.map_err(|_| HttpError::Write)
}

/// `GET /settings` — render the current snapshot with `wifi.psk`
/// redacted.
async fn handle_get_settings(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let snapshot = crate::storage::CONFIG_SNAPSHOT.lock().await.clone();
    let Some(config) = snapshot else {
        return write_text(socket, 503, "config snapshot unavailable\n").await;
    };
    match stackchan_net::render_settings_json(&config, true) {
        Ok(body) => write_json(socket, 200, &body).await,
        Err(_) => write_text(socket, 500, "render failed\n").await,
    }
}

/// `PUT /settings` — full replace, atomic SD writeback. Returns
/// `{"reboot_required": true}` on success; the firmware doesn't
/// re-bring-up Wi-Fi mid-flight (avoids dropping the operator's
/// session when the SSID changes).
async fn handle_put_settings(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    let parsed_config = match stackchan_net::parse_settings_json(body) {
        Ok(c) => c,
        Err(e) => {
            defmt::warn!(
                "http: PUT /settings parse failed ({})",
                defmt::Debug2Format(&e)
            );
            let body = format!("invalid request body: {e:?}\n");
            return write_text(socket, 400, &body).await;
        }
    };
    // Substitute the `***` redaction sentinel for the persisted PSK
    // and token so a dashboard form that submits unchanged secrets
    // doesn't clobber them. With no current snapshot (the brief
    // pre-storage-mount window), preserving against the default
    // empty values is a no-op — the parsed body wins.
    let snapshot_for_merge = crate::storage::CONFIG_SNAPSHOT
        .lock()
        .await
        .clone()
        .unwrap_or_default();
    let new_config = stackchan_net::merge_settings_with_current(parsed_config, &snapshot_for_merge);
    let write_result =
        crate::storage::with_storage(|storage| storage.write_config(&new_config)).await;
    match write_result {
        Some(Ok(())) => {
            defmt::info!(
                "http: PUT /settings persisted (ssid={=str} hostname={=str}.local)",
                new_config.wifi.ssid.as_str(),
                new_config.mdns.hostname.as_str()
            );
            *crate::storage::CONFIG_SNAPSHOT.lock().await = Some(new_config);
            write_json(socket, 200, "{\"reboot_required\":true}\n").await
        }
        Some(Err(e)) => {
            defmt::warn!("http: PUT /settings write failed ({})", e);
            write_text(socket, 500, "config write failed\n").await
        }
        None => {
            defmt::warn!("http: PUT /settings rejected (no SD mounted)");
            write_text(socket, 503, "no SD card mounted\n").await
        }
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
            write_no_content(socket).await
        }
        Err(e) => {
            defmt::warn!("http: bad request body ({})", defmt::Debug2Format(&e));
            let body = format!("invalid request body: {e:?}\n");
            write_text(socket, 400, &body).await
        }
    }
}

/// Write `204 No Content`. RFC 7230 says a 204 response "is always
/// terminated by the first empty line after the header fields"; the
/// general `write_text` helper would still emit `Content-Type` +
/// `Content-Length: 0`, which is pedantically allowed but unusual.
/// This helper omits both so the response is just headers + CRLF.
async fn write_no_content(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let header = "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n";
    socket
        .write_all(header.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;
    socket.flush().await.map_err(|_| HttpError::Write)
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
\"emotion\":\"{emotion}\",\
\"head_pose\":{{\"pan_deg\":{pan:.2},\"tilt_deg\":{tilt:.2}}},\
\"head_actual\":{actual},\
\"battery\":{{\"percent\":{pct},\"voltage_mv\":{mv}}},\
\"wifi\":{{\"connected\":{connected},\"ip\":{ip}}}\
}}\n",
        emotion = s.emotion.wire_str(),
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

/// Serve [`DASHBOARD_HTML`] with `Content-Type: text/html`. Cache is
/// disabled so a freshly flashed firmware's dashboard JS shows up on
/// the next reload — the payload is 10 KiB over LAN, so the saving
/// from a longer max-age was never worth the staleness it caused.
async fn write_dashboard(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {len}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        len = DASHBOARD_HTML.len(),
    );
    socket
        .write_all(header.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;
    socket
        .write_all(DASHBOARD_HTML)
        .await
        .map_err(|_| HttpError::Write)?;
    socket.flush().await.map_err(|_| HttpError::Write)
}

/// Mini status-reason table — only the codes this server emits.
const fn status_reason(status: u16) -> &'static str {
    match status {
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        // 200 + everything else fall through; the server only emits
        // codes from this short list, so anything else here is a
        // programming bug.
        _ => "OK",
    }
}

/// Best-effort: write a status response for a parse-side failure.
/// `Read`/`Write` skip — the socket is already broken so any further
/// write would just produce another `Write` error.
///
/// `Unauthorized` takes a slightly different path so the response
/// carries the `WWW-Authenticate: Bearer` challenge required by RFC
/// 6750 §3 on `401`.
async fn write_status_for_error(socket: &mut TcpSocket<'_>, err: &HttpError) {
    if matches!(err, HttpError::Unauthorized) {
        let _ = write_unauthorized(socket).await;
        return;
    }
    let (status, body) = match err {
        HttpError::Malformed => (400, "bad request\n"),
        HttpError::BodyTooLarge => (413, "payload too large\n"),
        HttpError::HeadersTooLarge => (431, "request header fields too large\n"),
        HttpError::Read | HttpError::Write | HttpError::Unauthorized => return,
    };
    let _ = write_text(socket, status, body).await;
}

/// Write `401 Unauthorized` with the `WWW-Authenticate: Bearer`
/// challenge header (RFC 6750 §3). Strict HTTP clients use the
/// challenge to know which auth scheme to negotiate; without it
/// they may treat the response as a hard failure.
async fn write_unauthorized(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let body = "unauthorized\n";
    let header = format!(
        "HTTP/1.1 401 Unauthorized\r\n\
         WWW-Authenticate: Bearer\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\r\n",
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
