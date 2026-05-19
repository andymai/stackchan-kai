//! Response-side helpers for the firmware HTTP server.
//!
//! Holds the [`HttpError`] enum, the body builders for the JSON
//! responses the route handlers emit, the response-writing wrappers
//! around the raw `TcpSocket` (`write_json`, `write_text`,
//! `write_no_content`, `write_dashboard`), the error-to-status mapper
//! ([`write_status_for_error`]), and the embedded operator dashboard
//! bundle ([`DASHBOARD_GZ`]).
//!
//! Split out from `http.rs` so the route handlers stay focused on
//! request parsing + dispatch; everything in this module is
//! independent of the route table and reusable from any future
//! per-concern handler module.

use alloc::format;
use alloc::string::String;

use embassy_net::tcp::TcpSocket;
use embedded_io_async::Write as AsyncWrite;

use super::snapshot::{self, AvatarSnapshot};

/// Self-contained operator dashboard, embedded at compile time as
/// gzip-compressed bytes. The bundle is built by `just web-build`
/// (Vite + Solid in `/web`); the firmware serves it under
/// `Content-Encoding: gzip` and lets the browser inflate.
///
/// `GET /` serves this; the bundle uses the SSE / POST / PUT routes
/// for live state and control.
pub(super) const DASHBOARD_GZ: &[u8] = include_bytes!("../../../../web/dist/index.html.gz");

/// Lightweight error wrapping for the request handler — ferries
/// socket and parse failures to a single `warn` log line at the
/// accept loop.
#[derive(Debug, defmt::Format)]
pub(super) enum HttpError {
    /// Socket read returned an error or EOF before the request was
    /// complete.
    Read,
    /// Socket write returned an error mid-response.
    Write,
    /// Header section never closed within `REQUEST_BUF_BYTES`.
    HeadersTooLarge,
    /// `Content-Length` exceeded `MAX_BODY_BYTES` or wasn't a valid
    /// non-negative integer.
    BodyTooLarge,
    /// Method + path didn't parse as a valid HTTP request line.
    Malformed,
    /// Write route required a bearer token; the request didn't carry
    /// one or it didn't match the configured value.
    Unauthorized,
}

/// Write `204 No Content` — used for write routes that don't need
/// a response body.
///
/// RFC 7230 §3.3.2 says a `204` "must not include a message body and
/// is terminated by the first empty line after the header fields";
/// the general `write_text` helper would still emit `Content-Type` +
/// `Content-Length: 0`, which is pedantically allowed but unusual.
/// This helper omits both so the response is just headers + CRLF.
pub(super) async fn write_no_content(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let header = "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n";
    socket
        .write_all(header.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;
    socket.flush().await.map_err(|_| HttpError::Write)
}

/// Serialise the `/health` body. Schema is a flat object — no nested
/// types, so a small `format!` keeps the dep surface clean.
pub(super) fn health_body() -> String {
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
pub(super) fn state_body(s: AvatarSnapshot) -> String {
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
    let decorator = s
        .decorator
        .map_or_else(|| String::from("null"), |d| format!("\"{}\"", d.wire_str()));
    format!(
        "{{\
\"emotion\":\"{emotion}\",\
\"mood\":\"{mood}\",\
\"face_geometry\":\"{face_geometry}\",\
\"decorator\":{decorator},\
\"head_pose\":{{\"pan_deg\":{pan:.2},\"tilt_deg\":{tilt:.2}}},\
\"head_actual\":{actual},\
\"battery\":{{\"percent\":{pct},\"voltage_mv\":{mv}}},\
\"wifi\":{{\"connected\":{connected},\"ip\":{ip}}},\
\"audio\":{{\"volume_pct\":{volume_pct},\"muted\":{muted}}},\
\"camera_mode\":{camera_mode}\
}}\n",
        emotion = s.emotion.wire_str(),
        mood = s.mood.wire_str(),
        face_geometry = s.face_geometry.wire_str(),
        pan = s.head_pose.pan_deg,
        tilt = s.head_pose.tilt_deg,
        connected = s.wifi.connected,
        volume_pct = s.audio.volume_pct,
        muted = s.audio.muted,
        camera_mode = s.camera_mode,
    )
}

/// Serialise the `/sensors` body. `null` for sensors that haven't
/// produced a sample yet; numbers elsewhere — three decimals for
/// accel-g, two for gyro-dps and ambient lux, four for the audio-RMS
/// norm.
pub(super) fn sensors_body(s: snapshot::SensorsSnapshot) -> String {
    let imu = s.imu.map_or_else(
        || String::from("null"),
        |i| {
            let (ax, ay, az) = i.accel_g;
            let (gx, gy, gz) = i.gyro_dps;
            format!(
                "{{\"accel_g\":[{ax:.3},{ay:.3},{az:.3}],\"gyro_dps\":[{gx:.2},{gy:.2},{gz:.2}]}}"
            )
        },
    );
    let lux = s
        .ambient_lux
        .map_or_else(|| String::from("null"), |l| format!("{l:.2}"));
    let body_touch = s.body_touch.map_or_else(
        || String::from("null"),
        |b| {
            format!(
                "{{\"left\":{},\"centre\":{},\"right\":{}}}",
                b.left, b.centre, b.right
            )
        },
    );
    format!(
        "{{\"imu\":{imu},\"ambient_lux\":{lux},\"audio_rms\":{rms:.4},\"body_touch\":{body_touch}}}\n",
        rms = s.audio_rms,
    )
}

/// Serialise the `/tasks` body — per-channel watchdog health.
pub(super) fn tasks_body(s: crate::watchdog::TasksSnapshot) -> String {
    use core::fmt::Write as _;
    let mut out = format!("{{\"window_ms\":{},\"channels\":[", s.window_ms);
    for (i, ch) in s.channels.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"name\":\"{}\",\"delta\":{},\"min_per_window\":{},\"stale\":{}}}",
            ch.name, ch.delta, ch.min_per_window, ch.stale
        );
    }
    out.push_str("]}\n");
    out
}

/// Serialise the `/events` body — recent ring entries, oldest first.
pub(super) fn events_body() -> String {
    use crate::event_log;
    use core::fmt::Write as _;
    let (total, entries) = event_log::drain_recent(event_log::CAP);
    let mut out = format!("{{\"total\":{total},\"events\":[");
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"at_ms\":{},\"kind\":\"{}\",\"message\":\"{}\"}}",
            e.at_ms,
            e.kind.wire_str(),
            json_escape(&e.message),
        );
    }
    out.push_str("]}\n");
    out
}

/// Minimal JSON string escaper — backslash, quote, and control bytes
/// get the `\\uNNNN` form. Event messages are firmware-controlled (no
/// user input lands here unredacted) so this stays simple.
pub(super) fn json_escape(s: &str) -> String {
    use core::fmt::Write as _;
    let mut out = String::with_capacity(s.len());
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
    out
}

/// Write `status` + `body` as `application/json`.
pub(super) async fn write_json(
    socket: &mut TcpSocket<'_>,
    status: u16,
    body: &str,
) -> Result<(), HttpError> {
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
pub(super) async fn write_text(
    socket: &mut TcpSocket<'_>,
    status: u16,
    body: &str,
) -> Result<(), HttpError> {
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

/// Serve [`DASHBOARD_GZ`] with `Content-Type: text/html` and
/// `Content-Encoding: gzip`. Cache is disabled so a freshly flashed
/// firmware's dashboard JS shows up on the next reload — the payload
/// is small over LAN, so a longer max-age was never worth the
/// staleness it caused.
pub(super) async fn write_dashboard(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Encoding: gzip\r\nContent-Length: {len}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        len = DASHBOARD_GZ.len(),
    );
    socket
        .write_all(header.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;
    socket
        .write_all(DASHBOARD_GZ)
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
pub(super) async fn write_status_for_error(socket: &mut TcpSocket<'_>, err: &HttpError) {
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
pub(super) async fn write_unauthorized(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
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
