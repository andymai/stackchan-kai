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
//! - `GET /sensor-history` — rolling 60 s window of one-per-second
//!   sensor snapshots, oldest first. Same field shape as `/sensors`
//!   per entry, plus an `age_secs` for each.
//! - `GET /tasks` — watchdog channel health: per-task heartbeat
//!   delta in the last window + a `stale` flag.
//! - `GET /hardware/status` — boot-time servo-power-rail health:
//!   whether the PY32 enable succeeded, attempt count, settle flag.
//!   Field-debugging aid for a head that won't move.
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
//! - `POST /behavior` — JSON `{"field": "<name>", "value": <bool>}`.
//!   Toggles one boolean flag in `behavior` (soliloquy / hourly
//!   chime / battery icon / toast overlay) and persists to
//!   `/sd/STACKCHAN.RON`. Each of these is captured at boot by its
//!   consuming task or modifier, so the change takes effect on the
//!   next reboot — the response carries `{"reboot_required": true}`
//!   to make that explicit. The reboot-only `wake_word_*` integers
//!   stay behind `PUT /settings`.
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
//! through [`REMOTE_COMMAND_QUEUE`]; the render task drains it
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
use embassy_sync::channel::Channel;
use embassy_sync::pubsub::WaitResult;
use embassy_sync::signal::Signal;
use embassy_time::Duration;
use embedded_io_async::Write as AsyncWrite;
use stackchan_core::{Clock as _, RemoteCommand};
use stackchan_net::http_command::{self as json, JsonError};
use stackchan_net::http_parse::{
    ct_eq, find_subsequence, parse_bearer_token, parse_content_length,
};

use super::respond::{
    HttpError, events_body, hardware_status_body, health_body, sensor_history_body, sensors_body,
    state_body, tasks_body, write_dashboard, write_json, write_no_content, write_status_for_error,
    write_text,
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
const REQUEST_BUF_BYTES: usize = 2048;

/// Cap on the `Content-Length` header. Bodies of this size or
/// larger are rejected before any body bytes are read.
///
/// Equal to [`REQUEST_BUF_BYTES`] on purpose: the buffer holds
/// headers + body together, so any `content_length` that hits the
/// cap can't physically fit alongside the request line. Sized for
/// `PUT /settings`: the full schema-v1 body with a 32-char SSID,
/// 63-char WPA2 PSK, an `America/…` IANA tz label, a few SNTP
/// servers, and the variable-length `agent_sidecar_url` lands
/// around 600 bytes; the 2048 ceiling leaves room for future
/// fields without forcing every operator update through a re-cap.
const MAX_BODY_BYTES: usize = 2048;

/// Capacity of [`REMOTE_COMMAND_QUEUE`]. Sized for typical bursts
/// from ~14 producers; the render loop drains all pending entries
/// each tick (~33 ms) so the queue rarely sits more than 1–2 deep.
const REMOTE_COMMAND_QUEUE_CAPACITY: usize = 8;

/// Control-plane command queue.
///
/// HTTP / MCP fan-in routes through here from many producer sites:
/// every operator command, plus wake-word fire, sidecar
/// `EnterThinking`, `mDNS` follower, `BluFi` GATT, `ESP-NOW` peer,
/// and the desktop-protocol bridge. Single consumer is the render
/// loop, which drains via [`embassy_sync::channel::Channel::try_receive`].
///
/// Was a [`Signal<_, RemoteCommand>`][`Signal`] until 2026-05-19;
/// a single Signal slot is last-write-wins, so two MCP tools fired
/// back-to-back within the ~33 ms drain interval would silently
/// drop the first. With a bounded channel the queue absorbs short
/// bursts. On overflow [`enqueue_remote_command`] logs and drops
/// the new value (drop-newest matches Signal's pre-existing
/// saturation behaviour), so producers never block.
pub static REMOTE_COMMAND_QUEUE: Channel<
    CriticalSectionRawMutex,
    RemoteCommand,
    REMOTE_COMMAND_QUEUE_CAPACITY,
> = Channel::new();

/// Enqueue one [`RemoteCommand`] onto [`REMOTE_COMMAND_QUEUE`].
///
/// Fire-and-forget: producers are called from many task contexts
/// (interrupt-adjacent BLE callbacks, render-loop adjacent
/// shortcuts, async HTTP handlers, …) and must never block. On
/// queue-full the value is dropped and a `defmt::warn!` surfaces
/// the saturation so the operator can detect a runaway producer
/// without it crashing the device.
pub fn enqueue_remote_command(cmd: RemoteCommand) {
    use embassy_sync::channel::TrySendError;
    if let Err(TrySendError::Full(_)) = REMOTE_COMMAND_QUEUE.try_send(cmd) {
        defmt::warn!(
            "remote-command: queue full (cap={=usize}); dropped command",
            REMOTE_COMMAND_QUEUE_CAPACITY,
        );
    }
}

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
        ("GET", "/state/ws") => {
            handle_state_websocket(socket, &buf[line_end + 2..header_end]).await
        }
        ("GET", "/sensors") => {
            write_json(socket, 200, &sensors_body(snapshot::read_sensors())).await
        }
        ("GET", "/sensor-history") => {
            let history = crate::sensor_history::read_history();
            let now_ms = embassy_time::Instant::now().as_millis();
            write_json(socket, 200, &sensor_history_body(&history, now_ms)).await
        }
        ("GET", "/tasks") => {
            write_json(
                socket,
                200,
                &tasks_body(crate::watchdog::read_tasks_snapshot()),
            )
            .await
        }
        ("GET", "/hardware/status") => {
            write_json(
                socket,
                200,
                &hardware_status_body(crate::servo_power::read_status()),
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
        ("POST", "/behavior") => handle_post_behavior(socket, body).await,
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

/// WebSocket ping interval. Same 15 s cadence as the SSE heartbeat
/// for the same reason — keeps proxy / NAT idle timers from
/// tearing the connection down.
const WS_PING_INTERVAL_SECS: u64 = 15;

/// `GET /state/ws` — upgrade the connection to a WebSocket and
/// push [`AvatarSnapshot`] events as text frames.
///
/// Mirrors [`handle_state_stream`] (SSE) over RFC 6455 framing.
/// Operators choose the transport their dashboard speaks; both
/// subscribe to the same [`super::snapshot::SNAPSHOT_PUBSUB`] so
/// payloads are bit-identical.
///
/// Pre-handshake the request body is empty (`Content-Length: 0`),
/// so the bearer-token gate higher up doesn't apply — `GET` reads
/// stay LAN-open regardless of `auth.token`.
async fn handle_state_websocket(
    socket: &mut TcpSocket<'_>,
    headers: &[u8],
) -> Result<(), HttpError> {
    use core::fmt::Write as _;

    use super::websocket;

    // RFC 6455 § 4.2.1: the server must validate `Upgrade:
    // websocket`, `Connection: Upgrade`, `Sec-WebSocket-Version:
    // 13`, and presence of `Sec-WebSocket-Key` before issuing
    // `101`. Without the version gate a client on an older draft
    // would receive a `101` with framing it doesn't understand
    // and silently break — a clean `400` lets the client fall
    // back. The `Connection` header is a comma-separated list per
    // RFC 9110 § 7.6.1, so a client legitimately sending
    // `Connection: Upgrade, keep-alive` must still pass.
    let upgrade_ok = websocket::parse_header_value(headers, b"upgrade")
        .is_some_and(|v| v.eq_ignore_ascii_case(b"websocket"));
    if !upgrade_ok {
        return write_text(socket, 400, "Upgrade: websocket required\n").await;
    }
    let connection_ok = websocket::parse_header_value(headers, b"connection")
        .is_some_and(|v| websocket::header_contains_token(v, b"upgrade"));
    if !connection_ok {
        return write_text(socket, 400, "Connection: Upgrade required\n").await;
    }
    let version_ok =
        websocket::parse_header_value(headers, b"sec-websocket-version") == Some(b"13" as &[u8]);
    if !version_ok {
        return write_text(socket, 400, "Sec-WebSocket-Version: 13 required\n").await;
    }
    let Some(key) = websocket::parse_websocket_key(headers) else {
        return write_text(socket, 400, "missing Sec-WebSocket-Key\n").await;
    };
    let Ok(mut subscriber) = snapshot::SNAPSHOT_PUBSUB.subscriber() else {
        return write_text(socket, 503, "stream slots exhausted\n").await;
    };

    // The handshake response is small; build it in a heapless
    // buffer so the 101 + Upgrade + Accept header lines never
    // touch the allocator.
    let accept = websocket::compute_accept_key(key);
    let mut response: heapless::String<160> = heapless::String::new();
    if write!(
        &mut response,
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\r\n",
        accept.as_str(),
    )
    .is_err()
    {
        return Err(HttpError::Write);
    }
    socket
        .write_all(response.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;
    socket.flush().await.map_err(|_| HttpError::Write)?;

    // After the upgrade the connection is server-push only; the
    // per-request inactivity timeout would tear a healthy
    // long-lived stream down.
    socket.set_timeout(None);

    // Initial snapshot so freshly-connected clients don't wait
    // for the next render tick to render their first frame.
    let body = state_body(snapshot::read());
    websocket::write_text_frame(socket, body.trim_end_matches('\n').as_bytes())
        .await
        .map_err(|()| HttpError::Write)?;

    loop {
        match select(
            subscriber.next_message(),
            embassy_time::Timer::after(Duration::from_secs(WS_PING_INTERVAL_SECS)),
        )
        .await
        {
            Either::First(WaitResult::Message(snap)) => {
                let body = state_body(snap);
                websocket::write_text_frame(socket, body.trim_end_matches('\n').as_bytes())
                    .await
                    .map_err(|()| HttpError::Write)?;
            }
            // `Lagged` means we missed N publishes. Same as SSE
            // — the next snapshot is more useful than backfilling.
            Either::First(WaitResult::Lagged(_)) => {}
            Either::Second(()) => {
                websocket::write_ping_frame(socket)
                    .await
                    .map_err(|()| HttpError::Write)?;
            }
        }
    }
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
/// **Live-applicable** fields, signalled through to the running task on
/// change:
/// - `wifi.ssid` / `wifi.psk` (via `WIFI_RECONFIG`)
/// - `audio.volume_pct` / `audio.muted` (via the audio task's signal
///   pair, mutated through `POST /volume` / `POST /mute`)
/// - `auth.token` (lock-free read on the next request)
///
/// **Reboot-only** fields — captured once at task spawn or modifier
/// instantiation, so a change requires a reboot to take effect:
/// - `mdns.hostname` (mdns task arg at spawn)
/// - `time.tz` / `time.sntp_servers` (SNTP task args at spawn)
/// - `tracker.*` (tracker overlay reads once at boot)
/// - `behavior.wake_word_enabled` / `wake_word_threshold` /
///   `wake_word_arena_kib` (wake-word task reads once + allocates
///   the tensor arena at spawn)
/// - `behavior.soliloquy_enabled` / `battery_icon_enabled` /
///   `toast_overlay_enabled` (each modifier instance captures the
///   flag at boot — see `main.rs` render-task setup)
/// - `behavior.hourly_chime_enabled` (chime task arg at spawn)
/// - `behavior.auto_torque_release_ms` (head task reads once at boot)
/// - `behavior.audio_debug_udp_target` (`audio_debug` task arg at spawn)
/// - `behavior.agent_sidecar_url` / `agent_sidecar_token` /
///   `persona_name` (agent sidecar task args at spawn)
/// - `behavior.follower_leader_hostname` (follower task arg at spawn)
/// - `behavior.voicevox_url` / `voicevox_speaker_id` (`VoiceVox`
///   synthesis task args at spawn)
///
/// Future work that wants any of these to apply live needs both a
/// signal channel from this handler AND a snapshot re-read in the
/// consuming task; today it's simpler to surface the reboot
/// requirement honestly.
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
    let b_prev = &prev.behavior;
    let b_new = &new.behavior;
    if b_prev.wake_word_enabled != b_new.wake_word_enabled
        || b_prev.wake_word_threshold != b_new.wake_word_threshold
        || b_prev.wake_word_arena_kib != b_new.wake_word_arena_kib
        || b_prev.soliloquy_enabled != b_new.soliloquy_enabled
        || b_prev.hourly_chime_enabled != b_new.hourly_chime_enabled
        || b_prev.battery_icon_enabled != b_new.battery_icon_enabled
        || b_prev.toast_overlay_enabled != b_new.toast_overlay_enabled
        || b_prev.auto_torque_release_ms != b_new.auto_torque_release_ms
        || b_prev.audio_debug_udp_target != b_new.audio_debug_udp_target
        || b_prev.agent_sidecar_url != b_new.agent_sidecar_url
        || b_prev.agent_sidecar_token != b_new.agent_sidecar_token
        || b_prev.follower_leader_hostname != b_new.follower_leader_hostname
        || b_prev.persona_name != b_new.persona_name
        || b_prev.voicevox_url != b_new.voicevox_url
        || b_prev.voicevox_speaker_id != b_new.voicevox_speaker_id
    {
        return true;
    }
    false
}

/// `PUT /settings` — block-level merge, atomic SD writeback. Blocks
/// present in the body replace wholesale; optional blocks omitted
/// from the body keep their currently-persisted values (a trimmed
/// curl body must not wipe tracker calibration or ESP-NOW keys).
/// Returns `{"reboot_required": <bool>}` where `<bool>` reflects
/// whether any changed field can only take effect on the next boot
/// (mDNS hostname, SNTP, tracker tuning) — Wi-Fi creds and audio
/// still apply immediately via the existing signal paths.
///
/// On a change to `wifi.ssid` or `wifi.psk` (compared against the
/// current `CONFIG_SNAPSHOT`), signals [`WIFI_RECONFIG`] so the
/// wifi task drops the link and reconnects with the new creds.
/// Operators changing the AP from the dashboard now see a brief
/// link blip rather than needing to power-cycle the device.
async fn handle_put_settings(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    // Track whether the snapshot was actually populated so the
    // reboot-required diff doesn't compare against a synthesized
    // default (which would falsely flag every first-boot PUT as
    // requiring reboot just because the new value differs from the
    // struct default). With no current snapshot (the brief
    // pre-storage-mount window), both the omitted-block fill and the
    // sentinel merge run against defaults — the parsed body wins.
    let prior_snapshot = crate::storage::CONFIG_SNAPSHOT.lock().await.clone();
    let had_prior_snapshot = prior_snapshot.is_some();
    let snapshot_for_merge = prior_snapshot.unwrap_or_default();
    let parsed_config =
        match stackchan_net::parse_settings_json_with_current(body, &snapshot_for_merge) {
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
    // doesn't clobber them.
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

/// Outcome of a single-field behavior-flag persist. Mirrors the
/// [`crate::audio::AudioPersistOutcome`] shape so HTTP / MCP error
/// surfaces stay symmetric across `POST /behavior` and `POST /volume`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
enum BehaviorPersistOutcome {
    /// Config was written and `CONFIG_SNAPSHOT` updated. The change is
    /// on disk but the running consumers (soliloquy / hourly chime /
    /// battery overlay / toast overlay) each captured the flag at boot
    /// — see [`requires_reboot`] for the full list — so a reboot is
    /// needed before the change takes effect.
    Persisted,
    /// `CONFIG_SNAPSHOT` was empty — boot hadn't loaded any config
    /// yet, so synthesising a default-everything-else config to write
    /// would clobber whatever is on disk.
    NoSnapshot,
    /// SD card isn't mounted — there's nowhere to persist.
    NoStorage,
    /// SD write failed mid-transaction. Cache stays at the previous
    /// value rather than partially applying.
    WriteFailed,
}

/// Apply one [`stackchan_net::http_command::BehaviorFlagUpdate`] to
/// the live `CONFIG_SNAPSHOT` + SD-backed `STACKCHAN.RON`.
///
/// Shared by `POST /behavior` and the MCP `set_behavior_flag` tool so
/// operator-driven curl and LLM-driven tool calls take identical
/// code paths. Mirrors the volume / mute persist shape.
///
/// Callers ask the update itself whether to surface `reboot_required`
/// via [`stackchan_net::http_command::BehaviorFlagUpdate::requires_reboot`]
/// — a future enum variant that's live-applicable opts out there
/// rather than forcing edits in two routes.
async fn persist_behavior_flag(
    update: stackchan_net::http_command::BehaviorFlagUpdate,
) -> BehaviorPersistOutcome {
    let Some(current) = crate::storage::CONFIG_SNAPSHOT.lock().await.clone() else {
        return BehaviorPersistOutcome::NoSnapshot;
    };
    let mut new_config = current;
    update.apply(&mut new_config.behavior);
    let write_result =
        crate::storage::with_storage(|storage| storage.write_config(&new_config)).await;
    match write_result {
        Some(Ok(())) => {
            defmt::info!(
                "behavior: {=str} persisted (value={=bool})",
                update.field_name(),
                update.value(),
            );
            *crate::storage::CONFIG_SNAPSHOT.lock().await = Some(new_config);
            BehaviorPersistOutcome::Persisted
        }
        Some(Err(e)) => {
            defmt::warn!("behavior: write failed ({})", e);
            BehaviorPersistOutcome::WriteFailed
        }
        None => BehaviorPersistOutcome::NoStorage,
    }
}

/// Map a [`BehaviorPersistOutcome`] to its HTTP response.
async fn behavior_persist_to_http(
    socket: &mut TcpSocket<'_>,
    outcome: BehaviorPersistOutcome,
    reboot_required: bool,
) -> Result<(), HttpError> {
    match outcome {
        // Response shape mirrors `PUT /settings` so a dashboard can
        // show the same reboot nag. The bool came in from the
        // `BehaviorFlagUpdate::requires_reboot` method, so a future
        // live-applicable variant correctly reports `false` here
        // without editing this function.
        BehaviorPersistOutcome::Persisted => {
            let body = format!("{{\"reboot_required\":{reboot_required}}}\n");
            write_json(socket, 200, &body).await
        }
        BehaviorPersistOutcome::NoSnapshot => {
            write_text(socket, 503, "config snapshot unavailable\n").await
        }
        BehaviorPersistOutcome::NoStorage => write_text(socket, 503, "no SD card mounted\n").await,
        BehaviorPersistOutcome::WriteFailed => {
            write_text(socket, 500, "config write failed\n").await
        }
    }
}

/// Map a [`BehaviorPersistOutcome`] to a `&'static str` MCP error
/// detail. Mirrors [`audio_persist_detail`] in shape.
const fn behavior_persist_detail(outcome: BehaviorPersistOutcome) -> &'static str {
    match outcome {
        BehaviorPersistOutcome::Persisted => "behavior persisted (unexpected for error path)",
        BehaviorPersistOutcome::NoSnapshot => "config snapshot unavailable",
        BehaviorPersistOutcome::NoStorage => "no SD card mounted",
        BehaviorPersistOutcome::WriteFailed => "config write failed",
    }
}

/// `POST /behavior` — toggle one boolean flag in `behavior` by name.
/// Body shape per [`stackchan_net::http_command::parse_behavior_flag`]:
/// `{"field": "<name>", "value": <bool>}`. Field vocabulary is the
/// variants of [`stackchan_net::http_command::BehaviorFlagUpdate`].
///
/// On success returns `{"reboot_required": <bool>}`. The bool comes
/// from the [`BehaviorFlagUpdate::requires_reboot`] method on the
/// update itself, so a future variant that's live-applicable gets
/// the right answer without editing this handler.
///
/// [`BehaviorFlagUpdate::requires_reboot`]: stackchan_net::http_command::BehaviorFlagUpdate::requires_reboot
async fn handle_post_behavior(socket: &mut TcpSocket<'_>, body: &str) -> Result<(), HttpError> {
    let update = match json::parse_behavior_flag(body) {
        Ok(u) => u,
        Err(e) => {
            defmt::warn!(
                "http: POST /behavior parse failed ({})",
                defmt::Debug2Format(&e)
            );
            let body = format!("invalid request body: {e:?}\n");
            return write_text(socket, 400, &body).await;
        }
    };
    let reboot_required = update.requires_reboot();
    behavior_persist_to_http(socket, persist_behavior_flag(update).await, reboot_required).await
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
/// [`crate::head::OFFSETS_SIGNAL`]. Persists to `/sd/RUNTIME.RON`
/// so a reboot restores the dialled-in trim.
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
    // The persist outcome is logged by `runtime_store::persist` itself
    // (`info!` on success, `warn!` on SD-write failure) — no need to
    // annotate either side here, which would risk a contradictory
    // ordering in the serial log if the SD write failed.
    let _ = crate::runtime_store::update_head_offsets(firmware_offsets).await;
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
    let request = match json::parse_toast(body) {
        Ok(r) => r,
        Err(e) => {
            defmt::warn!(
                "http: POST /toast parse failed ({})",
                defmt::Debug2Format(&e)
            );
            let msg = format!("invalid toast: {e:?}\n");
            return write_text(socket, 400, &msg).await;
        }
    };
    let Some(level) = toast_level_from_wire(&request.level) else {
        defmt::warn!(
            "http: POST /toast unknown level={=str}",
            request.level.as_str()
        );
        return write_text(
            socket,
            400,
            "level must be \"info\", \"warn\", or \"error\"\n",
        )
        .await;
    };
    crate::toast::push(level, &request.message, crate::clock::HalClock.now());
    defmt::info!(
        "http: POST /toast level={=?} msg={=str}",
        defmt::Debug2Format(&level),
        request.message.as_str()
    );
    write_no_content(socket).await
}

/// Map the wire string from `parse_toast` onto the firmware's
/// `ToastLevel` enum. Shared by `POST /toast` and the MCP
/// `push_toast` tool.
const fn toast_level_from_wire(s: &str) -> Option<crate::toast::ToastLevel> {
    // `str` doesn't `match` ergonomically in a const fn; fall back
    // to byte-equality for the known short labels.
    match s.as_bytes() {
        b"info" => Some(crate::toast::ToastLevel::Info),
        b"warn" => Some(crate::toast::ToastLevel::Warn),
        b"error" => Some(crate::toast::ToastLevel::Error),
        _ => None,
    }
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
/// `REMOTE_COMMAND_QUEUE` and rendering a fixed acknowledgement.
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
                enqueue_remote_command(cmd);
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
                enqueue_remote_command(cmd);
                render_success(id, &render_tool_text_result("look-at enqueued"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "look_at_point" => match json::parse_look_at_point(arguments) {
            Ok(cmd) => {
                enqueue_remote_command(cmd);
                render_success(id, &render_tool_text_result("look-at-point enqueued"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "speak" => match json::parse_speak(arguments) {
            Ok(cmd) => {
                enqueue_remote_command(cmd);
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
        "push_toast" => match json::parse_toast(arguments) {
            Ok(request) => match toast_level_from_wire(&request.level) {
                Some(level) => {
                    crate::toast::push(level, &request.message, crate::clock::HalClock.now());
                    render_success(id, &render_tool_text_result("toast pushed"))
                }
                None => render_error(
                    Some(id),
                    JsonRpcErrorCode::InvalidParams,
                    "level must be \"info\", \"warn\", or \"error\"",
                ),
            },
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "start_listen" => match json::parse_start_listen(arguments) {
            Ok(cmd) => {
                enqueue_remote_command(cmd);
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
                enqueue_remote_command(cmd);
                render_success(id, &render_tool_text_result("pairing window opened"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "enter_thinking" => match json::parse_enter_thinking(arguments) {
            Ok(cmd) => {
                enqueue_remote_command(cmd);
                render_success(id, &render_tool_text_result("thinking window opened"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "exit_thinking" => match json::parse_exit_thinking(arguments) {
            Ok(cmd) => {
                enqueue_remote_command(cmd);
                render_success(id, &render_tool_text_result("thinking hold released"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "reset" => match json::parse_reset(arguments) {
            Ok(cmd) => {
                enqueue_remote_command(cmd);
                render_success(id, &render_tool_text_result("holds released"))
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
                    action: crate::reminders::ScheduledAction::Speak(req.phrase),
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
        "schedule_motion" => match json::parse_schedule_motion(arguments) {
            Ok(req) => {
                let create_req = crate::reminders::CreateRequest {
                    fire_in_secs: u64::from(req.fire_in_secs),
                    action: crate::reminders::ScheduledAction::PlayMotion(req.motion),
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
        "get_health" => render_success(id, &render_tool_text_result(&health_body())),
        "get_sensors" => render_success(
            id,
            &render_tool_text_result(&sensors_body(snapshot::read_sensors())),
        ),
        "get_sensor_history" => {
            let history = crate::sensor_history::read_history();
            let now_ms = embassy_time::Instant::now().as_millis();
            render_success(
                id,
                &render_tool_text_result(&sensor_history_body(&history, now_ms)),
            )
        }
        "get_tasks" => render_success(
            id,
            &render_tool_text_result(&tasks_body(crate::watchdog::read_tasks_snapshot())),
        ),
        "get_events" => render_success(id, &render_tool_text_result(&events_body())),
        "get_crash" => {
            let outcome =
                crate::storage::with_storage(crate::storage::FirmwareStorage::read_crash).await;
            match outcome {
                Some(Ok(Some(text))) => render_success(id, &render_tool_text_result(&text)),
                Some(Ok(None)) => render_success(id, &render_tool_text_result("")),
                Some(Err(e)) => {
                    defmt::warn!("mcp: get_crash read failed ({})", defmt::Debug2Format(&e));
                    render_error(
                        Some(id),
                        JsonRpcErrorCode::InternalError,
                        "crash log read failed",
                    )
                }
                None => render_error(
                    Some(id),
                    JsonRpcErrorCode::InternalError,
                    "no SD card mounted",
                ),
            }
        }
        "set_palette" => match json::parse_palette(arguments) {
            Ok(palette) => {
                PALETTE_SIGNAL.signal(palette);
                let _ = crate::runtime_store::update_palette(palette).await;
                render_success(id, &render_tool_text_result("palette enqueued"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "set_face_target" => match json::parse_face_target(arguments) {
            Ok(cmd) => {
                enqueue_remote_command(cmd);
                render_success(id, &render_tool_text_result("face target enqueued"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "set_camera_mode" => match json::parse_camera_mode(arguments) {
            Ok(active) => {
                snapshot::update_camera_mode(active);
                crate::camera::CAMERA_MODE_SIGNAL.signal(active);
                render_success(id, &render_tool_text_result("camera mode enqueued"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "get_head_offsets" => {
            let offsets = crate::head::current_offsets();
            let body = format!(
                r#"{{"yaw_offset_deg":{:.3},"tilt_offset_deg":{:.3}}}"#,
                offsets.yaw_offset_deg, offsets.tilt_offset_deg,
            );
            render_success(id, &render_tool_text_result(&body))
        }
        "set_head_offsets" => match json::parse_head_offsets(arguments) {
            Ok(offsets) => {
                let firmware_offsets = crate::head::HeadOffsets {
                    yaw_offset_deg: offsets.yaw_offset_deg,
                    tilt_offset_deg: offsets.tilt_offset_deg,
                };
                crate::head::OFFSETS_SIGNAL.signal(firmware_offsets);
                crate::head::OFFSETS_CACHE.lock(|cell| cell.set(firmware_offsets));
                let _ = crate::runtime_store::update_head_offsets(firmware_offsets).await;
                render_success(id, &render_tool_text_result("head offsets enqueued"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "clear_crash" => {
            let outcome =
                crate::storage::with_storage(crate::storage::FirmwareStorage::delete_crash).await;
            match outcome {
                Some(Ok(())) => render_success(id, &render_tool_text_result("crash log cleared")),
                Some(Err(e)) => {
                    defmt::warn!("mcp: clear_crash failed ({})", defmt::Debug2Format(&e));
                    render_error(
                        Some(id),
                        JsonRpcErrorCode::InternalError,
                        "crash log delete failed",
                    )
                }
                None => render_error(
                    Some(id),
                    JsonRpcErrorCode::InternalError,
                    "no SD card mounted",
                ),
            }
        }
        "play_dance" => match stackchan_net::dance::parse_dance(arguments) {
            Ok(script) => {
                use alloc::sync::Arc;
                let frames = script.keyframes.len();
                DANCE_SCRIPT_SIGNAL.signal(Arc::new(script));
                defmt::info!("mcp: play_dance → {=usize} keyframes loaded", frames);
                render_success(id, &render_tool_text_result("dance script enqueued"))
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
        "set_behavior_flag" => match json::parse_behavior_flag(arguments) {
            Ok(update) => {
                let reboot_required = update.requires_reboot();
                let outcome = persist_behavior_flag(update).await;
                if outcome == BehaviorPersistOutcome::Persisted {
                    let detail = if reboot_required {
                        "behavior flag persisted; reboot to apply"
                    } else {
                        "behavior flag persisted; applied"
                    };
                    let body =
                        format!(r#"{{"reboot_required":{reboot_required},"detail":"{detail}"}}"#);
                    render_success(id, &render_tool_text_result(&body))
                } else {
                    render_error(
                        Some(id),
                        JsonRpcErrorCode::InternalError,
                        behavior_persist_detail(outcome),
                    )
                }
            }
            Err(e) => render_error(
                Some(id),
                JsonRpcErrorCode::InvalidParams,
                tool_parse_detail(&e),
            ),
        },
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
    use crate::reminders::ScheduledAction;
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
        // Discriminate by which field is present: `phrase` for the
        // Speak action (current `create_reminder` shape, preserved for
        // backward compatibility with existing dashboard JS), `motion`
        // for the PlayMotion action introduced with `schedule_motion`.
        // A consumer can switch on whichever key it finds.
        match r.action {
            ScheduledAction::Speak(phrase) => {
                let _ = write!(
                    out,
                    r#"{{"id":{},"fire_in_secs":{},"phrase":"{}"}}"#,
                    r.id,
                    remaining_secs,
                    json::phrase_wire_str(phrase),
                );
            }
            ScheduledAction::PlayMotion(motion) => {
                let _ = write!(
                    out,
                    r#"{{"id":{},"fire_in_secs":{},"motion":"{}"}}"#,
                    r.id,
                    remaining_secs,
                    motion.wire_str(),
                );
            }
        }
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
        E::UnknownBehaviorField => "unknown behavior field",
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
            enqueue_remote_command(cmd);
            write_no_content(socket).await
        }
        Err(e) => {
            defmt::warn!("http: bad request body ({})", defmt::Debug2Format(&e));
            let body = format!("invalid request body: {e:?}\n");
            write_text(socket, 400, &body).await
        }
    }
}
