//! `VoiceVox` self-hosted TTS synthesis task.
//!
//! Closes the firmware side of the [`stackchan_tts::voicevox`] wire
//! format: a dedicated async task that turns reply text into spoken
//! audio over two HTTP round-trips to an operator-configured
//! `VoiceVox`-compatible engine.
//!
//! 1. A producer fires [`SYNTH_TEXT_TRIGGER`] with the text to speak —
//!    today the [`crate::agent_sidecar`] task does this on the
//!    `"audio_url": null` reply branch when `behavior.voicevox_url` is
//!    set, so a sidecar reply with no sidecar-synthesised audio still
//!    gets a voice.
//! 2. The task POSTs `/audio_query?speaker=<id>&text=<utf8>`
//!    ([`stackchan_tts::audio_query_path`]), reads the prosody JSON, and
//!    rewrites its `outputSamplingRate` to [`crate::audio::SAMPLE_RATE_HZ`]
//!    via [`stackchan_tts::with_output_sampling_rate`] so the engine
//!    renders at the one rate the I²S path can play without a resampler.
//! 3. The task POSTs the rewritten JSON to `/synthesis?speaker=<id>`
//!    ([`stackchan_tts::synthesis_path`]), drains the WAV response, and
//!    decodes it with [`stackchan_tts::wav_to_samples`] (which also
//!    enforces the 16 kHz rate gate).
//! 4. The decoded PCM is wrapped in a [`stackchan_tts::BufferedSource`]
//!    and enqueued on [`crate::audio::AUDIO_TX_QUEUE`] at
//!    [`Priority::Normal`] — same rank as sidecar / emotion chirps.
//!
//! Empty / unparseable `voicevox_url` (the default) parks the task — no
//! socket, no trigger consumer. The HTTP plumbing mirrors
//! [`crate::agent_sidecar`]'s drain shape; the two stay deliberately
//! separate copies rather than sharing a helper until a refactor PR
//! lifts the common code into `crate::net`.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_net::Stack;
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use stackchan_core::voice::Priority;
use stackchan_net::http_parse::{find_subsequence, parse_content_length};
use stackchan_tts::{BufferedSource, audio_query_path, synthesis_path, wav_to_samples};

use crate::audio::{AUDIO_TX_QUEUE, SAMPLE_RATE_HZ, SpeechSlot};
use crate::net::wifi::{WIFI_LINK_WATCH, WifiLinkState};

/// Text-to-synthesise trigger.
///
/// Fired by a producer (the [`crate::agent_sidecar`] reply path) with
/// the UTF-8 text to speak. Latest-wins: a second trigger that lands
/// while a synthesis is in flight overwrites the pending text — back-to
/// -back triggers are coalesced rather than queued, matching the
/// "newest reply is the one worth voicing" semantics of a desk toy.
pub static SYNTH_TEXT_TRIGGER: Signal<CriticalSectionRawMutex, String> = Signal::new();

/// Set once [`voicevox_task`] arms against a valid engine URL. Lets a
/// producer (the sidecar reply path) cheaply gate whether firing
/// [`SYNTH_TEXT_TRIGGER`] will reach a live consumer, without threading
/// the parsed config through every task.
static VOICEVOX_ARMED: AtomicBool = AtomicBool::new(false);

/// Whether the synthesis task is armed against a configured engine.
///
/// A producer checks this before firing [`SYNTH_TEXT_TRIGGER`] so it
/// can fall back to text-only output when synthesis is disabled.
#[must_use]
pub fn is_armed() -> bool {
    VOICEVOX_ARMED.load(Ordering::Relaxed)
}

/// TCP RX buffer for the synthesis socket. Covers the status line plus
/// a handful of standard headers with margin before the body drain
/// (audio-query JSON or WAV) takes over into a PSRAM `Vec`.
const TCP_RX_BYTES: usize = 2048;

/// TCP TX buffer. Smoltcp streams the request body as it flushes, so
/// this only needs to stay ahead of the rewritten audio-query JSON
/// write — 4 KiB covers a long-prosody query with headroom.
const TCP_TX_BYTES: usize = 4096;

/// Scratch for draining response headers before the body. Sized like
/// the sidecar's: 2 KiB covers the status line + Content-Type /
/// Content-Length / Connection with room to spare.
const HEADER_SCRATCH_BYTES: usize = 2048;

/// Cap on the rendered request-header string. The POST line plus
/// `Host` / `Content-Type` / `Content-Length` / `Connection` is well
/// under 300 B; the headroom covers a long percent-encoded
/// `/audio_query` path (Japanese text expands ~3× per byte).
const HEADER_CAPACITY: usize = 1024;

/// Cap on the audio-query JSON body we'll buffer. The prosody response
/// for a long sentence is a few KiB of accent-phrase structure; 64 KiB
/// is generous headroom before we treat the response as runaway.
const QUERY_BODY_MAX_BYTES: usize = 64 * 1024;

/// Cap on the synthesis WAV body we'll buffer in PSRAM. 1 MiB ≈ 32 s at
/// 16 kHz s16 mono — well past anything the desk toy will say in one
/// reply. Larger bodies get truncated with a warn log so a runaway
/// engine can't OOM the heap.
const WAV_BODY_MAX_BYTES: usize = 1_048_576;

/// Total synthesis timeout (both round-trips, connect + write + read).
/// Sized generously because `VoiceVox` `/audio_query` + `/synthesis`
/// can take several seconds for a long sentence on a CPU-only engine —
/// we'd rather wait than drop a reply.
const SYNTH_TIMEOUT_MS: u64 = 30_000;

/// One `VoiceVox` engine endpoint parsed from `behavior.voicevox_url`.
///
/// Host is a raw IPv4 literal — same shape as
/// [`crate::agent_sidecar`]'s sidecar endpoint. DNS resolution would
/// require routing each request through `embassy_net::dns`; deferring
/// until the operator surface needs hostnames.
struct VoiceVoxEndpoint {
    /// Numeric IPv4 octets for [`embassy_net::Ipv4Address::new`].
    ip: [u8; 4],
    /// TCP port.
    port: u16,
    /// Pre-formatted `host:port` string used as the `Host:` header
    /// value. Built once at parse time to avoid re-formatting per
    /// request.
    host_header: String,
}

/// Parse `"http://a.b.c.d:port"` into a [`VoiceVoxEndpoint`].
///
/// Returns `None` on any malformed component: HTTPS scheme, missing
/// port, hostname (non-numeric host). A trailing path is ignored — the
/// engine paths are built per-request from
/// [`stackchan_tts::audio_query_path`] / [`stackchan_tts::synthesis_path`],
/// so only the host:port is taken from the operator URL.
fn parse_voicevox_url(s: &str) -> Option<VoiceVoxEndpoint> {
    let rest = s.strip_prefix("http://")?;
    let host_port = rest.find('/').map_or(rest, |idx| &rest[..idx]);
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
    Some(VoiceVoxEndpoint {
        ip: octets,
        port,
        host_header: host_port.into(),
    })
}

/// Reasons one synthesis round-trip can fail. Surfaced as a warn-class
/// log — a missing voice is a graceful-degrade (the sidecar reply text
/// already surfaced on the toast band), not a "request failed" event.
///
/// WAV-decode failures collapse into a single [`Self::Wav`] variant; the
/// underlying `WavError` is logged via `Debug2Format` at the decode site
/// since it doesn't derive `defmt::Format`.
#[derive(Debug, Clone, Copy, defmt::Format)]
enum SynthError {
    /// `TcpSocket::connect` returned an error or timed out.
    Connect,
    /// Request header didn't fit in the heapless string.
    HeaderTooLong,
    /// `socket.write_all` returned an error before the request flushed.
    Write,
    /// `socket.read` returned 0 / Err before headers + body landed.
    Read,
    /// Status line wasn't `HTTP/1.1 2xx`.
    BadStatus,
    /// Audio-query JSON body wasn't valid UTF-8.
    BadQuery,
    /// WAV decode failed (bad RIFF, wrong rate, unsupported format).
    Wav,
    /// `AUDIO_TX_QUEUE` was full at enqueue time.
    QueueFull,
}

/// `VoiceVox` synthesis task entry point.
///
/// Idles until Wi-Fi is up, then loops on [`SYNTH_TEXT_TRIGGER`],
/// running the two-step `/audio_query` → `/synthesis` round-trip and
/// enqueuing the decoded PCM on [`crate::audio::AUDIO_TX_QUEUE`].
///
/// `voicevox_url` is the operator-configured `behavior.voicevox_url`.
/// Empty / unparseable → park. `speaker_id` is
/// `behavior.voicevox_speaker_id`.
///
/// Every failure path logs a warning and continues — the task keeps
/// running so the next trigger still has a consumer.
#[embassy_executor::task]
pub async fn voicevox_task(stack: Stack<'static>, voicevox_url: String, speaker_id: u16) -> ! {
    if voicevox_url.is_empty() {
        defmt::info!("voicevox: url empty, idle");
        park_forever().await;
    }
    let Some(endpoint) = parse_voicevox_url(&voicevox_url) else {
        defmt::error!(
            "voicevox: invalid url '{=str}' (expected http://ipv4:port); idle",
            voicevox_url.as_str(),
        );
        park_forever().await;
    };
    let Some(mut link) = WIFI_LINK_WATCH.receiver() else {
        defmt::error!("voicevox: WIFI_LINK_WATCH receiver slot exhausted; idle");
        park_forever().await;
    };

    defmt::info!(
        "voicevox: armed (engine {=u8}.{=u8}.{=u8}.{=u8}:{=u16}, speaker {=u16})",
        endpoint.ip[0],
        endpoint.ip[1],
        endpoint.ip[2],
        endpoint.ip[3],
        endpoint.port,
        speaker_id,
    );
    VOICEVOX_ARMED.store(true, Ordering::Relaxed);

    // Gate the first synthesis on Wi-Fi being up so an early trigger
    // doesn't burn the connect timeout against an unassociated radio.
    while !matches!(link.get().await, WifiLinkState::Connected) {
        defmt::info!("voicevox: waiting for Wi-Fi link before arming synthesis");
        link.changed().await;
    }

    loop {
        let text = SYNTH_TEXT_TRIGGER.wait().await;
        if text.is_empty() {
            continue;
        }

        if !matches!(link.get().await, WifiLinkState::Connected) {
            defmt::warn!("voicevox: link not Connected; skipping synthesis");
            continue;
        }

        let outcome = embassy_time::with_timeout(
            Duration::from_millis(SYNTH_TIMEOUT_MS),
            synthesise(stack, &endpoint, &text, speaker_id),
        )
        .await;
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(e)) => defmt::warn!("voicevox: synthesis failed: {:?}", e),
            Err(_) => defmt::warn!(
                "voicevox: synthesis timed out after {=u64}ms",
                SYNTH_TIMEOUT_MS
            ),
        }
    }
}

/// Run the two-step synthesis round-trip and enqueue the result.
async fn synthesise(
    stack: Stack<'static>,
    endpoint: &VoiceVoxEndpoint,
    text: &str,
    speaker_id: u16,
) -> Result<(), SynthError> {
    let query_path = audio_query_path(text, speaker_id);
    let query_body = post(stack, endpoint, &query_path, None, QUERY_BODY_MAX_BYTES).await?;
    let query_json = core::str::from_utf8(&query_body).map_err(|_| SynthError::BadQuery)?;
    let rewritten = stackchan_tts::with_output_sampling_rate(query_json, SAMPLE_RATE_HZ);

    let synth = synthesis_path(speaker_id);
    let wav = post(
        stack,
        endpoint,
        &synth,
        Some(rewritten.as_bytes()),
        WAV_BODY_MAX_BYTES,
    )
    .await?;
    let samples = wav_to_samples(&wav, SAMPLE_RATE_HZ).map_err(|e| {
        defmt::warn!("voicevox: WAV decode failed: {:?}", defmt::Debug2Format(&e));
        SynthError::Wav
    })?;
    enqueue(samples)
}

/// Single POST round-trip against `endpoint`. `body` is the request
/// payload (the audio-query JSON for `/synthesis`, `None` for
/// `/audio_query` whose input rides the query string). Returns the
/// response body drained into a PSRAM `Vec<u8>`, capped at
/// `body_cap_bytes`.
async fn post(
    stack: Stack<'static>,
    endpoint: &VoiceVoxEndpoint,
    path: &str,
    body: Option<&[u8]>,
    body_cap_bytes: usize,
) -> Result<Vec<u8>, SynthError> {
    use embedded_io_async::Write as _;

    let mut rx_buf = [0_u8; TCP_RX_BYTES];
    let mut tx_buf = [0_u8; TCP_TX_BYTES];
    let mut socket = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);
    socket.set_timeout(Some(Duration::from_millis(SYNTH_TIMEOUT_MS / 2)));

    let ip = embassy_net::IpAddress::Ipv4(embassy_net::Ipv4Address::new(
        endpoint.ip[0],
        endpoint.ip[1],
        endpoint.ip[2],
        endpoint.ip[3],
    ));
    let dest = embassy_net::IpEndpoint::new(ip, endpoint.port);
    socket.connect(dest).await.map_err(|e| {
        defmt::warn!("voicevox: connect failed: {:?}", defmt::Debug2Format(&e));
        SynthError::Connect
    })?;

    let body_len = body.map_or(0, <[u8]>::len);
    let mut header: heapless::String<HEADER_CAPACITY> = heapless::String::new();
    write!(
        &mut header,
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        path,
        endpoint.host_header.as_str(),
        body_len,
    )
    .map_err(|_| SynthError::HeaderTooLong)?;

    socket
        .write_all(header.as_bytes())
        .await
        .map_err(|_| SynthError::Write)?;
    if let Some(body) = body {
        socket
            .write_all(body)
            .await
            .map_err(|_| SynthError::Write)?;
    }
    socket.flush().await.map_err(|_| SynthError::Write)?;

    let mut hdr_scratch = [0_u8; HEADER_SCRATCH_BYTES];
    let (header_end, filled) = read_headers(&mut socket, &mut hdr_scratch).await?;
    if !status_is_2xx(&hdr_scratch[..header_end]) {
        return Err(SynthError::BadStatus);
    }
    let out = drain_body(
        &mut socket,
        &hdr_scratch[..filled],
        header_end,
        body_cap_bytes,
    )
    .await?;
    socket.close();
    Ok(out)
}

/// Drain bytes into `buf` until the `\r\n\r\n` header terminator.
/// Returns `(header_end, filled)`: `header_end` is the byte index after
/// the terminator (= body start), `filled` the total bytes read (may
/// exceed `header_end` when the peer flushed headers + body together).
async fn read_headers(
    socket: &mut TcpSocket<'_>,
    buf: &mut [u8],
) -> Result<(usize, usize), SynthError> {
    let mut filled = 0;
    loop {
        if filled >= buf.len() {
            defmt::warn!("voicevox: headers exceeded {=usize} bytes", buf.len());
            return Err(SynthError::Read);
        }
        match socket.read(&mut buf[filled..]).await {
            Ok(0) => {
                defmt::warn!("voicevox: peer closed before \\r\\n\\r\\n");
                return Err(SynthError::Read);
            }
            Ok(n) => {
                filled += n;
                if let Some(idx) = find_subsequence(&buf[..filled], b"\r\n\r\n") {
                    return Ok((idx + 4, filled));
                }
            }
            Err(e) => {
                defmt::warn!(
                    "voicevox: header read failed: {:?}",
                    defmt::Debug2Format(&e)
                );
                return Err(SynthError::Read);
            }
        }
    }
}

/// Drain the response body into a PSRAM `Vec<u8>`. Bytes already in
/// `hdr_scratch[header_end..]` (peer flushed the body alongside
/// headers) are copied first so none are lost. Honours an explicit
/// `Content-Length` (capped at `cap`) and falls back to read-until
/// -close when the header is absent.
async fn drain_body(
    socket: &mut TcpSocket<'_>,
    hdr_scratch: &[u8],
    header_end: usize,
    cap: usize,
) -> Result<Vec<u8>, SynthError> {
    let content_length =
        parse_content_length(&hdr_scratch[..header_end]).map_err(|_| SynthError::Read)?;
    let header_present = header_has_content_length(&hdr_scratch[..header_end]);
    if header_present && content_length > cap {
        defmt::warn!(
            "voicevox: body {=usize}B exceeds {=usize}B cap; truncating",
            content_length,
            cap,
        );
    }
    let target_len = if header_present {
        content_length.min(cap)
    } else {
        cap.min(64 * 1024)
    };
    let mut body: Vec<u8> = Vec::with_capacity(target_len);
    let leftover_end = hdr_scratch.len().min(header_end + target_len);
    body.extend_from_slice(&hdr_scratch[header_end..leftover_end]);
    let mut chunk = [0_u8; 4096];
    while body.len() < target_len {
        let want = (target_len - body.len()).min(chunk.len());
        match socket.read(&mut chunk[..want]).await {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(e) => {
                defmt::warn!("voicevox: body read failed: {:?}", defmt::Debug2Format(&e));
                return Err(SynthError::Read);
            }
        }
    }
    Ok(body)
}

/// Case-insensitive probe for a `Content-Length:` header. Distinguishes
/// "header absent" (read until close) from "explicit `Content-Length`"
/// (stop at the declared length) — [`parse_content_length`] collapses
/// both to `Ok(0)` for an empty body.
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

/// Whether the response status line is `HTTP/1.x 2xx`.
fn status_is_2xx(headers: &[u8]) -> bool {
    let Ok(text) = core::str::from_utf8(headers) else {
        return false;
    };
    let Some((status_line, _)) = text.split_once("\r\n") else {
        return false;
    };
    let mut parts = status_line.split_ascii_whitespace();
    let _ = parts.next();
    let Some(code_str) = parts.next() else {
        return false;
    };
    let Ok(code) = code_str.parse::<u16>() else {
        return false;
    };
    if !(200..300).contains(&code) {
        defmt::warn!("voicevox: non-2xx status {=u16}", code);
        return false;
    }
    true
}

/// Enqueue decoded PCM for TX playback at [`Priority::Normal`] — the
/// same rank sidecar / emotion chirps use.
fn enqueue(samples: Vec<i16>) -> Result<(), SynthError> {
    defmt::info!("voicevox: synthesised {=usize} samples", samples.len());
    let slot = SpeechSlot {
        source: alloc::boxed::Box::new(BufferedSource::new(samples)),
        priority: Priority::Normal,
    };
    AUDIO_TX_QUEUE.try_send(slot).map_err(|_| {
        defmt::warn!("voicevox: AUDIO_TX_QUEUE full; dropping synthesis");
        SynthError::QueueFull
    })
}

/// Spin forever, parking the task. Used when it can't proceed (no URL,
/// no link receiver).
async fn park_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_voicevox_url_accepts_ipv4_with_port() {
        let e = parse_voicevox_url("http://192.168.1.50:50021").unwrap();
        assert_eq!(e.ip, [192, 168, 1, 50]);
        assert_eq!(e.port, 50_021);
        assert_eq!(e.host_header, "192.168.1.50:50021");
    }

    #[test]
    fn parse_voicevox_url_ignores_trailing_path() {
        // The engine paths are built per-request; only host:port is
        // taken from the operator URL.
        let e = parse_voicevox_url("http://10.0.0.2:50021/ignored").unwrap();
        assert_eq!(e.ip, [10, 0, 0, 2]);
        assert_eq!(e.port, 50_021);
        assert_eq!(e.host_header, "10.0.0.2:50021");
    }

    #[test]
    fn parse_voicevox_url_rejects_https() {
        assert!(parse_voicevox_url("https://192.168.1.50:50021").is_none());
    }

    #[test]
    fn parse_voicevox_url_rejects_hostname() {
        assert!(parse_voicevox_url("http://voicevox.local:50021").is_none());
    }

    #[test]
    fn parse_voicevox_url_rejects_missing_port() {
        assert!(parse_voicevox_url("http://192.168.1.50").is_none());
    }

    #[test]
    fn parse_voicevox_url_rejects_empty() {
        assert!(parse_voicevox_url("").is_none());
    }

    #[test]
    fn status_is_2xx_accepts_200_and_204() {
        assert!(status_is_2xx(
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"
        ));
        assert!(status_is_2xx(b"HTTP/1.1 204 No Content\r\n\r\n"));
    }

    #[test]
    fn status_is_2xx_rejects_non_2xx_and_garbage() {
        assert!(!status_is_2xx(
            b"HTTP/1.1 500 Internal Server Error\r\n\r\n"
        ));
        assert!(!status_is_2xx(b"not http\r\n\r\n"));
        assert!(!status_is_2xx(b"HTTP/1.1\r\n\r\n"));
    }

    #[test]
    fn header_has_content_length_distinguishes_absent_from_present() {
        assert!(header_has_content_length(
            b"HTTP/1.1 200 OK\r\nContent-Length: 42\r\n\r\n"
        ));
        assert!(header_has_content_length(
            b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\n\r\n"
        ));
        assert!(!header_has_content_length(
            b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\n\r\n"
        ));
        // Substring match doesn't fool it.
        assert!(!header_has_content_length(
            b"HTTP/1.1 200 OK\r\nContent-Lengthful: 42\r\n\r\n"
        ));
    }
}
