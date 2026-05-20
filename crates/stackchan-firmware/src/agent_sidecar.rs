//! Push-to-talk capture + sidecar HTTP agent client.
//!
//! Closes the loop on the operator-visible "speak to the avatar, get
//! a reply" path:
//!
//! 1. `POST /listen` (or MCP `start_listen`) lands a
//!    [`RemoteCommand::StartListen`] on
//!    [`crate::net::http::REMOTE_COMMAND_SIGNAL`]; the render-loop
//!    intercept fires [`PTT_TRIGGER`] with the requested
//!    `duration_ms` *and* forwards the variant to the
//!    `RemoteCommandModifier` so the cosmetic listen state still
//!    runs (`Attention::Listening`, Ear decorator, ack chirp).
//! 2. [`agent_sidecar_task`] subscribes to
//!    [`crate::audio::AUDIO_FRAME_PUBSUB`], drains any pre-trigger
//!    backlog so capture starts at the trigger edge, then
//!    accumulates 20 ms frames for the requested window into a
//!    PSRAM-allocated `Vec<i16>`.
//! 3. The captured PCM is posted to the operator-configured
//!    `behavior.agent_sidecar_url` (raw little-endian s16,
//!    `Content-Type: audio/L16;rate=16000;channels=1`). The task
//!    fires [`RemoteCommand::EnterThinking`] on the same signal as
//!    the listen command so the face transitions from Listening
//!    (Ear decorator) to Thinking (thought-bubble) for the duration
//!    of the network round-trip.
//! 4. The sidecar's JSON reply
//!    (`{"text":"...","emotion":"..."}`, OpenAI-Chat-Completions
//!    -shaped projection) surfaces on the firmware toast band, and
//!    any `emotion` tag fires a [`RemoteCommand::SetEmotion`] so
//!    the avatar mirrors the agent's mood. The `SetEmotion` handler in
//!    [`stackchan_core::modifiers::RemoteCommandModifier`] clears
//!    the in-flight thinking hold as a side effect, so the
//!    thought-bubble fades out at the same instant the emotion +
//!    speech-bubble carry the reply.
//! 5. Every failure path (link down, POST failed, timeout) fires
//!    [`RemoteCommand::SetEmotion`] with [`Emotion::Sad`] for a
//!    2.5 s hold so the face visibly registers the failure
//!    instead of just printing a toast. The same `SetEmotion`
//!    side effect clears any in-flight thinking hold on the
//!    paths where one was opened.
//!
//! Empty `agent_sidecar_url` (the default) parks the task — no
//! socket, no pubsub slot, no PTT consumer.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write as _;

use embassy_net::Stack;
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::WaitResult;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use stackchan_core::Emotion;
use stackchan_core::input::RemoteCommand;
use stackchan_net::http_parse::{find_subsequence, parse_content_length};

use crate::audio::{AUDIO_FRAME_PUBSUB, AUDIO_FRAME_SAMPLES};
use crate::net::http::REMOTE_COMMAND_SIGNAL;
use crate::net::wifi::{WIFI_LINK_WATCH, WifiLinkState};
use crate::toast::{ToastLevel, push as toast_push};

/// Push-to-talk capture trigger.
///
/// Signalled from the `REMOTE_COMMAND_SIGNAL` intercept in
/// `main.rs` whenever a [`RemoteCommand::StartListen`] lands.
/// Payload is the requested listen window in ms; the task captures
/// audio for that many ms before posting it to the sidecar.
pub static PTT_TRIGGER: Signal<CriticalSectionRawMutex, u32> = Signal::new();

/// One frame is 20 ms at 16 kHz — see [`AUDIO_FRAME_SAMPLES`].
const FRAME_MS: u64 = 20;

/// Format 16 random bytes as a canonical RFC 4122 v4 UUID string
/// (`xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`, lowercase hex).
///
/// Sets the version (4) nibble in byte 6 and the variant (10xx)
/// nibble in byte 8 before serialising. The caller supplies the
/// entropy — typically `esp_hal::rng::Rng` at boot — so this stays
/// pure and host-testable.
#[must_use]
pub fn format_uuid_v4(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut out = String::with_capacity(36);
    for (i, b) in bytes.iter().enumerate() {
        let _ = write!(&mut out, "{b:02x}");
        if matches!(i, 3 | 5 | 7 | 9) {
            out.push('-');
        }
    }
    out
}

/// TCP RX buffer for the sidecar socket. Headers + JSON response
/// ride here; 2 KiB covers a typical OpenAI-Chat-Completions reply
/// shape plus our `kai_emotion` extension with margin.
const TCP_RX_BYTES: usize = 2048;

/// TCP TX buffer. Smoltcp streams the body as it flushes, so this
/// doesn't need to hold the whole audio payload at once — 4 KiB is
/// enough to keep the socket fed without underflowing while the
/// loop produces 1 KiB write chunks of PCM.
const TCP_TX_BYTES: usize = 4096;

/// Cap on response body size we'll buffer before bailing. The
/// sidecar's reply is short by construction; anything beyond this
/// is malformed or hostile and we'd rather log + drop than blow
/// stack or wait forever.
const RESPONSE_MAX_BYTES: usize = 2048;

/// Cap on the rendered request-header string. The baseline POST
/// line plus `Host` / `Content-Type` / `Content-Length` /
/// `Connection` fits well under 300 B; the headroom covers a long
/// `Authorization: Bearer …` token (operator-supplied secret, up
/// to ~256 chars) plus the fixed 36-char `X-Session-Id` value.
const HEADER_CAPACITY: usize = 768;

/// Cap on the captured PCM duration. Operator-driven windows beyond
/// this are clamped — a 30 s upload at 16 kHz mono is ~960 KiB,
/// already pushing PSRAM-allocator pressure and well past anything
/// a desk-toy operator would ask for.
const CAPTURE_DURATION_CAP_MS: u32 = 30_000;

/// Total POST timeout (connect + write + read). Caps the worst case
/// when the sidecar is sluggish or vanished — we'd rather surface a
/// toast and free the task to handle the next PTT than block here.
const REQUEST_TIMEOUT_MS: u64 = 15_000;

/// One sidecar endpoint parsed from `behavior.agent_sidecar_url`.
///
/// Host is required to be a raw IPv4 literal — same shape as
/// [`crate::audio_debug`]'s UDP target. DNS resolution would
/// require routing the request through `embassy_net::dns` per
/// dispatch; deferring until the operator surface needs hostnames.
struct SidecarEndpoint {
    /// Numeric IPv4 octets for [`embassy_net::Ipv4Address::new`].
    ip: [u8; 4],
    /// TCP port.
    port: u16,
    /// Request path including the leading `/`.
    path: String,
    /// Pre-formatted `host:port` string used as the `Host:` header
    /// value. Built once at parse time to avoid re-formatting per
    /// request.
    host_header: String,
}

/// Parse `"http://a.b.c.d:port[/path]"` into a [`SidecarEndpoint`].
///
/// Returns `None` on any malformed component: HTTPS scheme,
/// missing port, hostname (non-numeric host), bad path. The task
/// logs and parks rather than crashing.
fn parse_sidecar_url(s: &str) -> Option<SidecarEndpoint> {
    let rest = s.strip_prefix("http://")?;
    let (host_port, path) = rest
        .find('/')
        .map_or((rest, "/"), |idx| (&rest[..idx], &rest[idx..]));
    let (host, port_str) = host_port.rsplit_once(':')?;
    let port: u16 = port_str.parse().ok()?;
    let mut octets = [0_u8; 4];
    let mut count = 0;
    for part in host.split('.') {
        if count >= 4 {
            return None;
        }
        octets[count] = part.parse().ok()?;
        count += 1;
    }
    if count != 4 {
        return None;
    }
    Some(SidecarEndpoint {
        ip: octets,
        port,
        path: path.to_string(),
        host_header: host_port.to_string(),
    })
}

/// Reasons one POST round-trip can fail. Surfaced as a toast +
/// defmt log so the operator sees the failure path without an
/// attached monitor.
#[derive(Debug, Clone, Copy)]
enum PostError {
    /// `TcpSocket::connect` returned an error or timed out.
    Connect,
    /// Request header didn't fit in the 512-byte heapless string.
    /// Realistic sidecar URLs won't trigger this; pathological
    /// `agent_sidecar_url` values might.
    HeaderTooLong,
    /// `socket.write_all` returned an error before the full
    /// request body was flushed.
    Write,
    /// `socket.read` returned 0 / Err before the response status
    /// line + headers + body landed.
    Read,
    /// Status line wasn't `HTTP/1.1 2xx`.
    BadStatus,
    /// Response body didn't contain the required `"text"` field
    /// (or it was empty).
    MissingText,
}

impl defmt::Format for PostError {
    fn format(&self, f: defmt::Formatter<'_>) {
        // Each arm emits a distinct label; clippy sees structurally
        // identical macro expansions (only the literal differs)
        // and false-flags them as duplicates. Mirrors the
        // `DispatchError` impl in [`crate::audio`].
        #[allow(
            clippy::match_same_arms,
            reason = "labels are distinct strings even though clippy reads the macro arms as identical"
        )]
        match self {
            Self::Connect => defmt::write!(f, "Connect"),
            Self::HeaderTooLong => defmt::write!(f, "HeaderTooLong"),
            Self::Write => defmt::write!(f, "Write"),
            Self::Read => defmt::write!(f, "Read"),
            Self::BadStatus => defmt::write!(f, "BadStatus"),
            Self::MissingText => defmt::write!(f, "MissingText"),
        }
    }
}

/// Sidecar agent task entry point.
///
/// Idles until Wi-Fi is up, then loops on:
///
/// - Wait for [`PTT_TRIGGER`].
/// - Drain pre-trigger pubsub backlog.
/// - Accumulate frames for `duration_ms` (clamped to
///   `CAPTURE_DURATION_CAP_MS`).
/// - POST the PCM to `sidecar_url`.
/// - Surface the reply via toast; fire `SetEmotion` if tagged.
///
/// Failures (invalid URL, no socket, sidecar unreachable, malformed
/// reply) log a warning and push a toast — the task keeps running
/// so the next PTT still has a consumer.
///
/// `sidecar_url` is the operator-configured
/// `behavior.agent_sidecar_url`. Empty / unparseable → park.
///
/// `bearer_token` is the operator-configured
/// `behavior.agent_sidecar_token`. Empty disables the
/// `Authorization: Bearer …` header (LAN-only mode).
///
/// `session_id` is the per-device identifier sent as `X-Session-Id`
/// so the sidecar can scope conversation memory to this physical
/// unit across requests. Hydrated at boot from `/sd/SESSION.UUID`.
#[embassy_executor::task]
pub async fn agent_sidecar_task(
    stack: Stack<'static>,
    sidecar_url: String,
    bearer_token: String,
    session_id: String,
) -> ! {
    if sidecar_url.is_empty() {
        defmt::info!("agent-sidecar: url empty, idle");
        park_forever().await;
    }
    let Some(endpoint) = parse_sidecar_url(&sidecar_url) else {
        defmt::error!(
            "agent-sidecar: invalid url '{=str}' (expected http://ipv4:port[/path]); idle",
            sidecar_url.as_str(),
        );
        park_forever().await;
    };
    let Ok(mut subscriber) = AUDIO_FRAME_PUBSUB.subscriber() else {
        defmt::error!("agent-sidecar: subscriber slot exhausted; idle");
        park_forever().await;
    };
    let Some(mut link) = WIFI_LINK_WATCH.receiver() else {
        defmt::error!("agent-sidecar: WIFI_LINK_WATCH receiver slot exhausted; idle");
        park_forever().await;
    };

    defmt::info!(
        "agent-sidecar: armed (target {=u8}.{=u8}.{=u8}.{=u8}:{=u16}, path '{=str}')",
        endpoint.ip[0],
        endpoint.ip[1],
        endpoint.ip[2],
        endpoint.ip[3],
        endpoint.port,
        endpoint.path.as_str(),
    );

    // Gate the very first trigger on Wi-Fi being up. Without this,
    // a `POST /listen` fired before initial association consumes
    // the listen window in a capture that then bails on the
    // post-capture link check — operator sees a "link down" toast
    // instead of the listen edge being held until the radio is
    // ready. Subsequent loop iterations keep the post-capture
    // gate so a Wi-Fi drop mid-conversation still surfaces.
    while !matches!(link.get().await, WifiLinkState::Connected) {
        defmt::info!("agent-sidecar: waiting for Wi-Fi link before arming PTT");
        link.changed().await;
    }

    let mut rx_buf = [0_u8; TCP_RX_BYTES];
    let mut tx_buf = [0_u8; TCP_TX_BYTES];

    loop {
        let requested_ms = PTT_TRIGGER.wait().await;
        let duration_ms = clamped_capture_ms(requested_ms);
        let pcm = capture_window(&mut subscriber, duration_ms).await;

        // Bail out if Wi-Fi vanished between trigger and now —
        // posting to a dead link would burn the request timeout
        // and emit a misleading "sidecar unreachable" toast.
        if !matches!(link.get().await, WifiLinkState::Connected) {
            defmt::warn!("agent-sidecar: link not Connected after capture; skipping POST");
            toast_warn("sidecar: link down");
            signal_failure_emotion();
            continue;
        }

        // Swap the face from Listening (Ear) to Thinking
        // (thought-bubble) for the network round-trip. Hold is sized
        // to the request timeout as an upper bound on pathological
        // reply latency. Every exit path clears the hold actively:
        // a successful reply with an emotion tag fires `SetEmotion`
        // (whose modifier side effect clears the hold); success
        // without an emotion fires `ExitThinking`; and the failure /
        // timeout paths fire `signal_failure_emotion()`, whose
        // `SetEmotion(Sad)` clears the hold the same way.
        REMOTE_COMMAND_SIGNAL.signal(RemoteCommand::EnterThinking {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "REQUEST_TIMEOUT_MS is a 5-digit const; truncation is impossible at the source"
            )]
            hold_ms: REQUEST_TIMEOUT_MS as u32,
        });

        let outcome = embassy_time::with_timeout(
            Duration::from_millis(REQUEST_TIMEOUT_MS),
            post_pcm(
                stack,
                &endpoint,
                &pcm,
                &bearer_token,
                &session_id,
                &mut rx_buf,
                &mut tx_buf,
            ),
        )
        .await;
        apply_outcome(outcome);
    }
}

/// Clamp the requested capture window to [`CAPTURE_DURATION_CAP_MS`]
/// and log a warning if the operator's value was reduced.
fn clamped_capture_ms(requested_ms: u32) -> u32 {
    let clamped = requested_ms.min(CAPTURE_DURATION_CAP_MS);
    if clamped != requested_ms {
        defmt::warn!(
            "agent-sidecar: capture window {=u32}ms clamped to cap {=u32}ms",
            requested_ms,
            CAPTURE_DURATION_CAP_MS,
        );
    }
    clamped
}

/// Drain the pubsub backlog and accumulate `duration_ms` worth of
/// audio frames into a fresh `Vec<i16>`. Pre-trigger frames are
/// discarded so capture starts at the trigger edge — without this,
/// up to 160 ms of pre-press chair-scraping would prepend to the
/// upload.
async fn capture_window(
    subscriber: &mut embassy_sync::pubsub::Subscriber<
        '_,
        CriticalSectionRawMutex,
        crate::audio::AudioFrame,
        { crate::audio::AUDIO_FRAME_PUBSUB_DEPTH },
        { crate::audio::AUDIO_FRAME_MAX_SUBSCRIBERS },
        { crate::audio::AUDIO_FRAME_MAX_PUBLISHERS },
    >,
    duration_ms: u32,
) -> Vec<i16> {
    while subscriber.try_next_message().is_some() {}

    let n_frames = usize::try_from(u64::from(duration_ms) / FRAME_MS)
        .unwrap_or(usize::MAX)
        .max(1);
    let target_samples = n_frames * AUDIO_FRAME_SAMPLES;
    let mut pcm: Vec<i16> = Vec::with_capacity(target_samples);
    let mut lagged: u64 = 0;

    defmt::info!(
        "agent-sidecar: capture window {=u32}ms ({=usize} frames)",
        duration_ms,
        n_frames,
    );

    while pcm.len() < target_samples {
        match subscriber.next_message().await {
            WaitResult::Message(frame) => {
                pcm.extend_from_slice(&frame.samples);
            }
            WaitResult::Lagged(n) => {
                lagged = lagged.saturating_add(n);
            }
        }
    }
    if lagged > 0 {
        defmt::warn!(
            "agent-sidecar: capture dropped {=u64} frames mid-window",
            lagged,
        );
    }
    pcm
}

/// Apply the [`post_pcm`] result: surface a toast in every branch,
/// fire [`RemoteCommand::SetEmotion`] with the tagged emotion on a
/// successful reply, and fall back to either [`ExitThinking`] (success
/// with no emotion tag) or a brief Sad face (POST failed, timeout) so
/// the avatar's visible state always matches what just happened.
///
/// [`ExitThinking`]: stackchan_core::input::RemoteCommand::ExitThinking
fn apply_outcome(
    outcome: Result<
        Result<(heapless::String<256>, Option<Emotion>), PostError>,
        embassy_time::TimeoutError,
    >,
) {
    match outcome {
        Ok(Ok((text, emotion))) => {
            defmt::info!(
                "agent-sidecar: reply '{=str}' emotion {:?}",
                text.as_str(),
                defmt::Debug2Format(&emotion),
            );
            toast_info(text.as_str());
            if let Some(e) = emotion {
                REMOTE_COMMAND_SIGNAL.signal(RemoteCommand::SetEmotion {
                    emotion: e,
                    hold_ms: 2_500,
                });
            } else {
                // Reply landed but the sidecar declined to tag an
                // emotion — `SetEmotion`'s thinking-clear side effect
                // would have run otherwise. Fall back to an explicit
                // clear so the thought-bubble fades with the toast
                // instead of lingering for the full request timeout.
                REMOTE_COMMAND_SIGNAL.signal(RemoteCommand::ExitThinking);
            }
        }
        Ok(Err(e)) => {
            defmt::warn!("agent-sidecar: POST failed ({:?})", e);
            toast_warn("sidecar: post failed");
            signal_failure_emotion();
        }
        Err(_) => {
            defmt::warn!(
                "agent-sidecar: POST timed out after {=u64}ms",
                REQUEST_TIMEOUT_MS
            );
            toast_warn("sidecar: timed out");
            signal_failure_emotion();
        }
    }
}

/// Brief face-level reaction to a sidecar failure path. Fires a
/// short-hold [`Emotion::Sad`] so the avatar visibly registers the
/// "I tried and couldn't" beat, complementing the warn-class toast
/// the operator sees in the band beneath the face.
///
/// Doubles as the thinking-hold clear on every code path where a
/// thinking window was opened (post-failed, timeout): the
/// `RemoteCommandModifier` clears `Attention::Thinking` as a side
/// effect of any `SetEmotion`, so the thought-bubble fades on the
/// same tick the Sad face takes over. The link-down path also fires
/// this even though no thinking window opened — the clear side effect
/// is a no-op there, and the Sad face still reads as the failure
/// signal.
fn signal_failure_emotion() {
    REMOTE_COMMAND_SIGNAL.signal(RemoteCommand::SetEmotion {
        emotion: Emotion::Sad,
        hold_ms: 2_500,
    });
}

/// Sample count per body-write chunk. 512 i16 samples → 1 KiB of
/// TCP write per loop iteration; small enough to keep smoltcp's
/// send window primed without holding a giant scratch on the
/// stack, large enough to amortise per-write overhead.
const CHUNK_SAMPLES: usize = 512;

/// Single POST round-trip. Opens a fresh socket per request so a
/// half-closed sidecar doesn't poison subsequent uploads.
async fn post_pcm(
    stack: Stack<'static>,
    endpoint: &SidecarEndpoint,
    pcm: &[i16],
    bearer_token: &str,
    session_id: &str,
    rx_buf: &mut [u8; TCP_RX_BYTES],
    tx_buf: &mut [u8; TCP_TX_BYTES],
) -> Result<(heapless::String<256>, Option<Emotion>), PostError> {
    use embedded_io_async::Write as _;

    let mut socket = TcpSocket::new(stack, rx_buf, tx_buf);
    socket.set_timeout(Some(Duration::from_millis(REQUEST_TIMEOUT_MS / 2)));

    let ip = embassy_net::IpAddress::Ipv4(embassy_net::Ipv4Address::new(
        endpoint.ip[0],
        endpoint.ip[1],
        endpoint.ip[2],
        endpoint.ip[3],
    ));
    let dest = embassy_net::IpEndpoint::new(ip, endpoint.port);
    socket.connect(dest).await.map_err(|e| {
        defmt::warn!(
            "agent-sidecar: connect failed: {:?}",
            defmt::Debug2Format(&e)
        );
        PostError::Connect
    })?;

    let body_len = pcm.len() * 2;
    let mut header: heapless::String<HEADER_CAPACITY> = heapless::String::new();
    write!(
        &mut header,
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         Content-Type: audio/L16;rate=16000;channels=1\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n",
        endpoint.path.as_str(),
        endpoint.host_header.as_str(),
        body_len,
    )
    .map_err(|_| PostError::HeaderTooLong)?;
    if !bearer_token.is_empty() {
        write!(&mut header, "Authorization: Bearer {bearer_token}\r\n")
            .map_err(|_| PostError::HeaderTooLong)?;
    }
    if !session_id.is_empty() {
        write!(&mut header, "X-Session-Id: {session_id}\r\n")
            .map_err(|_| PostError::HeaderTooLong)?;
    }
    header
        .push_str("\r\n")
        .map_err(|()| PostError::HeaderTooLong)?;

    socket
        .write_all(header.as_bytes())
        .await
        .map_err(|_| PostError::Write)?;

    // Stream PCM in 1 KiB chunks. The intermediate byte buffer
    // converts each i16 to LE; doing it in chunks keeps the stack
    // footprint bounded and lets smoltcp send while we're still
    // serialising.
    let mut chunk_bytes = [0_u8; CHUNK_SAMPLES * 2];
    for chunk in pcm.chunks(CHUNK_SAMPLES) {
        for (i, &sample) in chunk.iter().enumerate() {
            let b = sample.to_le_bytes();
            chunk_bytes[i * 2] = b[0];
            chunk_bytes[i * 2 + 1] = b[1];
        }
        socket
            .write_all(&chunk_bytes[..chunk.len() * 2])
            .await
            .map_err(|_| PostError::Write)?;
    }
    socket.flush().await.map_err(|_| PostError::Write)?;

    // Read the response in two phases:
    // 1) Drain bytes until the `\r\n\r\n` header terminator
    //    appears. Bounded by RESPONSE_MAX_BYTES so a hostile or
    //    chatty peer can't make us read forever before we even
    //    have headers.
    // 2) Once the headers are known, parse `Content-Length` and
    //    stop at the body end exactly. Without this, a sidecar
    //    that holds the socket open (e.g., honours keep-alive
    //    despite our `Connection: close` request, or sits behind
    //    a reverse proxy that does) would block our `read()`
    //    until the 15 s task-level timeout — every reply pays
    //    that latency tax.
    let mut resp = [0_u8; RESPONSE_MAX_BYTES];
    let (header_end, after_headers_filled) = read_headers(&mut socket, &mut resp).await?;
    // `parse_content_length` returns `Ok(0)` for both an absent
    // header and an explicit `Content-Length: 0`. The two have
    // different terminate semantics: explicit-0 means "no body,
    // stop now"; absent means "read until peer closes" (which is
    // HTTP/1.0 / HTTP/1.1 with `Connection: close` legacy). Probe
    // the raw header block to disambiguate.
    let header_present = header_has_content_length(&resp[..header_end]);
    let content_length = parse_content_length(&resp[..header_end]).map_err(|_| PostError::Read)?;
    let body_end = read_body(
        &mut socket,
        &mut resp,
        header_end,
        after_headers_filled,
        content_length,
        header_present,
    )
    .await?;
    socket.close();

    let body = parse_http_response(&resp[..body_end])?;
    let text = extract_string(body, "\"text\"").ok_or(PostError::MissingText)?;
    if text.is_empty() {
        return Err(PostError::MissingText);
    }
    let mut out: heapless::String<256> = heapless::String::new();
    for ch in text.chars() {
        if out.push(ch).is_err() {
            break;
        }
    }
    let emotion = extract_string(body, "\"emotion\"").and_then(parse_emotion);
    Ok((out, emotion))
}

/// Drain bytes from `socket` into `buf` until the `\r\n\r\n` header
/// terminator appears. Returns `(header_end, filled)` where
/// `header_end` is the byte index immediately after the terminator
/// (= start of body) and `filled` is the total bytes read so far
/// (may exceed `header_end` if the peer flushed headers + body
/// together). [`PostError::Read`] if the peer closes before the
/// terminator arrives, the read loop errors, or `buf` fills without
/// one.
async fn read_headers(
    socket: &mut TcpSocket<'_>,
    buf: &mut [u8],
) -> Result<(usize, usize), PostError> {
    let mut filled = 0;
    loop {
        if filled >= buf.len() {
            defmt::warn!(
                "agent-sidecar: headers exceeded {=usize} bytes; bailing",
                buf.len()
            );
            return Err(PostError::Read);
        }
        match socket.read(&mut buf[filled..]).await {
            Ok(0) => {
                defmt::warn!("agent-sidecar: peer closed before \\r\\n\\r\\n");
                return Err(PostError::Read);
            }
            Ok(n) => {
                filled += n;
                if let Some(idx) = find_subsequence(&buf[..filled], b"\r\n\r\n") {
                    return Ok((idx + 4, filled));
                }
            }
            Err(e) => {
                defmt::warn!("agent-sidecar: read failed: {:?}", defmt::Debug2Format(&e));
                return Err(PostError::Read);
            }
        }
    }
}

/// Continue reading body bytes into `buf` starting from the
/// `start_filled` high-water mark left by [`read_headers`] until
/// either the `content_length`-sized body lands or the peer closes.
/// Returns the new `filled` count.
///
/// `header_present` distinguishes the two `content_length == 0`
/// shapes: when the header is absent we fall through to the
/// read-until-close fallback (HTTP/1.0 / explicit
/// `Connection: close` legacy); when the header is explicitly
/// `Content-Length: 0` we stop immediately at `header_end`.
/// Without this distinction, an explicit empty-body reply from a
/// peer that holds the socket open would still burn the 15 s
/// task-level timeout.
async fn read_body(
    socket: &mut TcpSocket<'_>,
    buf: &mut [u8],
    header_end: usize,
    start_filled: usize,
    content_length: usize,
    header_present: bool,
) -> Result<usize, PostError> {
    let target = if header_present {
        header_end.saturating_add(content_length).min(buf.len())
    } else {
        buf.len()
    };
    let mut filled = start_filled.max(header_end);
    while filled < target {
        match socket.read(&mut buf[filled..target]).await {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) => {
                defmt::warn!(
                    "agent-sidecar: body read failed: {:?}",
                    defmt::Debug2Format(&e),
                );
                return Err(PostError::Read);
            }
        }
    }
    Ok(filled)
}

/// Case-insensitive search for a `Content-Length:` header in a
/// raw header block. Used to distinguish "header absent" from
/// "header explicitly `Content-Length: 0`" — the two have
/// different termination semantics that
/// [`stackchan_net::http_parse::parse_content_length`] collapses
/// to `Ok(0)`.
fn header_has_content_length(headers: &[u8]) -> bool {
    for line in headers.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            continue;
        };
        let (name, _) = line.split_at(colon);
        if name.eq_ignore_ascii_case(b"content-length") {
            return true;
        }
    }
    false
}

/// Locate the response body. Status must be `2xx`; anything else is
/// reported as [`PostError::BadStatus`] so the operator-side toast
/// surfaces "sidecar: post failed" instead of pretending success.
fn parse_http_response(buf: &[u8]) -> Result<&str, PostError> {
    let text = core::str::from_utf8(buf).map_err(|_| PostError::Read)?;
    let (status_line, rest) = text.split_once("\r\n").ok_or(PostError::Read)?;
    let mut parts = status_line.split_ascii_whitespace();
    // `HTTP/1.1`
    let _ = parts.next().ok_or(PostError::Read)?;
    let code: u16 = parts
        .next()
        .ok_or(PostError::Read)?
        .parse()
        .map_err(|_| PostError::Read)?;
    if !(200..300).contains(&code) {
        defmt::warn!("agent-sidecar: non-2xx status {=u16}", code);
        return Err(PostError::BadStatus);
    }
    // Body starts after the blank line.
    let body = rest.split_once("\r\n\r\n").map_or(rest, |(_, b)| b);
    Ok(body)
}

/// Extract a JSON string value for the given quoted key.
///
/// Flat-object scanner: finds `key` (including its surrounding
/// quotes), expects `:"value"` immediately after (whitespace
/// tolerated), and walks the value character-by-character to
/// honour `\"` and `\\` escapes so a sidecar reply that contains
/// a quoted citation (`"He said \"hi\""`) isn't silently
/// truncated at the inner quote.
///
/// Caveats:
/// - Returns the *raw* slice including any backslash escape
///   characters — no unescaping. Downstream consumers (the toast
///   path) iterate `chars()` and accept the literal characters,
///   which matches the existing behaviour for valid plain ASCII
///   replies. Sidecars that need full-fidelity unescape should
///   simplify before sending.
/// - Does not parse nested objects. The protocol is a flat
///   `{"text":"...","emotion":"..."}` projection of `OpenAI` Chat
///   Completions, intentionally — the sidecar owns the unwrap.
fn extract_string<'a>(json: &'a str, key_quoted: &str) -> Option<&'a str> {
    let i = json.find(key_quoted)?;
    let after_key = &json[i + key_quoted.len()..];
    let after_colon = after_key.trim_start().strip_prefix(':')?.trim_start();
    let after_quote = after_colon.strip_prefix('"')?;
    let bytes = after_quote.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        match bytes[idx] {
            b'\\' if idx + 1 < bytes.len() => idx += 2,
            b'"' => return Some(&after_quote[..idx]),
            _ => idx += 1,
        }
    }
    None
}

/// Map the wire-form emotion string to an [`Emotion`] variant.
/// Returns `None` on any unknown name so the reply still surfaces
/// without the emotion side effect — better than dropping the
/// whole exchange on a typo.
fn parse_emotion(name: &str) -> Option<Emotion> {
    match name {
        "neutral" => Some(Emotion::Neutral),
        "happy" => Some(Emotion::Happy),
        "sleepy" => Some(Emotion::Sleepy),
        "surprised" => Some(Emotion::Surprised),
        "sad" => Some(Emotion::Sad),
        "angry" => Some(Emotion::Angry),
        _ => None,
    }
}

/// Push an info-class toast for a sidecar reply.
///
/// Delegates the truncation to [`crate::toast::push`] — that
/// function iterates `message.chars()` and stops when the
/// 32-char `heapless::String` is full, so multi-byte UTF-8
/// (Japanese, accented Latin, emoji) is cut at a `char` boundary
/// rather than a byte index. Pre-truncating here with
/// `&message[..MAX_TOAST_LEN]` would byte-slice and panic the
/// moment byte 32 landed mid-codepoint — fatal on `no_std` embedded.
fn toast_info(message: &str) {
    let now = stackchan_core::Instant::from_millis(embassy_time::Instant::now().as_millis());
    toast_push(ToastLevel::Info, message, now);
}

/// Push a warn-class toast for a sidecar failure path. Mirrors
/// [`toast_info`] but routes to the warn tier so the band colour
/// matches the operator-facing severity (yellow vs teal).
fn toast_warn(message: &str) {
    let now = stackchan_core::Instant::from_millis(embassy_time::Instant::now().as_millis());
    toast_push(ToastLevel::Warn, message, now);
}

/// Spin forever, parking the task. Used when the task can't
/// proceed (no URL, no subscriber, etc.).
async fn park_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sidecar_url_accepts_ipv4_with_path() {
        let e = parse_sidecar_url("http://192.168.1.42:8080/v1/listen").unwrap();
        assert_eq!(e.ip, [192, 168, 1, 42]);
        assert_eq!(e.port, 8080);
        assert_eq!(e.path, "/v1/listen");
        assert_eq!(e.host_header, "192.168.1.42:8080");
    }

    #[test]
    fn parse_sidecar_url_defaults_path_to_root() {
        let e = parse_sidecar_url("http://10.0.0.1:9000").unwrap();
        assert_eq!(e.path, "/");
    }

    #[test]
    fn parse_sidecar_url_rejects_https() {
        assert!(parse_sidecar_url("https://192.168.1.42:8080/listen").is_none());
    }

    #[test]
    fn parse_sidecar_url_rejects_hostname() {
        assert!(parse_sidecar_url("http://localhost:8080/listen").is_none());
    }

    #[test]
    fn parse_sidecar_url_rejects_missing_port() {
        assert!(parse_sidecar_url("http://192.168.1.42/listen").is_none());
    }

    #[test]
    fn parse_sidecar_url_rejects_empty() {
        assert!(parse_sidecar_url("").is_none());
    }

    #[test]
    fn extract_string_finds_value_with_whitespace() {
        let body = r#"{ "text" : "hello" , "emotion" : "happy" }"#;
        assert_eq!(extract_string(body, "\"text\""), Some("hello"));
        assert_eq!(extract_string(body, "\"emotion\""), Some("happy"));
    }

    #[test]
    fn extract_string_returns_none_when_key_missing() {
        let body = r#"{"text":"hi"}"#;
        assert_eq!(extract_string(body, "\"emotion\""), None);
    }

    #[test]
    fn extract_string_returns_empty_for_empty_value() {
        let body = r#"{"text":""}"#;
        assert_eq!(extract_string(body, "\"text\""), Some(""));
    }

    #[test]
    fn extract_string_walks_past_escaped_quotes() {
        // A sidecar that emits a quoted citation in `text` used
        // to make the scanner truncate at the first `"`, dropping
        // everything after — *and* the `emotion` key that lives
        // later in the body would never be found because the
        // scanner anchored on the stray quote. Pin the fix.
        let body = r#"{"text":"He said \"hi\" softly","emotion":"happy"}"#;
        assert_eq!(
            extract_string(body, "\"text\""),
            Some(r#"He said \"hi\" softly"#)
        );
        assert_eq!(extract_string(body, "\"emotion\""), Some("happy"));
    }

    #[test]
    fn extract_string_handles_escaped_backslash() {
        // `\\` is a single escaped backslash; the *next* `"` is
        // the value terminator, not a continuation of the
        // escape.
        let body = r#"{"text":"path\\to\\file"}"#;
        assert_eq!(extract_string(body, "\"text\""), Some(r#"path\\to\\file"#));
    }

    #[test]
    fn extract_string_returns_none_on_unterminated_value() {
        let body = r#"{"text":"oops"#;
        assert_eq!(extract_string(body, "\"text\""), None);
    }

    #[test]
    fn parse_emotion_accepts_canonical_names() {
        assert_eq!(parse_emotion("neutral"), Some(Emotion::Neutral));
        assert_eq!(parse_emotion("happy"), Some(Emotion::Happy));
        assert_eq!(parse_emotion("sleepy"), Some(Emotion::Sleepy));
        assert_eq!(parse_emotion("surprised"), Some(Emotion::Surprised));
        assert_eq!(parse_emotion("sad"), Some(Emotion::Sad));
        assert_eq!(parse_emotion("angry"), Some(Emotion::Angry));
    }

    #[test]
    fn parse_emotion_rejects_unknown() {
        assert_eq!(parse_emotion("ecstatic"), None);
        assert_eq!(parse_emotion(""), None);
    }

    #[test]
    fn parse_http_response_extracts_body_on_2xx() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"text\":\"hi\"}";
        let body = parse_http_response(raw).unwrap();
        assert_eq!(body, r#"{"text":"hi"}"#);
    }

    #[test]
    fn parse_http_response_rejects_non_2xx() {
        let raw = b"HTTP/1.1 500 Internal Server Error\r\n\r\n{}";
        assert!(matches!(
            parse_http_response(raw),
            Err(PostError::BadStatus)
        ));
    }

    #[test]
    fn header_has_content_length_distinguishes_absent_from_explicit_zero() {
        // Pin the `Content-Length: 0` vs absent disambiguation
        // `read_body` needs to avoid waiting on a peer that
        // legitimately signalled an empty-body response.
        assert!(header_has_content_length(
            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n"
        ));
        assert!(header_has_content_length(
            b"HTTP/1.1 200 OK\r\ncontent-length: 42\r\n\r\n"
        ));
        assert!(header_has_content_length(
            b"HTTP/1.1 200 OK\r\nCONTENT-LENGTH:   42  \r\n\r\n"
        ));
        // Header absent.
        assert!(!header_has_content_length(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n"
        ));
        // Substring match doesn't fool it (`Content-Lengthful` is bogus).
        assert!(!header_has_content_length(
            b"HTTP/1.1 200 OK\r\nContent-Lengthful: 42\r\n\r\n"
        ));
    }

    #[test]
    fn format_uuid_v4_matches_canonical_layout() {
        // Hyphen positions and the version/variant nibbles are
        // load-bearing for downstream UUID parsers.
        let bytes = [
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
            0xde, 0xf0,
        ];
        let uuid = format_uuid_v4(bytes);
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[14..15], "4", "version nibble must be 4");
        let variant = uuid.as_bytes()[19];
        assert!(
            matches!(variant, b'8' | b'9' | b'a' | b'b'),
            "variant nibble must be one of 8/9/a/b, got '{}'",
            variant as char
        );
        for pos in [8, 13, 18, 23] {
            assert_eq!(&uuid[pos..=pos], "-", "expected hyphen at position {pos}");
        }
    }

    #[test]
    fn format_uuid_v4_handles_zero_bytes() {
        // All-zero entropy is the worst case: the formatter still
        // has to inject the version + variant nibbles correctly.
        let uuid = format_uuid_v4([0_u8; 16]);
        assert_eq!(uuid, "00000000-0000-4000-8000-000000000000");
    }
}
