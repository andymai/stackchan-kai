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
//!    `Content-Type: audio/L16;rate=16000;channels=1`).
//! 4. The sidecar's JSON reply
//!    (`{"text":"...","emotion":"..."}`, OpenAI-Chat-Completions
//!    -shaped projection) surfaces on the firmware toast band, and
//!    any `emotion` tag fires a [`RemoteCommand::SetEmotion`] so
//!    the avatar mirrors the agent's mood.
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

use crate::audio::{AUDIO_FRAME_PUBSUB, AUDIO_FRAME_SAMPLES};
use crate::net::http::REMOTE_COMMAND_SIGNAL;
use crate::net::wifi::{WIFI_LINK_WATCH, WifiLinkState};
use crate::toast::{MAX_TOAST_LEN, ToastLevel, push as toast_push};

/// Push-to-talk capture trigger.
///
/// Signalled from the `REMOTE_COMMAND_SIGNAL` intercept in
/// `main.rs` whenever a [`RemoteCommand::StartListen`] lands.
/// Payload is the requested listen window in ms; the task captures
/// audio for that many ms before posting it to the sidecar.
pub static PTT_TRIGGER: Signal<CriticalSectionRawMutex, u32> = Signal::new();

/// One frame is 20 ms at 16 kHz — see [`AUDIO_FRAME_SAMPLES`].
const FRAME_MS: u64 = 20;

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
    /// Request header didn't fit in the 256-byte heapless string.
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
///   [`CAPTURE_DURATION_CAP_MS`]).
/// - POST the PCM to `sidecar_url`.
/// - Surface the reply via toast; fire `SetEmotion` if tagged.
///
/// Failures (invalid URL, no socket, sidecar unreachable, malformed
/// reply) log a warning and push a toast — the task keeps running
/// so the next PTT still has a consumer.
///
/// `sidecar_url` is the operator-configured
/// `behavior.agent_sidecar_url`. Empty / unparseable → park.
#[embassy_executor::task]
pub async fn agent_sidecar_task(stack: Stack<'static>, sidecar_url: String) -> ! {
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
            continue;
        }

        let outcome = embassy_time::with_timeout(
            Duration::from_millis(REQUEST_TIMEOUT_MS),
            post_pcm(stack, &endpoint, &pcm, &mut rx_buf, &mut tx_buf),
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

/// Apply the [`post_pcm`] result: surface a toast in every branch
/// and fire [`RemoteCommand::SetEmotion`] on success when the reply
/// carries an emotion tag.
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
            }
        }
        Ok(Err(e)) => {
            defmt::warn!("agent-sidecar: POST failed ({:?})", e);
            toast_warn("sidecar: post failed");
        }
        Err(_) => {
            defmt::warn!(
                "agent-sidecar: POST timed out after {=u64}ms",
                REQUEST_TIMEOUT_MS
            );
            toast_warn("sidecar: timed out");
        }
    }
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
    let mut header: heapless::String<512> = heapless::String::new();
    write!(
        &mut header,
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         Content-Type: audio/L16;rate=16000;channels=1\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        endpoint.path.as_str(),
        endpoint.host_header.as_str(),
        body_len,
    )
    .map_err(|_| PostError::HeaderTooLong)?;

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

    // Read until the peer closes (we sent `Connection: close`).
    // Bounded by RESPONSE_MAX_BYTES so a hostile peer can't make
    // us read forever.
    let mut resp = [0_u8; RESPONSE_MAX_BYTES];
    let mut filled = 0;
    loop {
        if filled >= resp.len() {
            defmt::warn!(
                "agent-sidecar: response exceeded {=usize} bytes; truncating",
                resp.len()
            );
            break;
        }
        match socket.read(&mut resp[filled..]).await {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) => {
                defmt::warn!("agent-sidecar: read failed: {:?}", defmt::Debug2Format(&e));
                return Err(PostError::Read);
            }
        }
    }
    socket.close();

    let body = parse_http_response(&resp[..filled])?;
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
/// Naïve flat-object scanner: finds `key` (including quotes),
/// expects `:"value"` immediately after (whitespace tolerated),
/// returns the substring between the value-side quotes.
///
/// Caveats:
/// - Does not handle backslash-escaped quotes inside the value.
///   The sidecar is operator-controlled; a well-behaved sidecar
///   that needs to emit literal quotes can pre-substitute or
///   wrap the payload differently.
/// - Does not parse nested objects. The protocol is a flat
///   `{"text":"...","emotion":"..."}` projection of `OpenAI` Chat
///   Completions, intentionally — the sidecar owns the unwrap.
fn extract_string<'a>(json: &'a str, key_quoted: &str) -> Option<&'a str> {
    let i = json.find(key_quoted)?;
    let after_key = &json[i + key_quoted.len()..];
    let after_colon = after_key.trim_start().strip_prefix(':')?.trim_start();
    let after_quote = after_colon.strip_prefix('"')?;
    let end = after_quote.find('"')?;
    Some(&after_quote[..end])
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

/// Push an info-class toast for a sidecar reply. The toast band
/// truncates at [`MAX_TOAST_LEN`] bytes; we don't try to be clever
/// about elision because the operator can always read the full
/// reply over defmt.
fn toast_info(message: &str) {
    let now = stackchan_core::Instant::from_millis(embassy_time::Instant::now().as_millis());
    let truncated = if message.len() > MAX_TOAST_LEN {
        &message[..MAX_TOAST_LEN]
    } else {
        message
    };
    toast_push(ToastLevel::Warn, truncated, now);
}

/// Push a warn-class toast for a sidecar error. Mirrors
/// [`toast_info`] but is reserved for failure paths so a future
/// toast `Info` tier can split out the success surface cleanly.
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
}
