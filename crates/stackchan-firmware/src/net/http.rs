//! Tiny hand-rolled HTTP/1.1 server for the LAN-scoped control plane.
//!
//! ## Routes
//!
//! - `GET /` — operator dashboard. TS + Solid bundle from `/web`,
//!   built by `just web-build` and embedded as gzipped bytes at
//!   compile time. Drives the live state via `/state/stream` and
//!   POSTs to the control routes.
//! - `GET /health` — uptime, firmware version, free heap (handy for
//!   liveness checks and post-flash smoke tests).
//! - `GET /state` — `AvatarSnapshot` JSON read non-destructively
//!   from `super::snapshot`.
//! - `GET /state/stream` — Server-Sent Events stream of
//!   `AvatarSnapshot` updates. Sends an initial event on connect,
//!   then one event per change (throttled at the producer to ~10 Hz),
//!   plus a `: heartbeat` SSE comment every 15 s.
//! - `GET /sensors` — `SensorsSnapshot` JSON: IMU accel/gyro,
//!   ambient lux, audio RMS, body-touch zones. Mirrored from each
//!   producer task into a snapshot static so the HTTP read never
//!   touches the source `Signal` channels (single-consumer/race).
//! - `GET /tasks` — watchdog channel health: per-task heartbeat
//!   delta in the last window + a `stale` flag.
//! - `GET /events` — recent operator-visible events (lifecycle,
//!   control actions, warnings) from a bounded RAM ring.
//! - `POST /emotion` — JSON `{"emotion": "...", "hold_ms": ...}`.
//!   Sets affect + holds `mind.autonomy` against the autonomous
//!   emotion drivers for `hold_ms` (default
//!   [`stackchan_net::http_command::DEFAULT_HOLD_MS`]).
//! - `POST /look-at` — JSON `{"pan_deg": f32, "tilt_deg": f32, "hold_ms": ...}`.
//!   Sets `mind.attention = Tracking { target }` for `hold_ms`,
//!   asserting the operator's target against camera tracking.
//! - `POST /face-target` — JSON `{"x": f32, "y": f32, "hold_ms": ...}`
//!   in normalised frame coordinates `[-1, 1]`. External CV servers
//!   (face / pose / object detectors on the LAN) post the latest
//!   centroid every frame; the firmware converts via the camera FOV
//!   and routes through the same `RemoteCommand::LookAt` path as
//!   `/look-at` so cognition modifiers don't have to distinguish the
//!   command source.
//! - `POST /palette` — JSON `{"palette": "<name>"}`. Switches the
//!   avatar's colour palette at runtime; persisted to
//!   `/sd/RUNTIME.RON` so a reboot restores the selection. Vocabulary
//!   matches `Palette::wire_str` (default / dark / cute / dog).
//! - `POST /sleep` — empty body. Drops eyes shut, head limp, LED
//!   ring dark, audio TX paused. Wake via `POST /wake`, MCP `wake`,
//!   any touch (`FT6336U` screen or `Si12T` body pads), or the
//!   `AXP2101` short-press. Runtime-only — sleep state resets on
//!   reboot.
//! - `POST /wake` — empty body. Resumes the live modifier face +
//!   head + LED state.
//! - `GET  /head/offsets` — current operator-applied yaw/tilt zero
//!   correction (degrees). Returns `{"yaw_offset_deg":F,"tilt_offset_deg":F}`.
//! - `POST /head/offsets` — JSON
//!   `{"yaw_offset_deg":F,"tilt_offset_deg":F}`. Sets the head's
//!   zero-point correction; both axes required, each clamped to
//!   `[-30°, +30°]`. Layered on top of the firmware's compile-time
//!   trim — runtime-only in v0.2.0, persistence ships with NVS.
//! - `POST /firmware/update` — ed25519-signed SCFW image upload.
//!   Verifies the signature against the build-time public key,
//!   streams the payload into the inactive OTA slot, flips the
//!   bootloader's `otadata` pointer, and soft-resets. Compiled out
//!   when `STACKCHAN_OTA_PUBLIC_KEY` isn't set at build time (`503
//!   Service Unavailable`); always requires a configured bearer
//!   token even when the global token is otherwise empty. See
//!   [`crate::ota`] for the full sequence.
//! - `POST /reset` — empty body. Clears any active emotion or
//!   look-at hold and returns the avatar to autonomous behaviour.
//! - `POST /speak` — JSON `{"phrase": "...", "locale": "..."}`.
//!   Renders a [`stackchan_core::voice::PhraseId`] from the baked
//!   catalog and queues it on the audio TX path. Fire-and-forget;
//!   no avatar-state hold timer.
//! - `POST /camera/mode` — JSON `{"enabled": <bool>}`. Flips the
//!   LCD between camera preview and avatar; tracking continues in
//!   either mode. Ephemeral (no SD writeback); a power-cycle returns
//!   to avatar.
//! - `POST /camera/capture` — empty body. Signals the camera task to
//!   write the latest QVGA RGB565 frame to `/sd/CAPTURE.565`; returns
//!   `202 Accepted`. The SD write happens asynchronously inside the
//!   camera task and stalls render for ~200 ms during the SPI burst.
//! - `POST /restart` — empty body. Returns `202 Accepted` and triggers
//!   an `esp_hal::system::software_reset` ~200 ms later (after the
//!   response drains). Always requires an authenticated token, even
//!   when the global token is empty — destructive ops opt-in only.
//! - `POST /factory-reset` — JSON body `{"confirm":"erase"}`. Wipes
//!   every operator-visible file (`STACKCHAN.RON`, `BONDS.BIN`,
//!   `CAPTURE.565`, staging files) and soft-resets. Same always-auth
//!   gate as `/restart`.
//! - `GET /settings` — current persisted [`stackchan_net::Config`]
//!   as JSON, with `wifi.psk` and `auth.token` redacted.
//! - `GET /settings/backup` — same payload UNREDACTED. The single
//!   auth-gated GET (every other GET stays open) — returning real
//!   secrets is what makes the backup usable for restore via PUT.
//! - `PUT /settings` — full-replace [`stackchan_net::Config`] body.
//!   Validates, writes back atomically to `/sd/STACKCHAN.RON`, and
//!   responds `{"reboot_required": <bool>}`. Wi-Fi creds reconnect
//!   immediately via [`super::wifi::WIFI_RECONFIG`] and audio
//!   mirrors into the live snapshot — `reboot_required` reflects
//!   only the blocks that take effect at start-up (mDNS hostname,
//!   SNTP, tracker tuning).
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
use stackchan_core::{Clock as _, RemoteCommand};
use stackchan_net::http_command::{self as json, JsonError};
use stackchan_net::http_parse::{
    ct_eq, find_subsequence, parse_bearer_token, parse_content_length,
};

use super::snapshot::{self, AvatarSnapshot};
use super::wifi::{LINK_READY, WIFI_RECONFIG, WifiCreds};

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

/// Self-contained operator dashboard, embedded at compile time as
/// gzip-compressed bytes. The bundle is built by `just web-build`
/// (Vite + Solid in `/web`); the firmware serves it under
/// `Content-Encoding: gzip` and lets the browser inflate.
///
/// `GET /` serves this; the bundle uses the SSE / POST / PUT routes
/// for live state and control.
const DASHBOARD_GZ: &[u8] = include_bytes!("../../../../web/dist/index.html.gz");

/// Latest control-plane command.
///
/// Set by the HTTP task on a successful POST; drained by the render
/// task into `entity.input.remote_command` before `Director::run`.
/// Latest-wins semantics — a second POST that lands before the
/// render task drains will overwrite the first.
pub static REMOTE_COMMAND_SIGNAL: Signal<CriticalSectionRawMutex, RemoteCommand> = Signal::new();

/// Latest mood the operator has selected via `POST /mood`.
///
/// Drained by the render task into `entity.mind.mood` ahead of
/// `Director::run`. Latest-wins semantics; the selection survives
/// reboots via `/sd/RUNTIME.RON` (see `runtime_store::update_mood`).
pub static MOOD_SIGNAL: Signal<CriticalSectionRawMutex, stackchan_core::Mood> = Signal::new();

/// Latest palette the operator has selected via `POST /palette`.
///
/// Drained by the render task into `entity.face.palette` ahead of
/// `Director::run`. Latest-wins semantics; persistence ships in a
/// follow-up alongside the NVS `RuntimeStore`.
pub static PALETTE_SIGNAL: Signal<CriticalSectionRawMutex, stackchan_core::Palette> = Signal::new();

/// Latest face-geometry preset the operator has selected via
/// `POST /face-geometry` or MCP `set_face_geometry`.
///
/// Drained by the render task, which calls `Face::set_geometry` so
/// dynamic state (blink phase, mouth open amount) survives the swap.
/// Latest-wins; persisted to `/sd/RUNTIME.RON` via
/// `runtime_store::update_face_geometry`.
pub static FACE_GEOMETRY_SIGNAL: Signal<CriticalSectionRawMutex, stackchan_core::FaceGeometry> =
    Signal::new();

/// Latest dance script uploaded via `POST /dance`.
///
/// Drained by the render task into `DancePlayer` via
/// `dance_player.load_script(script, now)` so the player anchors at
/// the upload instant. Latest-wins: a new upload mid-dance replaces
/// the active script. The script lives behind an `Arc` so the
/// signal handoff is cheap — no copy of the keyframe vector.
pub static DANCE_SCRIPT_SIGNAL: Signal<
    CriticalSectionRawMutex,
    alloc::sync::Arc<stackchan_core::dance::DanceScript>,
> = Signal::new();

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
///
/// The route table is the natural shape for "every endpoint in one
/// place" — splitting it would scatter the auth gate + path matching
/// across files for no real win.
#[allow(clippy::too_many_lines)]
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

    // OTA `POST /firmware/update` is the one route that bypasses
    // the small fixed-body cap — the firmware payload is megabytes
    // and lives on PSRAM for the duration of the verify+flash. Detect
    // it here, after method/path are known but before the small-body
    // read loop runs the cap. Auth is checked inside the OTA
    // handler so the rest of the cap-and-buffer logic stays
    // dedicated to the small-body routes.
    let method_for_route =
        core::str::from_utf8(&buf[..first_sp]).map_err(|_| HttpError::Malformed)?;
    let path_for_route =
        core::str::from_utf8(&buf[path_start..second_sp]).map_err(|_| HttpError::Malformed)?;
    if method_for_route == "POST" && path_for_route == "/firmware/update" {
        // Hand off to the streaming OTA handler — it copies the
        // body bytes already in `buf` and reads the rest from the
        // socket directly into a heap-allocated Vec.
        let auth_token = parse_bearer_token(&buf[line_end + 2..header_end]);
        let already_buffered = filled.saturating_sub(body_start);
        let prefix = if already_buffered > 0 {
            &buf[body_start..body_start + already_buffered]
        } else {
            &[][..]
        };
        return handle_post_firmware_update(socket, content_length, prefix, auth_token).await;
    }

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
    //
    // Two exceptions on the otherwise-open GET / always-open POST
    // side:
    //
    // 1. `GET /settings/backup` returns the full config including
    //    secrets — auth-gated like a write, by design.
    // 2. `POST /restart` and `POST /factory-reset` always require an
    //    authenticated token, even when the global token is empty.
    //    Destructive ops shouldn't ride the LAN-open default; an
    //    operator must opt in to recovery by setting a token first.
    let backup_get = method == "GET" && path == "/settings/backup";
    let destructive = method == "POST" && (path == "/restart" || path == "/factory-reset");
    if matches!(method, "PUT" | "POST") || backup_get {
        let provided = parse_bearer_token(&buf[line_end + 2..header_end]);
        let snapshot = crate::storage::CONFIG_SNAPSHOT.lock().await;
        let authorized = match snapshot.as_ref() {
            Some(cfg) if !cfg.auth.token.is_empty() => {
                provided.is_some_and(|t| ct_eq(t.as_bytes(), cfg.auth.token.as_bytes()))
            }
            // Empty token = LAN-open for everyday ops, but destructive
            // routes need an explicit opt-in.
            _ if destructive => false,
            _ => true,
        };
        drop(snapshot);
        if !authorized {
            return Err(HttpError::Unauthorized);
        }
        // Record every authenticated write so an operator can see what
        // the dashboard / nRF Connect / curl session has been doing.
        // Stays after the auth gate to avoid logging probe traffic.
        crate::event_log::record_fmt(
            crate::event_log::Kind::Control,
            format_args!("{method} {path}"),
        );
    }

    match (method, path) {
        ("GET", "/" | "/index.html") => write_dashboard(socket).await,
        ("GET", "/health") => write_json(socket, 200, &health_body()).await,
        ("GET", "/state") => write_json(socket, 200, &state_body(snapshot::read())).await,
        ("GET", "/state/stream") => handle_state_stream(socket).await,
        ("GET", "/sensors") => {
            write_json(socket, 200, &sensors_body(snapshot::read_sensors())).await
        }
        ("GET", "/tasks") => {
            write_json(
                socket,
                200,
                &tasks_body(crate::watchdog::read_tasks_snapshot()),
            )
            .await
        }
        ("GET", "/events") => write_json(socket, 200, &events_body()).await,
        ("GET", "/settings") => handle_get_settings(socket).await,
        ("GET", "/settings/backup") => handle_get_settings_backup(socket).await,
        ("PUT", "/settings") => handle_put_settings(socket, body).await,
        ("POST", "/emotion") => handle_remote(socket, json::parse_set_emotion(body)).await,
        ("POST", "/look-at") => handle_remote(socket, json::parse_look_at(body)).await,
        ("POST", "/look-at-point") => handle_remote(socket, json::parse_look_at_point(body)).await,
        ("POST", "/face-target") => handle_remote(socket, json::parse_face_target(body)).await,
        ("POST", "/dance") => handle_post_dance(socket, body).await,
        ("POST", "/motion") => handle_post_motion(socket, body).await,
        ("POST", "/reset") => handle_remote(socket, Ok(RemoteCommand::Reset)).await,
        ("POST", "/speak") => handle_remote(socket, json::parse_speak(body)).await,
        ("POST", "/listen") => handle_remote(socket, json::parse_start_listen(body)).await,
        ("POST", "/pair") => handle_remote(socket, json::parse_enter_pairing(body)).await,
        ("POST", "/volume") => handle_post_volume(socket, body).await,
        ("POST", "/mute") => handle_post_mute(socket, body).await,
        ("POST", "/mood") => handle_post_mood(socket, body).await,
        ("POST", "/palette") => handle_post_palette(socket, body).await,
        ("POST", "/face-geometry") => handle_post_face_geometry(socket, body).await,
        ("GET", "/crash") => handle_get_crash(socket).await,
        ("POST", "/crash/clear") => handle_post_crash_clear(socket).await,
        ("POST", "/sleep") => handle_post_sleep(socket).await,
        ("POST", "/wake") => handle_post_wake(socket).await,
        ("POST", "/toast") => handle_post_toast(socket, body).await,
        ("GET", "/head/offsets") => handle_get_head_offsets(socket).await,
        ("POST", "/head/offsets") => handle_post_head_offsets(socket, body).await,
        ("POST", "/mcp") => handle_post_mcp(socket, body).await,
        ("POST", "/camera/mode") => handle_post_camera_mode(socket, body).await,
        ("POST", "/camera/capture") => handle_post_camera_capture(socket).await,
        ("GET", "/camera/snapshot") => handle_get_camera_snapshot(socket).await,
        ("POST", "/restart") => handle_post_restart(socket).await,
        ("POST", "/factory-reset") => handle_post_factory_reset(socket, body).await,
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

/// `GET /settings/backup` — render the current snapshot UNREDACTED.
///
/// The one auth-gated GET in this control plane: returning real
/// `wifi.psk` and `auth.token` over the wire is what makes a backup
/// usable for restore via PUT, but it's also why this route requires
/// the bearer token while every other GET stays open.
async fn handle_get_settings_backup(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let snapshot = crate::storage::CONFIG_SNAPSHOT.lock().await.clone();
    let Some(config) = snapshot else {
        return write_text(socket, 503, "config snapshot unavailable\n").await;
    };
    match stackchan_net::render_settings_json(&config, false) {
        Ok(body) => write_json(socket, 200, &body).await,
        Err(_) => write_text(socket, 500, "render failed\n").await,
    }
}

/// Decide whether the new config requires a reboot to fully apply.
///
/// Wi-Fi creds and audio (volume + mute) are signalled through to the
/// running tasks and apply immediately. mDNS hostname, SNTP servers,
/// and tracker tuning take effect on the next boot — those tasks read
/// their config once at start-up. Auth-token changes apply
/// immediately to the next request via the lock-free read in the
/// auth gate.
fn requires_reboot(prev: &stackchan_net::Config, new: &stackchan_net::Config) -> bool {
    if prev.mdns.hostname != new.mdns.hostname {
        return true;
    }
    if prev.time.tz != new.time.tz || prev.time.sntp_servers != new.time.sntp_servers {
        return true;
    }
    if prev.tracker != new.tracker {
        return true;
    }
    false
}

/// `PUT /settings` — full replace, atomic SD writeback. Returns
/// `{"reboot_required": <bool>}` where `<bool>` reflects whether any
/// changed field can only take effect on the next boot (mDNS
/// hostname, SNTP, tracker tuning) — Wi-Fi creds and audio still
/// apply immediately via the existing signal paths.
///
/// On a change to `wifi.ssid` or `wifi.psk` (compared against the
/// current `CONFIG_SNAPSHOT`), signals [`WIFI_RECONFIG`] so the
/// wifi task drops the link and reconnects with the new creds.
/// Operators changing the AP from the dashboard now see a brief
/// link blip rather than needing to power-cycle the device.
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
    //
    // Track whether the snapshot was actually populated so the
    // reboot-required diff doesn't compare against a synthesized
    // default (which would falsely flag every first-boot PUT as
    // requiring reboot just because the new value differs from the
    // struct default).
    let prior_snapshot = crate::storage::CONFIG_SNAPSHOT.lock().await.clone();
    let had_prior_snapshot = prior_snapshot.is_some();
    let snapshot_for_merge = prior_snapshot.unwrap_or_default();
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
            crate::event_log::record(crate::event_log::Kind::Lifecycle, "settings persisted");
            // Soft-reconnect Wi-Fi only when the credentials actually
            // changed. Comparing against the pre-merge snapshot avoids
            // a needless link drop when the dashboard re-PUTs the
            // same body (e.g. the user only changed mDNS hostname,
            // which can't be applied without a reboot anyway).
            let wifi_changed = new_config.wifi.ssid != snapshot_for_merge.wifi.ssid
                || new_config.wifi.psk != snapshot_for_merge.wifi.psk;
            if wifi_changed {
                defmt::info!("http: PUT /settings triggered Wi-Fi soft reconnect");
                WIFI_RECONFIG.signal(WifiCreds {
                    ssid: new_config.wifi.ssid.clone(),
                    psk: new_config.wifi.psk.clone(),
                });
            }
            // Mirror the audio block into the live snapshot so the
            // SSE stream picks up volume / mute changes from a full
            // settings PUT without waiting for a reboot.
            snapshot::update_audio(new_config.audio);
            // First-boot PUT (no prior snapshot) can't have changed
            // anything that was already running — every field in the
            // synthesised default was never live, so reboot_required
            // is unconditionally false.
            let reboot_required =
                had_prior_snapshot && requires_reboot(&snapshot_for_merge, &new_config);
            *crate::storage::CONFIG_SNAPSHOT.lock().await = Some(new_config);
            let body = format!("{{\"reboot_required\":{reboot_required}}}\n");
            write_json(socket, 200, &body).await
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

/// `POST /volume` — parse `{"level": 0..=100}`, persist to
/// `STACKCHAN.RON`, signal the audio task to apply via the AW88298.
///
/// Persist-then-signal ordering: a failed SD write leaves the amp
/// at its current level rather than partially applying. Mirrors the
/// shape of [`handle_put_settings`], including the clone-and-drop of
/// the `CONFIG_SNAPSHOT` mutex before the SD write — the mutex is
/// also taken by the auth gate, `GET /settings`, and the boot
/// audio-init path, so holding it across an SD write would stall
/// every concurrent request for the full write duration.
async fn handle_post_volume(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    let level = match json::parse_volume(body) {
        Ok(p) => p,
        Err(e) => {
            defmt::warn!(
                "http: POST /volume parse failed ({})",
                defmt::Debug2Format(&e)
            );
            let body = format!("invalid request body: {e:?}\n");
            return write_text(socket, 400, &body).await;
        }
    };
    audio_persist_to_http(socket, crate::audio::persist_volume(level).await).await
}

/// `POST /mute` — parse `{"muted": <bool>}`, persist to
/// `STACKCHAN.RON`, signal the audio task to apply via the AW88298.
///
/// Symmetric with [`handle_post_volume`]; mute is a separate boolean
/// (not `volume = 0`) so unmuting restores the prior level. Same
/// snapshot-mutex hygiene applies — clone the current snapshot, drop
/// the lock, then re-acquire only to install the new value.
async fn handle_post_mute(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    let muted = match json::parse_mute(body) {
        Ok(m) => m,
        Err(e) => {
            defmt::warn!(
                "http: POST /mute parse failed ({})",
                defmt::Debug2Format(&e)
            );
            let body = format!("invalid request body: {e:?}\n");
            return write_text(socket, 400, &body).await;
        }
    };
    audio_persist_to_http(socket, crate::audio::persist_mute(muted).await).await
}

/// Map an [`crate::audio::AudioPersistOutcome`] to its HTTP response.
/// Shared by `POST /volume` and `POST /mute` so the two routes always
/// surface the same status codes for matching outcomes.
async fn audio_persist_to_http(
    socket: &mut TcpSocket<'_>,
    outcome: crate::audio::AudioPersistOutcome,
) -> Result<(), HttpError> {
    use crate::audio::AudioPersistOutcome;
    match outcome {
        AudioPersistOutcome::Persisted => write_no_content(socket).await,
        AudioPersistOutcome::NoSnapshot => {
            write_text(socket, 503, "config snapshot unavailable\n").await
        }
        AudioPersistOutcome::NoStorage => write_text(socket, 503, "no SD card mounted\n").await,
        AudioPersistOutcome::WriteFailed => write_text(socket, 500, "config write failed\n").await,
    }
}

/// `POST /dance` — parse the keyframe stream, hand the script off
/// to the render task's `DancePlayer` via [`DANCE_SCRIPT_SIGNAL`].
///
/// Body shape per [`stackchan_net::dance::parse_dance`]: a top-level
/// object with a `keyframes` array. Each keyframe carries `at_ms`
/// plus any subset of motion / avatar / RGB channel fields.
///
/// Returns `204 No Content` on a parsed-and-handed-off script, `400`
/// with the parser error on malformed input.
async fn handle_post_dance(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    use alloc::sync::Arc;
    let script = match stackchan_net::dance::parse_dance(body) {
        Ok(s) => s,
        Err(e) => {
            defmt::warn!(
                "http: POST /dance parse failed ({})",
                defmt::Debug2Format(&e)
            );
            let msg = format!("invalid dance script: {e:?}\n");
            return write_text(socket, 400, &msg).await;
        }
    };
    let len = script.keyframes.len();
    DANCE_SCRIPT_SIGNAL.signal(Arc::new(script));
    defmt::info!("http: POST /dance → {=usize} keyframes loaded", len);
    write_no_content(socket).await
}

/// `POST /motion` — JSON `{"motion": "greet"|"nod"|"shake"|"laugh"}`.
/// Looks the variant up in [`NamedMotion::from_wire_str`], hands the
/// baked [`DanceScript`] off to [`DANCE_SCRIPT_SIGNAL`] so the
/// existing `DancePlayer` modifier drives the gesture. Returns 400
/// on an unknown motion name, 204 otherwise.
async fn handle_post_motion(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    use alloc::sync::Arc;
    let motion = match json::parse_motion(body) {
        Ok(m) => m,
        Err(e) => {
            defmt::warn!(
                "http: POST /motion parse failed ({})",
                defmt::Debug2Format(&e)
            );
            let msg = format!("invalid motion: {e:?}\n");
            return write_text(socket, 400, &msg).await;
        }
    };
    DANCE_SCRIPT_SIGNAL.signal(Arc::new(motion.script()));
    defmt::info!("http: POST /motion → {=str}", motion.wire_str());
    write_no_content(socket).await
}

/// `POST /mood` — parse `{"mood": "<string>"}`, push the new mood
/// at the render task via [`MOOD_SIGNAL`], and persist the choice to
/// `/sd/RUNTIME.RON` so a reboot restores it.
async fn handle_post_mood(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    let mood = match json::parse_mood(body) {
        Ok(m) => m,
        Err(e) => {
            defmt::warn!(
                "http: POST /mood parse failed ({})",
                defmt::Debug2Format(&e)
            );
            let body = format!("invalid request body: {e:?}\n");
            return write_text(socket, 400, &body).await;
        }
    };
    MOOD_SIGNAL.signal(mood);
    let _ = crate::runtime_store::update_mood(mood).await;
    defmt::info!("http: POST /mood → {}", mood.wire_str());
    write_no_content(socket).await
}

/// `GET /head/offsets` — read the current operator-applied head
/// zero-point correction. Returns the live cache published by the
/// head task on every offset update; an operator can confirm the
/// applied value without a round-trip through `/state`.
async fn handle_get_head_offsets(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let offsets = crate::head::current_offsets();
    let body = format!(
        r#"{{"yaw_offset_deg":{:.3},"tilt_offset_deg":{:.3}}}"#,
        offsets.yaw_offset_deg, offsets.tilt_offset_deg,
    );
    write_json(socket, 200, &body).await
}

/// `POST /head/offsets` — parse the JSON body and push the new
/// offsets at the head task via
/// [`crate::head::OFFSETS_SIGNAL`]. Runtime-only; head offsets are
/// not yet enrolled in the SD-backed `RuntimeStore` and reset on
/// reboot.
async fn handle_post_head_offsets(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    let offsets = match json::parse_head_offsets(body) {
        Ok(o) => o,
        Err(e) => {
            defmt::warn!(
                "http: POST /head/offsets parse failed ({})",
                defmt::Debug2Format(&e)
            );
            let body = format!("invalid request body: {e:?}\n");
            return write_text(socket, 400, &body).await;
        }
    };
    let firmware_offsets = crate::head::HeadOffsets {
        yaw_offset_deg: offsets.yaw_offset_deg,
        tilt_offset_deg: offsets.tilt_offset_deg,
    };
    crate::head::OFFSETS_SIGNAL.signal(firmware_offsets);
    // Also publish to the read-back cache directly so an immediate
    // `GET /head/offsets` from the same client doesn't lag behind
    // the head task's signal-drain (which only runs at the 50 Hz
    // tick, leaving a small POST-then-GET window where the read
    // returns the prior value).
    crate::head::OFFSETS_CACHE.lock(|cell| cell.set(firmware_offsets));
    defmt::info!(
        "http: POST /head/offsets → yaw={=f32} tilt={=f32}",
        firmware_offsets.yaw_offset_deg,
        firmware_offsets.tilt_offset_deg,
    );
    write_no_content(socket).await
}

/// `POST /palette` — parse `{"palette": "<string>"}`, push the
/// selected palette at the render task via [`PALETTE_SIGNAL`], and
/// persist the choice to `/sd/RUNTIME.RON` so a reboot restores it.
async fn handle_post_palette(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    let palette = match json::parse_palette(body) {
        Ok(p) => p,
        Err(e) => {
            defmt::warn!(
                "http: POST /palette parse failed ({})",
                defmt::Debug2Format(&e)
            );
            let body = format!("invalid request body: {e:?}\n");
            return write_text(socket, 400, &body).await;
        }
    };
    PALETTE_SIGNAL.signal(palette);
    let _ = crate::runtime_store::update_palette(palette).await;
    defmt::info!("http: POST /palette → {}", palette.wire_str());
    write_no_content(socket).await
}

/// `POST /face-geometry` — parse `{"geometry": "<string>"}`, push the
/// selected preset at the render task via [`FACE_GEOMETRY_SIGNAL`],
/// and persist the choice to `/sd/RUNTIME.RON` so a reboot restores it.
async fn handle_post_face_geometry(
    socket: &mut TcpSocket<'_>,
    body: &str,
) -> Result<(), HttpError> {
    let geometry = match json::parse_face_geometry(body) {
        Ok(g) => g,
        Err(e) => {
            defmt::warn!(
                "http: POST /face-geometry parse failed ({})",
                defmt::Debug2Format(&e)
            );
            let body = format!("invalid request body: {e:?}\n");
            return write_text(socket, 400, &body).await;
        }
    };
    FACE_GEOMETRY_SIGNAL.signal(geometry);
    let _ = crate::runtime_store::update_face_geometry(geometry).await;
    defmt::info!("http: POST /face-geometry → {}", geometry.wire_str());
    write_no_content(socket).await
}

/// `GET /crash` — return the most recent panic log written by the
/// boot path from the persistent RTC crash latch.
///
/// Body is the raw rendered log entry — line-delimited `key=value`
/// pairs (`reset_reason`, `file`, `line`, `message`). Returns 404
/// if no crash has been recorded since the last clear, 503 if the
/// SD card is absent or unreadable.
async fn handle_get_crash(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let outcome = crate::storage::with_storage(crate::storage::FirmwareStorage::read_crash).await;
    match outcome {
        Some(Ok(Some(text))) => write_text(socket, 200, &text).await,
        Some(Ok(None)) => write_text(socket, 404, "no crash recorded\n").await,
        Some(Err(e)) => {
            defmt::warn!("http: GET /crash read failed ({})", defmt::Debug2Format(&e));
            write_text(socket, 500, "crash log read failed\n").await
        }
        None => write_text(socket, 503, "no SD card mounted\n").await,
    }
}

/// `POST /crash/clear` — empty body. Deletes `/sd/CRASH.LOG` so
/// subsequent `GET /crash` calls return 404. Idempotent: clearing
/// when no log exists returns 204.
async fn handle_post_crash_clear(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let outcome = crate::storage::with_storage(crate::storage::FirmwareStorage::delete_crash).await;
    match outcome {
        Some(Ok(())) => write_no_content(socket).await,
        Some(Err(e)) => {
            defmt::warn!(
                "http: POST /crash/clear failed ({})",
                defmt::Debug2Format(&e)
            );
            write_text(socket, 500, "crash log delete failed\n").await
        }
        None => write_text(socket, 503, "no SD card mounted\n").await,
    }
}

/// `POST /sleep` — empty body. Drops eyes shut, head limp, LED ring
/// dark, audio TX paused. Wake via `POST /wake`, MCP `wake`, any
/// touch (`FT6336U` or `Si12T` body pads), or `AXP2101` short-press.
async fn handle_post_sleep(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    crate::sleep::SLEEP_SIGNAL.signal(crate::sleep::SleepState::Sleeping);
    defmt::info!("http: POST /sleep → entering sleep");
    write_no_content(socket).await
}

/// `POST /wake` — empty body. Reverse of [`handle_post_sleep`].
async fn handle_post_wake(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    crate::sleep::SLEEP_SIGNAL.signal(crate::sleep::SleepState::Awake);
    defmt::info!("http: POST /wake → exiting sleep");
    write_no_content(socket).await
}

/// `POST /toast` — JSON `{"level":"warn"|"error","message":"..."}`.
/// Pushes a toast onto the firmware's render-side overlay. Useful for
/// operator-driven verification when the overlay is enabled via
/// `behavior.toast_overlay_enabled`.
///
/// Returns 400 on a body that fails to parse, 204 otherwise. The
/// overlay still requires `toast_overlay_enabled = true` in the boot
/// config for the toast to actually render.
async fn handle_post_toast(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    use stackchan_net::mcp::find_string_field;
    let level = match find_string_field(body, "level") {
        Ok(Some("warn")) => crate::toast::ToastLevel::Warn,
        Ok(Some("error")) => crate::toast::ToastLevel::Error,
        Ok(Some(other)) => {
            defmt::warn!("http: POST /toast unknown level={=str}", other);
            return write_text(socket, 400, "level must be \"warn\" or \"error\"\n").await;
        }
        Ok(None) => {
            return write_text(socket, 400, "missing level field\n").await;
        }
        Err(_) => {
            return write_text(socket, 400, "malformed JSON body\n").await;
        }
    };
    let message = match find_string_field(body, "message") {
        Ok(Some(m)) => m,
        Ok(None) => "",
        Err(_) => {
            return write_text(socket, 400, "malformed JSON body\n").await;
        }
    };
    crate::toast::push(level, message, crate::clock::HalClock.now());
    defmt::info!(
        "http: POST /toast level={=?} msg={=str}",
        defmt::Debug2Format(&level),
        message
    );
    write_no_content(socket).await
}

/// `POST /mcp` — JSON-RPC 2.0 endpoint speaking minimal MCP.
///
/// Reads a JSON-RPC request, dispatches to one of `initialize` /
/// `tools/list` / `tools/call`, and returns the response. Tools wrap
/// the existing control-plane primitives (`set_emotion`, `set_mood`,
/// `look_at`, `speak`, `get_state`); the bridge is mechanical and the
/// MCP module in `stackchan-net` does the parsing.
///
/// All responses use HTTP 200 — JSON-RPC errors live in the body's
/// `error` field. The exception is a malformed JSON envelope, which
/// returns 400 since the request never made it into the protocol.
async fn handle_post_mcp(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    use stackchan_net::mcp::{
        INITIALIZE_RESULT_JSON, JsonRpcErrorCode, TOOLS_LIST_RESULT_JSON, find_object_field,
        find_string_field, parse_request, render_error, render_success,
    };

    let req = match parse_request(body) {
        Ok(r) => r,
        Err(e) => {
            defmt::warn!("http: POST /mcp parse failed ({=str})", e.detail);
            let resp = render_error(None, e.code, e.detail);
            return write_json(socket, 400, &resp).await;
        }
    };

    let resp = match req.method {
        "initialize" => render_success(req.id, INITIALIZE_RESULT_JSON),
        "tools/list" => render_success(req.id, TOOLS_LIST_RESULT_JSON),
        "tools/call" => {
            let Some(params) = req.params_raw else {
                let r = render_error(
                    Some(req.id),
                    JsonRpcErrorCode::InvalidParams,
                    "tools/call requires params",
                );
                return write_json(socket, 200, &r).await;
            };
            let tool_name = match find_string_field(params, "name") {
                Ok(Some(n)) => n,
                Ok(None) => {
                    let r = render_error(
                        Some(req.id),
                        JsonRpcErrorCode::InvalidParams,
                        "missing 'name' field",
                    );
                    return write_json(socket, 200, &r).await;
                }
                Err(e) => {
                    let r = render_error(Some(req.id), e.code, e.detail);
                    return write_json(socket, 200, &r).await;
                }
            };
            let arguments = find_object_field(params, "arguments")
                .ok()
                .flatten()
                .unwrap_or("{}");
            mcp_dispatch_tool(req.id, tool_name, arguments).await
        }
        _ => render_error(
            Some(req.id),
            JsonRpcErrorCode::MethodNotFound,
            "unknown JSON-RPC method",
        ),
    };
    write_json(socket, 200, &resp).await
}

/// Dispatch a `tools/call` request to the matching control-plane
/// primitive. Returns a fully-rendered JSON-RPC response (success or
/// error) string ready for `write_json`.
///
/// Async because `set_volume` and `set_mute` thread through the audio
/// task's SD-write persistence path, mirroring `POST /volume` and
/// `POST /mute`. Other tools resolve synchronously by signalling
/// `REMOTE_COMMAND_SIGNAL` and rendering a fixed acknowledgement.
#[allow(
    clippy::too_many_lines,
    reason = "single dispatch table that mirrors `TOOLS_LIST_RESULT_JSON`; \
              splitting fragments the per-tool layout the catalogue depends on"
)]
async fn mcp_dispatch_tool(id: i64, tool: &str, arguments: &str) -> String {
    use stackchan_net::mcp::{
        JsonRpcErrorCode, render_error, render_success, render_tool_text_result,
    };

    match tool {
        "set_emotion" => match json::parse_set_emotion(arguments) {
            Ok(cmd) => {
                REMOTE_COMMAND_SIGNAL.signal(cmd);
                render_success(id, &render_tool_text_result("emotion enqueued"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "set_mood" => match json::parse_mood(arguments) {
            Ok(mood) => {
                crate::net::http::MOOD_SIGNAL.signal(mood);
                let _ = crate::runtime_store::update_mood(mood).await;
                render_success(id, &render_tool_text_result("mood enqueued"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "set_face_geometry" => match json::parse_face_geometry(arguments) {
            Ok(geometry) => {
                FACE_GEOMETRY_SIGNAL.signal(geometry);
                let _ = crate::runtime_store::update_face_geometry(geometry).await;
                render_success(id, &render_tool_text_result("face geometry enqueued"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "look_at" => match json::parse_look_at(arguments) {
            Ok(cmd) => {
                REMOTE_COMMAND_SIGNAL.signal(cmd);
                render_success(id, &render_tool_text_result("look-at enqueued"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "speak" => match json::parse_speak(arguments) {
            Ok(cmd) => {
                REMOTE_COMMAND_SIGNAL.signal(cmd);
                render_success(id, &render_tool_text_result("phrase enqueued"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "play_motion" => {
            use alloc::sync::Arc;
            match json::parse_motion(arguments) {
                Ok(motion) => {
                    DANCE_SCRIPT_SIGNAL.signal(Arc::new(motion.script()));
                    render_success(id, &render_tool_text_result("motion enqueued"))
                }
                Err(e) => render_error(
                    Some(id),
                    JsonRpcErrorCode::InvalidParams,
                    tool_parse_detail(&e),
                ),
            }
        }
        "start_listen" => match json::parse_start_listen(arguments) {
            Ok(cmd) => {
                REMOTE_COMMAND_SIGNAL.signal(cmd);
                render_success(id, &render_tool_text_result("listen window opened"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "enter_pairing" => match json::parse_enter_pairing(arguments) {
            Ok(cmd) => {
                if let RemoteCommand::EnterPairing { duration_ms } = cmd {
                    crate::net::esp_now::open_pair_window(duration_ms);
                }
                REMOTE_COMMAND_SIGNAL.signal(cmd);
                render_success(id, &render_tool_text_result("pairing window opened"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "set_volume" => match json::parse_volume(arguments) {
            Ok(level) => match crate::audio::persist_volume(level).await {
                crate::audio::AudioPersistOutcome::Persisted => {
                    render_success(id, &render_tool_text_result("volume persisted"))
                }
                outcome => render_error(
                    Some(id),
                    JsonRpcErrorCode::InternalError,
                    audio_persist_detail(outcome),
                ),
            },
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "set_mute" => match json::parse_mute(arguments) {
            Ok(muted) => match crate::audio::persist_mute(muted).await {
                crate::audio::AudioPersistOutcome::Persisted => {
                    render_success(id, &render_tool_text_result("mute persisted"))
                }
                outcome => render_error(
                    Some(id),
                    JsonRpcErrorCode::InternalError,
                    audio_persist_detail(outcome),
                ),
            },
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "create_reminder" => match json::parse_create_reminder(arguments) {
            Ok(req) => {
                let create_req = crate::reminders::CreateRequest {
                    fire_in_secs: u64::from(req.fire_in_secs),
                    phrase: req.phrase,
                };
                match crate::reminders::add_reminder(embassy_time::Instant::now(), create_req) {
                    Ok(reminder_id) => {
                        let body = format!(r#"{{"id":{reminder_id}}}"#);
                        render_success(id, &render_tool_text_result(&body))
                    }
                    Err(e) => render_error(
                        Some(id),
                        JsonRpcErrorCode::InvalidParams,
                        reminder_error_detail(e),
                    ),
                }
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "list_reminders" => {
            let body = render_reminders_json(&crate::reminders::list_reminders());
            render_success(id, &render_tool_text_result(&body))
        }
        "cancel_reminder" => match json::parse_cancel_reminder(arguments) {
            Ok(reminder_id) => {
                if crate::reminders::cancel_reminder(reminder_id) {
                    render_success(id, &render_tool_text_result("reminder cancelled"))
                } else {
                    render_error(
                        Some(id),
                        JsonRpcErrorCode::InvalidParams,
                        "no reminder with that id",
                    )
                }
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "take_photo" => {
            // Trigger the camera-task capture path; the actual SD
            // write happens out-of-band ~200–500 ms later. The MCP
            // client polls `GET /camera/snapshot` to retrieve it.
            crate::camera::CAMERA_CAPTURE_REQUEST.signal(());
            defmt::info!("mcp: take_photo → camera capture queued");
            render_success(
                id,
                &render_tool_text_result(
                    r#"{"url":"/camera/snapshot","format":"rgb565be","width":320,"height":240,"note":"available within ~500ms"}"#,
                ),
            )
        }
        "sleep" => {
            crate::sleep::SLEEP_SIGNAL.signal(crate::sleep::SleepState::Sleeping);
            defmt::info!("mcp: sleep → entering sleep");
            render_success(id, &render_tool_text_result("entering sleep"))
        }
        "wake" => {
            crate::sleep::SLEEP_SIGNAL.signal(crate::sleep::SleepState::Awake);
            defmt::info!("mcp: wake → exiting sleep");
            render_success(id, &render_tool_text_result("exiting sleep"))
        }
        "get_state" => {
            let snap = snapshot::read();
            render_success(id, &render_tool_text_result(&state_body(snap)))
        }
        _ => render_error(Some(id), JsonRpcErrorCode::MethodNotFound, "unknown tool"),
    }
}

/// Map a [`crate::reminders::ReminderError`] to a `&'static str`
/// MCP error detail. Mirrors [`audio_persist_detail`] in shape.
const fn reminder_error_detail(e: crate::reminders::ReminderError) -> &'static str {
    use crate::reminders::ReminderError;
    match e {
        ReminderError::NotInTheFuture => "fire_in_secs must be > 0",
        ReminderError::HorizonExceeded => "fire_in_secs exceeds the 5-day horizon",
        ReminderError::QueueFull => "reminder list is full",
    }
}

/// Render the reminder list as a compact JSON object — no allocation
/// beyond the `String` — for the MCP `list_reminders` text payload.
fn render_reminders_json(
    list: &heapless::Vec<crate::reminders::Reminder, { crate::reminders::MAX_REMINDERS }>,
) -> String {
    use core::fmt::Write as _;
    let mut out = String::new();
    out.push_str(r#"{"reminders":["#);
    let now = embassy_time::Instant::now();
    for (idx, r) in list.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        // Convert the monotonic deadline back into a relative
        // "fires in N seconds" so the operator-facing surface stays
        // shielded from boot-relative ticks. Saturating subtraction
        // keeps already-due reminders at 0 instead of underflowing.
        let remaining_secs = r.deadline.saturating_duration_since(now).as_secs();
        let phrase = r.phrase;
        let _ = write!(
            out,
            r#"{{"id":{},"fire_in_secs":{},"phrase":"{}"}}"#,
            r.id,
            remaining_secs,
            json::phrase_wire_str(phrase),
        );
    }
    out.push_str("]}");
    out
}

/// Map a non-`Persisted` [`crate::audio::AudioPersistOutcome`] to a
/// `&'static str` MCP error detail. Mirrors [`audio_persist_to_http`]
/// but in JSON-RPC error-detail form rather than HTTP status codes.
const fn audio_persist_detail(outcome: crate::audio::AudioPersistOutcome) -> &'static str {
    use crate::audio::AudioPersistOutcome;
    match outcome {
        // The success path is filtered out at the call site before we
        // get here. A distinct sentinel makes any accidental future
        // call with `Persisted` self-describing inside a -32603
        // response rather than a misleading "ok".
        AudioPersistOutcome::Persisted => "audio persisted (unexpected for error path)",
        AudioPersistOutcome::NoSnapshot => "config snapshot unavailable",
        AudioPersistOutcome::NoStorage => "no SD card mounted",
        AudioPersistOutcome::WriteFailed => "config write failed",
    }
}

/// Map a `JsonError` from the JSON parser into a static detail
/// string. The parser's error variants don't carry message text, so
/// we lookup-table here. Returns `&'static str` so the
/// `render_error` call doesn't need a trailing `String`.
const fn tool_parse_detail(e: &JsonError) -> &'static str {
    use JsonError as E;
    match e {
        E::NotAnObject => "params is not an object",
        E::Unterminated => "unterminated JSON",
        E::MissingKey(_) => "missing required key",
        E::UnknownKey => "unknown key in params",
        E::DuplicateKey(_) => "duplicate key in params",
        E::BadValue => "wrong type or out-of-range value",
        E::UnknownEmotion => "unknown emotion",
        E::UnknownPhrase => "unknown phrase",
        E::UnknownLocale => "unknown locale",
        E::UnknownMood => "unknown mood",
        E::UnknownPalette => "unknown palette",
        E::UnknownFaceGeometry => "unknown face geometry",
        E::UnknownMotion => "unknown motion",
        E::VolumeOutOfRange(_) => "volume out of range",
    }
}

/// `POST /camera/mode` — parse `{"enabled": <bool>}`, update the
/// avatar snapshot, and signal the render task to flip the LCD.
/// Display-only — tracking continues in either mode. No SD writeback;
/// camera mode is intentionally ephemeral so a power-cycle returns
/// to avatar view.
async fn handle_post_camera_mode(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    let active = match json::parse_camera_mode(body) {
        Ok(a) => a,
        Err(e) => {
            defmt::warn!(
                "http: POST /camera/mode parse failed ({})",
                defmt::Debug2Format(&e)
            );
            let body = format!("invalid request body: {e:?}\n");
            return write_text(socket, 400, &body).await;
        }
    };
    snapshot::update_camera_mode(active);
    crate::camera::CAMERA_MODE_SIGNAL.signal(active);
    defmt::info!("http: POST /camera/mode → {=bool}", active);
    write_no_content(socket).await
}

/// `GET /camera/snapshot` — read `/sd/CAPTURE.565` (the most recent
/// frame written by `POST /camera/capture`) and return it as raw
/// QVGA RGB565 big-endian bytes. The dashboard renders these onto a
/// canvas after fetching.
///
/// Returns `404 Not Found` if no capture exists yet (the
/// before-first-capture state). The interaction model is "POST
/// /camera/capture → wait a moment → GET /camera/snapshot" — the
/// snapshot endpoint always reads the SD copy rather than holding a
/// live frame in RAM.
async fn handle_get_camera_snapshot(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    let frame =
        match crate::storage::with_storage(crate::storage::FirmwareStorage::read_capture).await {
            Some(Ok(Some(frame))) => frame,
            Some(Ok(None)) => {
                return write_text(
                    socket,
                    404,
                    "no capture available — POST /camera/capture first\n",
                )
                .await;
            }
            Some(Err(e)) => {
                defmt::warn!(
                    "http: GET /camera/snapshot SD read failed ({})",
                    defmt::Debug2Format(&e)
                );
                return write_text(socket, 503, "SD read failed\n").await;
            }
            None => {
                return write_text(socket, 503, "storage unavailable\n").await;
            }
        };
    // Frame layout headers let an MCP / curl client know exactly
    // what the byte stream is without a separate descriptor. Format
    // is fixed: 320 × 240 RGB565 big-endian, 153 600 bytes total —
    // matches what the GC0308 capture path writes.
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
         X-Frame-Format: rgb565be\r\nX-Frame-Width: 320\r\nX-Frame-Height: 240\r\n\
         Content-Length: {len}\r\nConnection: close\r\n\r\n",
        len = frame.len(),
    );
    socket
        .write_all(header.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;
    socket
        .write_all(&frame)
        .await
        .map_err(|_| HttpError::Write)?;
    socket.flush().await.map_err(|_| HttpError::Write)
}

/// `POST /camera/capture` — empty body, signals the camera task to
/// snapshot the latest frame and persist it to `/sd/CAPTURE.565`.
/// The actual SD write happens out-of-band (a few hundred ms later
/// inside the camera task), so this returns `202 Accepted` rather
/// than blocking the HTTP worker on the SPI write.
async fn handle_post_camera_capture(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    crate::camera::CAMERA_CAPTURE_REQUEST.signal(());
    defmt::info!("http: POST /camera/capture → signal queued");
    write_text(socket, 202, "capture queued\n").await
}

/// Cap on the OTA request body. Sized for the SCFW header (12) +
/// the worst-case payload (`MAX_OTA_PAYLOAD_BYTES`) + the signature
/// (64). Bigger requests get a `413` before any allocation happens
/// — defends the worker against a malicious operator hammering the
/// PSRAM allocator.
const MAX_OTA_REQUEST_BYTES: usize = stackchan_net::ota::OTA_HEADER_LEN
    + stackchan_net::ota::MAX_OTA_PAYLOAD_BYTES as usize
    + stackchan_net::ota::OTA_SIGNATURE_LEN;

/// Stack-resident chunk size for the streaming socket→Vec read.
/// 1 KiB matches the request-buffer convention used by the rest of
/// the HTTP module without any new constants.
const OTA_RECV_CHUNK_BYTES: usize = 1024;

/// Single-flight latch on `POST /firmware/update`.
///
/// `HTTP_WORKER_COUNT` workers can otherwise each spin up their own
/// PSRAM-backed body buffer in parallel, OOM-ing the heap on a
/// flood of concurrent OTA requests (4 × ~4 MiB > available
/// PSRAM after the framebuffer + heap overhead). One in-flight
/// upload at a time is the right concurrency for an irreversible
/// reboot anyway.
static OTA_IN_FLIGHT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// RAII helper that releases the `OTA_IN_FLIGHT` latch on every
/// non-reboot exit path of [`handle_post_firmware_update`]. A
/// successful flash soft-resets before drop runs, which is fine —
/// the latch's only job is to gate concurrent attempts in the
/// pre-reboot window.
struct OtaInFlightGuard;

impl Drop for OtaInFlightGuard {
    fn drop(&mut self) {
        OTA_IN_FLIGHT.store(false, core::sync::atomic::Ordering::Release);
    }
}

/// `POST /firmware/update` — accept an SCFW-framed firmware image,
/// verify the ed25519 signature against the build-time public key,
/// stream the payload into the inactive OTA slot, flip the
/// `otadata` pointer, and soft-reset.
///
/// Always requires a non-empty `Authorization: Bearer <token>` —
/// even when the global token is empty, OTA is destructive and
/// shouldn't ride the LAN-open default. The auth gate matches the
/// `/restart` and `/factory-reset` discipline.
///
/// Returns `503 Service Unavailable` when OTA was compiled out (no
/// `STACKCHAN_OTA_PUBLIC_KEY` env var at build time). Returns `413`
/// for oversize bodies, `400` for SCFW framing failures, `403` for
/// signature mismatches, and `500` for flash-write failures.
async fn handle_post_firmware_update(
    socket: &mut TcpSocket<'_>,
    content_length: usize,
    already_buffered: &[u8],
    auth_token: Option<&str>,
) -> Result<(), HttpError> {
    use core::sync::atomic::Ordering;
    if OTA_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        defmt::warn!("http: POST /firmware/update — refused, another OTA in flight");
        // The in-flight latch releases on every non-reboot exit
        // (auth fail, signature mismatch, flash error). Operators
        // who hit this message can retry as soon as the prior
        // request finishes — only a successful flash blocks them
        // until the soft-reset cycles the device.
        return write_text(
            socket,
            409,
            "ota already in flight; retry shortly or after reboot\n",
        )
        .await;
    }
    // Defer-style guard: clear the latch on every exit. Successful
    // updates soft-reset before this runs (so the latch never
    // releases — which is correct: the reboot is the only outcome
    // we want after a verified flash).
    let _release = OtaInFlightGuard;
    if !crate::ota::ota_enabled() {
        defmt::warn!("http: POST /firmware/update — OTA compiled out");
        return write_text(
            socket,
            503,
            "ota disabled in this build (STACKCHAN_OTA_PUBLIC_KEY unset)\n",
        )
        .await;
    }

    // Always-auth gate — non-empty token required, even when the
    // global token is empty. Mirrors `/restart` / `/factory-reset`.
    let snapshot = crate::storage::CONFIG_SNAPSHOT.lock().await;
    let configured_token = snapshot
        .as_ref()
        .map(|c| c.auth.token.clone())
        .unwrap_or_default();
    drop(snapshot);
    if configured_token.is_empty() {
        defmt::warn!("http: POST /firmware/update — auth token not configured");
        return Err(HttpError::Unauthorized);
    }
    let authorized = auth_token.is_some_and(|t| ct_eq(t.as_bytes(), configured_token.as_bytes()));
    if !authorized {
        return Err(HttpError::Unauthorized);
    }

    if content_length > MAX_OTA_REQUEST_BYTES {
        return write_text(socket, 413, "ota image exceeds the 4 MiB cap\n").await;
    }
    if content_length < stackchan_net::ota::OTA_HEADER_LEN + stackchan_net::ota::OTA_SIGNATURE_LEN {
        return write_text(socket, 400, "ota image truncated\n").await;
    }

    // Allocate on the heap; PSRAM absorbs the multi-MB body without
    // pressuring internal SRAM.
    let mut body: alloc::vec::Vec<u8> = alloc::vec::Vec::with_capacity(content_length);
    body.extend_from_slice(already_buffered);

    let mut chunk = [0u8; OTA_RECV_CHUNK_BYTES];
    while body.len() < content_length {
        let want = content_length - body.len();
        let take = want.min(OTA_RECV_CHUNK_BYTES);
        let n = match socket.read(&mut chunk[..take]).await {
            Ok(n) if n > 0 => n,
            _ => return Err(HttpError::Read),
        };
        body.extend_from_slice(&chunk[..n]);
    }

    defmt::info!(
        "http: POST /firmware/update — body received ({=usize} bytes), verifying",
        body.len()
    );
    crate::event_log::record_fmt(
        crate::event_log::Kind::Control,
        format_args!("POST /firmware/update {} bytes", body.len()),
    );

    match crate::ota::perform_update(&body) {
        Ok(()) => {
            defmt::info!(
                "http: OTA flush succeeded — soft-resetting in 200 ms to boot the new image"
            );
            // Free the body buffer before reset so PSRAM is in a
            // known-empty state if the bootloader leaves it
            // initialised across the soft-reset.
            drop(body);
            write_text(socket, 200, "ota verified + flashed; rebooting\n").await?;
            socket.flush().await.map_err(|_| HttpError::Write)?;
            embassy_time::Timer::after(embassy_time::Duration::from_millis(200)).await;
            esp_hal::system::software_reset()
        }
        Err(e) => {
            defmt::warn!("http: OTA failed: {}", e);
            let (status, msg) = match &e {
                crate::ota::OtaPerformError::Disabled => (503, "ota disabled in this build\n"),
                // Both flash-state errors are 503 (service
                // unavailable) — `FlashUnavailable` is a build-time
                // miswiring, `FlashConsumedThisBoot` is a runtime
                // exhaustion; both are operationally "OTA can't
                // serve this request" rather than "server bug".
                crate::ota::OtaPerformError::FlashUnavailable => {
                    (503, "flash peripheral unavailable\n")
                }
                crate::ota::OtaPerformError::FlashConsumedThisBoot => (
                    503,
                    "flash consumed by a prior failed ota; reboot to retry\n",
                ),
                // Signature mismatch is the most operator-actionable
                // error path — surface it as 403 specifically.
                crate::ota::OtaPerformError::Image(stackchan_net::ota::OtaImageError::Verify(
                    _,
                )) => (403, "signature verification failed\n"),
                crate::ota::OtaPerformError::Image(_) => (400, "image rejected\n"),
                crate::ota::OtaPerformError::Flash(_) => (500, "flash write failed\n"),
            };
            write_text(socket, status, msg).await
        }
    }
}

/// `POST /restart` — write the response, briefly let the TCP buffer
/// drain, then trigger an `esp_hal::reset::software_reset`.
///
/// The flush + 200 ms timer matter: without them, calling
/// `software_reset` immediately after `write_all` returns would yank
/// the chip mid-FIN — the dashboard would see a TCP RST instead of
/// the response body, and the toast would be a 'connection reset'
/// instead of 'rebooting'.
async fn handle_post_restart(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
    defmt::info!("http: POST /restart → soft reset in 200 ms");
    crate::event_log::record(crate::event_log::Kind::Lifecycle, "restart requested");
    write_text(socket, 202, "rebooting\n").await?;
    embassy_time::Timer::after(embassy_time::Duration::from_millis(200)).await;
    esp_hal::system::software_reset();
}

/// `POST /factory-reset` — gated by token (always required, even when
/// the global `auth.token` is empty) plus a body-confirm phrase
/// `{"confirm":"erase"}`.
///
/// Wipes every operator-visible file on the SD (`STACKCHAN.RON`,
/// `BONDS.BIN`, `CAPTURE.565`, plus the staging files) and then soft-
/// resets so the boot path comes up against defaults.
async fn handle_post_factory_reset(
    socket: &mut TcpSocket<'_>,
    body: &str,
) -> Result<(), HttpError> {
    if !body_confirms_erase(body) {
        return write_text(
            socket,
            400,
            "factory-reset requires body {\"confirm\":\"erase\"}\n",
        )
        .await;
    }
    crate::event_log::record(crate::event_log::Kind::Lifecycle, "factory-reset requested");
    let wipe = crate::storage::with_storage(crate::storage::FirmwareStorage::factory_reset).await;
    match wipe {
        Some(Ok(())) => {
            defmt::warn!("http: POST /factory-reset wiped SD; resetting in 200 ms");
            write_text(socket, 202, "wiped, rebooting\n").await?;
            embassy_time::Timer::after(embassy_time::Duration::from_millis(200)).await;
            esp_hal::system::software_reset();
        }
        Some(Err(e)) => {
            defmt::warn!("http: POST /factory-reset wipe failed ({})", e);
            write_text(socket, 500, "wipe failed\n").await
        }
        None => {
            defmt::warn!("http: POST /factory-reset rejected (no SD mounted)");
            write_text(socket, 503, "no SD card mounted\n").await
        }
    }
}

/// Look for a `"confirm":"erase"` key/value pair anywhere in the
/// body. The hand-rolled parsers in `stackchan-net` are JSON-aware,
/// but the match here is loose on purpose — we only care whether the
/// operator typed the magic key/value sequence, not whether it sits
/// at exactly the right JSON depth.
///
/// Tolerates whitespace between the colon and the value (`"confirm"
/// : "erase"`) but requires the two tokens to be paired — an unrelated
/// `"erase"` somewhere else in the body, with `"confirm"` mapped to a
/// different value, will not pass.
fn body_confirms_erase(body: &str) -> bool {
    // Find `"confirm"` occurrences and check that the next non-space
    // token is `:"erase"` (allowing optional whitespace either side
    // of the colon).
    let key = "\"confirm\"";
    let mut search = body;
    while let Some(idx) = search.find(key) {
        let after = &search[idx + key.len()..];
        let trimmed = after.trim_start();
        if let Some(rest) = trimmed.strip_prefix(':') {
            let v = rest.trim_start();
            if v.starts_with("\"erase\"") {
                return true;
            }
        }
        search = &search[idx + key.len()..];
    }
    false
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
            // EnterPairing has two consumers: the avatar-side modifier
            // (visual decorator + chirp) and the radio-side ESP-NOW
            // task (open peer-registration window). Signal both before
            // forwarding the command to the modifier.
            if let RemoteCommand::EnterPairing { duration_ms } = cmd {
                crate::net::esp_now::open_pair_window(duration_ms);
            }
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
fn sensors_body(s: snapshot::SensorsSnapshot) -> String {
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
fn tasks_body(s: crate::watchdog::TasksSnapshot) -> String {
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
fn events_body() -> String {
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
fn json_escape(s: &str) -> String {
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

/// Serve [`DASHBOARD_GZ`] with `Content-Type: text/html` and
/// `Content-Encoding: gzip`. Cache is disabled so a freshly flashed
/// firmware's dashboard JS shows up on the next reload — the payload
/// is small over LAN, so a longer max-age was never worth the
/// staleness it caused.
async fn write_dashboard(socket: &mut TcpSocket<'_>) -> Result<(), HttpError> {
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
