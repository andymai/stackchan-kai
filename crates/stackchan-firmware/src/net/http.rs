//! Tiny hand-rolled HTTP/1.1 server for the LAN-scoped control
//! plane. Two read-only routes:
//!
//! - `GET /health` — uptime, firmware version, free heap (handy for
//!   liveness checks and post-flash smoke tests).
//! - `GET /state` — `AvatarSnapshot` JSON read non-destructively
//!   from `super::snapshot`.
//!
//! No external HTTP crate. The wire format is small, the surface
//! is fixed, and a hand-roll dodges the impl-trait-in-assoc-type
//! requirement that picoserve's `AppBuilder` brings in. Other
//! routes (`POST /pose`, `GET/PUT /settings`) extend the same
//! request matcher in a follow-up.

use alloc::format;
use alloc::string::String;

use embassy_net::Stack;
use embassy_net::tcp::TcpSocket;
use embassy_time::Duration;
use embedded_io_async::Write as AsyncWrite;

use super::snapshot::{self, AvatarSnapshot};
use super::wifi::{WIFI_LINK_SIGNAL, WifiLinkState};

/// Listening port. LAN-only; no auth.
const HTTP_PORT: u16 = 80;

/// Maximum request line + headers size we'll buffer before responding
/// `400`. Schema v1 only handles `GET /health` / `GET /state`, both
/// trivially short.
const REQUEST_BUF_BYTES: usize = 1024;

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
    /// Socket read returned an error or EOF before headers landed.
    Read,
    /// Socket write returned an error mid-response.
    Write,
    /// Header section never closed within `REQUEST_BUF_BYTES`.
    HeadersTooLarge,
    /// Method + path didn't parse as a valid HTTP request line.
    Malformed,
}

/// Serve a single HTTP/1.1 exchange against an accepted socket.
async fn serve_one(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let mut buf = [0u8; REQUEST_BUF_BYTES];
    let mut filled = 0usize;
    // Read until we see `\r\n\r\n` or hit the cap.
    loop {
        if filled >= REQUEST_BUF_BYTES {
            return Err(HttpError::HeadersTooLarge);
        }
        match socket.read(&mut buf[filled..]).await {
            Ok(n) if n > 0 => filled += n,
            // Zero-byte read = EOF before headers landed; same outcome
            // as a transport error from the caller's perspective.
            _ => return Err(HttpError::Read),
        }
        if buf[..filled].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }

    // Parse the request line: METHOD SP PATH SP HTTP/1.x CRLF
    let line_end = buf[..filled]
        .windows(2)
        .position(|w| w == b"\r\n")
        .ok_or(HttpError::Malformed)?;
    let line = core::str::from_utf8(&buf[..line_end]).map_err(|_| HttpError::Malformed)?;
    let mut parts = line.split(' ');
    let method = parts.next().ok_or(HttpError::Malformed)?;
    let path = parts.next().ok_or(HttpError::Malformed)?;

    match (method, path) {
        ("GET", "/health") => write_json(socket, 200, &health_body()).await,
        ("GET", "/state") => write_json(socket, 200, &state_body(snapshot::read())).await,
        ("GET", _) => write_text(socket, 404, "not found\n").await,
        _ => write_text(socket, 405, "method not allowed\n").await,
    }
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

/// Plain-text response, used for non-JSON error paths.
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
        404 => "Not Found",
        405 => "Method Not Allowed",
        // 200 + everything else fall through; the server only emits
        // codes from this short list, so anything else here is a
        // programming bug.
        _ => "OK",
    }
}
