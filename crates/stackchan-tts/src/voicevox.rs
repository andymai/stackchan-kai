//! `VoiceVox` self-hosted TTS — HTTP request shape, WAV parsing,
//! and a buffered [`AudioSource`] that the firmware can fill with
//! synthesis output.
//!
//! `VoiceVox` is a popular self-hostable Japanese TTS engine; the same
//! HTTP API is also implemented by [`AivisSpeech`] and a handful of
//! community engines, so this module's wire format is reusable
//! beyond `VoiceVox` itself. See <https://voicevox.hiroshiba.jp/> for
//! the upstream.
//!
//! ## Two-step protocol
//!
//! `VoiceVox` synthesis is two HTTP round-trips:
//!
//! 1. `POST /audio_query?speaker=<id>&text=<utf8>` → returns a JSON
//!    "audio query" describing prosody, pitch, accent, etc.
//! 2. `POST /synthesis?speaker=<id>` with the audio-query JSON as the
//!    body → returns a WAV file (RIFF + PCM data chunk).
//!
//! The firmware mostly round-trips the audio-query JSON from step 1
//! into step 2, but first rewrites its `outputSamplingRate` field via
//! [`with_output_sampling_rate`] so the engine renders at a rate the
//! I²S path can play without an on-device resampler. This module
//! ships the URL builders for both steps, the rate-override rewrite,
//! a WAV parser that finds the PCM payload inside the synthesis
//! response, and [`wav_to_samples`] to decode that payload into an
//! owned `Vec<i16>`.
//!
//! ## Why the backend stays host-only
//!
//! [`SpeechBackend::render`] is synchronous (`fn render(&self, ...)`),
//! but HTTP I/O on this firmware target is async (`embassy-net`), so
//! `render` cannot itself perform the two synthesis round-trips. The
//! end-to-end path instead runs in a dedicated async firmware task
//! (`stackchan_firmware::voicevox`) that fetches + decodes the WAV
//! (reusing this module's host-tested [`with_output_sampling_rate`] /
//! [`wav_to_samples`] helpers) and enqueues a [`BufferedSource`]
//! directly onto the audio TX queue. Because that task — not the
//! backend — is the synthesis driver, [`VoiceVoxBackend::render`]
//! stays [`crate::RenderError::BackendUnavailable`]: the backend is a
//! config carrier + `can_handle` gate.
//!
//! [`AivisSpeech`]: https://github.com/Aivis-Project/AivisSpeech-Engine
//! [`AudioSource`]: crate::AudioSource
//! [`SpeechBackend::render`]: crate::SpeechBackend::render

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use stackchan_core::voice::{SpeechContent, Utterance};

use crate::backend::{RenderError, SpeechBackend};
use crate::source::AudioSource;

/// `VoiceVox` engine connection settings.
///
/// `host` is the HTTP host (no scheme, no port) — e.g.
/// `"voicevox.local"` or `"192.168.1.20"`. `port` defaults to
/// `50_021` (the upstream's default). `speaker_id` is the per-voice
/// integer the engine ships with — common defaults: `1` (Zundamon
/// "ノーマル"), `2` (Zundamon "あまあま"), `3` (春日部つむぎ
/// "ノーマル"). See the engine's `GET /speakers` endpoint for the
/// full catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceVoxConfig {
    /// HTTP host (no scheme, no port).
    pub host: String,
    /// HTTP port. Default `50_021`.
    pub port: u16,
    /// Engine speaker ID. Default `1` (Zundamon normal).
    pub speaker_id: u16,
}

impl Default for VoiceVoxConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: DEFAULT_VOICEVOX_PORT,
            speaker_id: DEFAULT_VOICEVOX_SPEAKER_ID,
        }
    }
}

/// Default `VoiceVox` engine port — matches the upstream Docker image.
pub const DEFAULT_VOICEVOX_PORT: u16 = 50_021;

/// Default speaker ID — Zundamon "ノーマル", a common default voice.
pub const DEFAULT_VOICEVOX_SPEAKER_ID: u16 = 1;

/// Build the path for step 1 (`/audio_query`).
///
/// Returns `path?speaker=<id>&text=<percent-encoded>`. The path is
/// fixed; only the query string varies per call. Percent-encoding
/// follows RFC 3986 unreserved characters — alphanumerics +
/// `- _ . ~` pass through, everything else gets `%XX`.
///
/// # Panics
///
/// Does not panic. Allocations are bounded by `text.len() * 3` plus
/// a fixed prefix, well under any realistic embedded heap.
#[must_use]
pub fn audio_query_path(text: &str, speaker_id: u16) -> String {
    let encoded = percent_encode(text);
    format!("/audio_query?speaker={speaker_id}&text={encoded}")
}

/// Build the path for step 2 (`/synthesis`).
///
/// The audio-query JSON from step 1 goes in the request body, not the
/// query string — so this path only needs the speaker ID.
#[must_use]
pub fn synthesis_path(speaker_id: u16) -> String {
    format!("/synthesis?speaker={speaker_id}")
}

/// Percent-encode an arbitrary UTF-8 string per RFC 3986
/// `unreserved`. Allocates worst-case `3 * input.len()` bytes.
fn percent_encode(input: &str) -> String {
    // Pre-allocate the worst case so the primary use-case (Japanese
    // TTS text where every UTF-8 byte percent-encodes to 3 chars)
    // doesn't reallocate mid-loop.
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            // Two-digit uppercase hex — fixed allocation per byte.
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0xF) as usize] as char);
        }
    }
    out
}

/// JSON key in the `/audio_query` response that controls the
/// synthesis output sample rate. The engine populates it with its
/// own default (24 000 Hz); rewriting it before the `/synthesis` POST
/// is how the firmware asks for a rate it can play without an
/// on-device resampler.
const OUTPUT_SAMPLING_RATE_KEY: &str = "\"outputSamplingRate\":";

/// Rewrite the `outputSamplingRate` value in an `/audio_query` JSON
/// body so `/synthesis` returns PCM at `target_rate_hz`.
///
/// `VoiceVox` defaults to 24 kHz, but the firmware I²S path (and the
/// [`parse_wav`] rate gate) are wired for a single rate. Asking the
/// engine to render at that rate up front keeps the firmware free of
/// a resampler. The audio-query JSON always carries an
/// `outputSamplingRate` field, so this is a targeted numeric rewrite
/// rather than a full JSON reparse — cheap enough for the embedded
/// path and host-testable in isolation.
///
/// Returns the rewritten JSON. If the key is absent (a
/// non-`VoiceVox` engine, or a future schema change) the input is
/// returned unchanged — the engine then falls back to its own
/// default rate, which [`parse_wav`] rejects loudly downstream rather
/// than silently mis-playing.
#[must_use]
pub fn with_output_sampling_rate(audio_query_json: &str, target_rate_hz: u32) -> String {
    let Some(key_at) = audio_query_json.find(OUTPUT_SAMPLING_RATE_KEY) else {
        return String::from(audio_query_json);
    };
    let value_start = key_at + OUTPUT_SAMPLING_RATE_KEY.len();
    let value_len = audio_query_json[value_start..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    if value_len == 0 {
        return String::from(audio_query_json);
    }
    let mut out = String::with_capacity(audio_query_json.len() + 8);
    out.push_str(&audio_query_json[..value_start]);
    let _ = core::fmt::Write::write_fmt(&mut out, format_args!("{target_rate_hz}"));
    out.push_str(&audio_query_json[value_start + value_len..]);
    out
}

/// Errors the WAV parser can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WavError {
    /// Input shorter than the minimum RIFF header (12 bytes).
    TooShort,
    /// First four bytes weren't `b"RIFF"`.
    NotRiff,
    /// `RIFF` header's container type wasn't `b"WAVE"`.
    NotWave,
    /// `fmt ` sub-chunk missing or malformed.
    BadFormat,
    /// `data` sub-chunk not found before the file ended.
    NoDataChunk,
    /// `fmt ` declared a sample format other than 16-bit PCM mono.
    /// `VoiceVox` always returns 16-bit mono; anything else is a
    /// configuration mismatch upstream.
    UnsupportedFormat,
    /// `fmt ` declared a sample rate this firmware can't play.
    /// Today: must equal the firmware I²S rate. The firmware audio
    /// task asserts on the same constant.
    UnsupportedSampleRate(u32),
}

/// Header describing the PCM payload extracted from a WAV file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavHeader {
    /// Sample rate in Hz. `VoiceVox` defaults to 24 000; firmware
    /// is wired for 16 000.
    pub sample_rate_hz: u32,
    /// Bytes per sample — always `2` for the formats this parser
    /// accepts.
    pub bytes_per_sample: u8,
    /// Channel count — always `1` for the formats this parser
    /// accepts.
    pub channels: u8,
    /// Byte offset (from the start of the input) where the PCM data
    /// begins. The trailing slice is `bytes[data_offset..]`.
    pub data_offset: usize,
    /// Length of the PCM `data` sub-chunk in bytes.
    pub data_len: usize,
}

impl WavHeader {
    /// Number of `i16` samples in the PCM payload.
    #[must_use]
    pub const fn sample_count(&self) -> usize {
        self.data_len / self.bytes_per_sample as usize
    }
}

/// Parse a `WAV` byte stream and locate the PCM payload.
///
/// Validates the RIFF/WAVE container, parses the `fmt ` sub-chunk,
/// walks chunk-by-chunk to find `data`, and returns a [`WavHeader`]
/// describing where the PCM lives. The caller then slices
/// `bytes[header.data_offset..][..header.data_len]` to read samples.
///
/// `expected_sample_rate_hz` is checked against the file's declared
/// rate; mismatches return [`WavError::UnsupportedSampleRate`] so the
/// firmware can either reject the synthesis or kick off a resample.
///
/// # Errors
///
/// See [`WavError`] variants. Files that aren't 16-bit mono PCM are
/// rejected with [`WavError::UnsupportedFormat`].
pub fn parse_wav(bytes: &[u8], expected_sample_rate_hz: u32) -> Result<WavHeader, WavError> {
    if bytes.len() < 12 {
        return Err(WavError::TooShort);
    }
    if &bytes[0..4] != b"RIFF" {
        return Err(WavError::NotRiff);
    }
    if &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotWave);
    }

    let mut cursor = 12usize;
    let mut fmt: Option<FmtChunk> = None;

    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let size = u32::from_le_bytes([
            bytes[cursor + 4],
            bytes[cursor + 5],
            bytes[cursor + 6],
            bytes[cursor + 7],
        ]) as usize;
        let body_start = cursor + 8;
        let body_end = body_start.saturating_add(size);
        if body_end > bytes.len() {
            // A truncated `fmt ` is a malformed format header, not a
            // missing-data-chunk situation — return the more precise
            // error so the caller can distinguish the two.
            return Err(if id == b"fmt " {
                WavError::BadFormat
            } else {
                WavError::NoDataChunk
            });
        }

        match id {
            b"fmt " => {
                fmt = Some(parse_fmt(&bytes[body_start..body_end])?);
            }
            b"data" => {
                let f = fmt.ok_or(WavError::BadFormat)?;
                if f.sample_rate != expected_sample_rate_hz {
                    return Err(WavError::UnsupportedSampleRate(f.sample_rate));
                }
                return Ok(WavHeader {
                    sample_rate_hz: f.sample_rate,
                    bytes_per_sample: f.bytes_per_sample,
                    channels: f.channels,
                    data_offset: body_start,
                    data_len: size,
                });
            }
            _ => {
                // Unknown chunk — skip body, plus padding byte if size is odd.
            }
        }
        // RIFF pads odd-size chunks to an even boundary.
        let padded = if size & 1 == 1 { size + 1 } else { size };
        cursor = body_start.saturating_add(padded);
    }

    Err(WavError::NoDataChunk)
}

/// Decode a synthesis WAV response into an owned PCM sample buffer.
///
/// Composes [`parse_wav`] (RIFF/format validation + rate gate) with a
/// little-endian `i16` decode of the located `data` sub-chunk. The
/// firmware calls this on the buffered HTTP response body, then wraps
/// the result in a [`BufferedSource`] for the audio TX queue.
///
/// A trailing odd byte in the `data` chunk (a malformed WAV whose
/// PCM payload isn't a whole number of samples) is dropped rather
/// than rejected — the parser already validated 16-bit mono, so a
/// dangling byte is engine noise, not a format error worth aborting
/// playback over.
///
/// # Errors
///
/// Propagates every [`WavError`] [`parse_wav`] can return, including
/// [`WavError::UnsupportedSampleRate`] when the engine ignored the
/// [`with_output_sampling_rate`] override.
pub fn wav_to_samples(bytes: &[u8], expected_rate_hz: u32) -> Result<Vec<i16>, WavError> {
    let header = parse_wav(bytes, expected_rate_hz)?;
    let pcm = &bytes[header.data_offset..header.data_offset + header.data_len];
    let mut samples = Vec::with_capacity(header.sample_count());
    for chunk in pcm.as_chunks::<2>().0 {
        samples.push(i16::from_le_bytes(*chunk));
    }
    Ok(samples)
}

/// Parsed `fmt ` sub-chunk — just the fields we care about.
#[derive(Debug, Clone, Copy)]
struct FmtChunk {
    /// Sample rate in Hz.
    sample_rate: u32,
    /// Bytes per sample. Always `2` for the formats accepted here.
    bytes_per_sample: u8,
    /// Channel count. Always `1` for the formats accepted here.
    channels: u8,
}

/// Validate the `fmt ` sub-chunk body and extract the fields we need.
fn parse_fmt(body: &[u8]) -> Result<FmtChunk, WavError> {
    if body.len() < 16 {
        return Err(WavError::BadFormat);
    }
    let format_tag = u16::from_le_bytes([body[0], body[1]]);
    if format_tag != 1 {
        // 1 = PCM. Anything else (IEEE float, ADPCM, extensible) is
        // out of scope — VoiceVox always returns plain PCM.
        return Err(WavError::UnsupportedFormat);
    }
    let channels = u16::from_le_bytes([body[2], body[3]]);
    if channels != 1 {
        return Err(WavError::UnsupportedFormat);
    }
    let sample_rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
    let bits_per_sample = u16::from_le_bytes([body[14], body[15]]);
    if bits_per_sample != 16 {
        return Err(WavError::UnsupportedFormat);
    }
    // `channels` already validated to equal 1 above, so the truncation
    // from u16 is exact. Pin the assumption for any future widening of
    // the channel-count check.
    let channels_u8 = u8::try_from(channels).map_err(|_| WavError::UnsupportedFormat)?;
    Ok(FmtChunk {
        sample_rate,
        bytes_per_sample: 2,
        channels: channels_u8,
    })
}

/// [`AudioSource`] that yields samples from an owned `Vec<i16>`.
///
/// Used by the future firmware HTTP path: the synthesis fetcher
/// allocates a `Vec<i16>` in PSRAM, copies the WAV PCM payload into
/// it (byteswap if needed), wraps it in a [`BufferedSource`], and
/// hands the boxed source back to the speech router.
pub struct BufferedSource {
    /// PCM samples to yield.
    samples: Vec<i16>,
    /// Index of the next unread sample.
    cursor: usize,
}

impl BufferedSource {
    /// Construct from an owned PCM buffer.
    #[must_use]
    pub const fn new(samples: Vec<i16>) -> Self {
        Self { samples, cursor: 0 }
    }
}

impl AudioSource for BufferedSource {
    fn fill(&mut self, buf: &mut [i16]) -> usize {
        let remaining = self.samples.len().saturating_sub(self.cursor);
        let n = buf.len().min(remaining);
        if n == 0 {
            return 0;
        }
        buf[..n].copy_from_slice(&self.samples[self.cursor..self.cursor + n]);
        self.cursor += n;
        n
    }

    fn len_hint(&self) -> Option<usize> {
        Some(self.samples.len().saturating_sub(self.cursor))
    }
}

/// `VoiceVox` HTTP TTS backend.
///
/// Carries the engine config (`host` / `port` / `speaker_id`) and
/// gates routing via [`Self::can_handle`]. [`Self::render`] returns
/// [`RenderError::BackendUnavailable`] by design: the synchronous
/// `render` can't drive the async two-step HTTP round-trip, so the
/// firmware task `stackchan_firmware::voicevox` owns synthesis (see the
/// module docs).
#[derive(Debug, Clone)]
pub struct VoiceVoxBackend {
    /// Engine connection settings.
    pub config: VoiceVoxConfig,
}

impl VoiceVoxBackend {
    /// Construct a backend with the supplied config.
    #[must_use]
    pub const fn new(config: VoiceVoxConfig) -> Self {
        Self { config }
    }
}

impl SpeechBackend for VoiceVoxBackend {
    fn name(&self) -> &'static str {
        "VoiceVox"
    }

    fn can_handle(&self, content: &SpeechContent) -> bool {
        // Dynamic content (handle-based) is the natural fit — the
        // firmware speech router will look up the text payload by
        // ContentRef and pass it to the fetcher.
        matches!(content, SpeechContent::Dynamic(_))
    }

    fn render(&self, _utterance: &Utterance) -> Result<Box<dyn AudioSource>, RenderError> {
        // Synthesis is async HTTP, which this synchronous trait can't
        // drive; the firmware `voicevox` task owns the round-trip and
        // enqueues audio directly. Return a clear-cause error so the
        // speech router falls through to BakedBackend.
        Err(RenderError::BackendUnavailable)
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::missing_docs_in_private_items,
    reason = "test-only: ContentRef::new(1) and parse_wav on a fixture \
              built one line earlier are non-failing by construction"
)]
mod tests {
    use super::*;
    use stackchan_core::voice::{ContentRef, PhraseId};

    #[test]
    fn audio_query_path_includes_speaker_and_text() {
        let path = audio_query_path("hello", 3);
        assert_eq!(path, "/audio_query?speaker=3&text=hello");
    }

    #[test]
    fn audio_query_path_percent_encodes_special_characters() {
        // Spaces and CJK characters must be percent-encoded.
        let path = audio_query_path("hi world", 1);
        assert!(path.contains("hi%20world"));
        // Japanese: あ = U+3042 = E3 81 82 in UTF-8 → %E3%81%82
        let path = audio_query_path("あ", 1);
        assert!(path.contains("%E3%81%82"));
    }

    #[test]
    fn audio_query_path_passes_unreserved_characters() {
        // Per RFC 3986 unreserved: alphanum + `- _ . ~`
        let path = audio_query_path("AZaz09-_.~", 1);
        assert!(path.ends_with("text=AZaz09-_.~"));
    }

    #[test]
    fn synthesis_path_includes_speaker() {
        assert_eq!(synthesis_path(7), "/synthesis?speaker=7");
    }

    /// Build a minimal valid 16-bit mono WAV header for tests. Returns
    /// the full byte vector (header + N silent samples).
    fn build_test_wav(sample_rate: u32, samples: &[i16]) -> Vec<u8> {
        let mut out = Vec::new();
        // RIFF header
        out.extend_from_slice(b"RIFF");
        let data_len = samples.len() * 2;
        let total_size = 36 + data_len;
        #[allow(clippy::cast_possible_truncation)]
        let total_size_u32 = total_size as u32;
        out.extend_from_slice(&total_size_u32.to_le_bytes());
        out.extend_from_slice(b"WAVE");
        // fmt chunk
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&sample_rate.to_le_bytes());
        let byte_rate = sample_rate * 2;
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes()); // block align
        out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        // data chunk
        out.extend_from_slice(b"data");
        #[allow(clippy::cast_possible_truncation)]
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for &s in samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    #[test]
    fn parse_wav_accepts_valid_minimal_file() {
        let samples = [100_i16, -100, 200, -200];
        let bytes = build_test_wav(16_000, &samples);
        let header = parse_wav(&bytes, 16_000).expect("valid wav should parse");
        assert_eq!(header.sample_rate_hz, 16_000);
        assert_eq!(header.channels, 1);
        assert_eq!(header.bytes_per_sample, 2);
        assert_eq!(header.data_len, 8);
        assert_eq!(header.sample_count(), 4);
        // PCM bytes should round-trip back to the original samples.
        let pcm = &bytes[header.data_offset..header.data_offset + header.data_len];
        assert_eq!(pcm, &[100u8, 0, 156, 255, 200, 0, 56, 255][..]);
    }

    #[test]
    fn parse_wav_rejects_too_short() {
        assert_eq!(parse_wav(&[][..], 16_000), Err(WavError::TooShort));
        assert_eq!(parse_wav(&[1u8; 11][..], 16_000), Err(WavError::TooShort));
    }

    #[test]
    fn parse_wav_rejects_non_riff() {
        let mut bytes = build_test_wav(16_000, &[]);
        bytes[0] = b'X';
        assert_eq!(parse_wav(&bytes, 16_000), Err(WavError::NotRiff));
    }

    #[test]
    fn parse_wav_rejects_non_wave() {
        let mut bytes = build_test_wav(16_000, &[]);
        bytes[8] = b'X';
        assert_eq!(parse_wav(&bytes, 16_000), Err(WavError::NotWave));
    }

    #[test]
    fn parse_wav_rejects_sample_rate_mismatch() {
        let bytes = build_test_wav(24_000, &[]);
        // VoiceVox default is 24 kHz; firmware wants 16 kHz.
        assert_eq!(
            parse_wav(&bytes, 16_000),
            Err(WavError::UnsupportedSampleRate(24_000))
        );
    }

    #[test]
    fn wav_to_samples_round_trips_pcm() {
        let samples = [0_i16, 1, -1, 32_767, -32_768, 12_345];
        let bytes = build_test_wav(16_000, &samples);
        let decoded = wav_to_samples(&bytes, 16_000).expect("valid wav should decode");
        assert_eq!(decoded, samples);
    }

    #[test]
    fn wav_to_samples_surfaces_sample_rate_mismatch() {
        let bytes = build_test_wav(24_000, &[1, 2, 3]);
        assert_eq!(
            wav_to_samples(&bytes, 16_000),
            Err(WavError::UnsupportedSampleRate(24_000))
        );
    }

    #[test]
    fn with_output_sampling_rate_rewrites_value() {
        let json = "{\"accent_phrases\":[],\"outputSamplingRate\":24000,\"outputStereo\":false}";
        let out = with_output_sampling_rate(json, 16_000);
        assert_eq!(
            out,
            "{\"accent_phrases\":[],\"outputSamplingRate\":16000,\"outputStereo\":false}"
        );
    }

    #[test]
    fn with_output_sampling_rate_rewrites_when_value_is_last_field() {
        let json = "{\"outputSamplingRate\":24000}";
        assert_eq!(
            with_output_sampling_rate(json, 16_000),
            "{\"outputSamplingRate\":16000}"
        );
    }

    #[test]
    fn with_output_sampling_rate_passes_through_when_key_absent() {
        // A non-VoiceVox engine or a future schema change without the
        // key leaves the body untouched; the rate gate in parse_wav
        // then surfaces any mismatch downstream.
        let json = "{\"accent_phrases\":[]}";
        assert_eq!(with_output_sampling_rate(json, 16_000), json);
    }

    #[test]
    fn buffered_source_yields_samples_then_exhausts() {
        let mut src = BufferedSource::new(alloc::vec![1, 2, 3, 4, 5]);
        let mut buf = [0i16; 3];
        let n = src.fill(&mut buf);
        assert_eq!(n, 3);
        assert_eq!(buf, [1, 2, 3]);
        let n = src.fill(&mut buf);
        assert_eq!(n, 2);
        assert_eq!(&buf[..2], &[4, 5]);
        let n = src.fill(&mut buf);
        assert_eq!(n, 0);
    }

    #[test]
    fn buffered_source_len_hint_decreases_with_consumption() {
        let mut src = BufferedSource::new(alloc::vec![0i16; 10]);
        assert_eq!(src.len_hint(), Some(10));
        let mut buf = [0i16; 4];
        src.fill(&mut buf);
        assert_eq!(src.len_hint(), Some(6));
    }

    #[test]
    fn voicevox_backend_handles_dynamic_content() {
        let backend = VoiceVoxBackend::new(VoiceVoxConfig::default());
        let r = ContentRef::new(1).expect("non-zero");
        assert!(backend.can_handle(&SpeechContent::Dynamic(r)));
    }

    #[test]
    fn voicevox_backend_does_not_handle_baked_phrases() {
        // Baked phrases stay with BakedBackend.
        let backend = VoiceVoxBackend::new(VoiceVoxConfig::default());
        assert!(!backend.can_handle(&SpeechContent::Phrase(PhraseId::Greeting)));
    }

    #[test]
    fn voicevox_backend_name_is_stable() {
        // The name appears in defmt logs and the diagnostics SSE
        // stream; pin it so a future rename surfaces here.
        let backend = VoiceVoxBackend::new(VoiceVoxConfig::default());
        assert_eq!(backend.name(), "VoiceVox");
    }

    #[test]
    fn parse_wav_rejects_truncated_fmt_chunk_as_bad_format() {
        // A `fmt ` chunk whose declared size runs past EOF must be
        // reported as BadFormat (the more precise error), not as
        // NoDataChunk.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&100u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&100u32.to_le_bytes()); // claims 100 bytes
        bytes.extend_from_slice(&[0u8; 4]); // only 4 supplied
        assert_eq!(parse_wav(&bytes, 16_000), Err(WavError::BadFormat));
    }

    #[test]
    fn parse_wav_rejects_truncated_unknown_chunk_as_no_data() {
        // An unknown chunk whose declared size runs past EOF must be
        // reported as NoDataChunk (we never saw a `data` chunk before
        // running out of bytes).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&100u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"junk"); // unknown chunk id
        bytes.extend_from_slice(&100u32.to_le_bytes()); // claims 100 bytes
        bytes.extend_from_slice(&[0u8; 4]); // only 4 supplied
        assert_eq!(parse_wav(&bytes, 16_000), Err(WavError::NoDataChunk));
    }

    #[test]
    fn parse_wav_skips_unknown_chunk_and_reaches_data() {
        // A LIST or junk chunk before the data chunk must be skipped,
        // not abort the parse. Builds a WAV with a 6-byte unknown
        // chunk (odd size → padding byte) before the real data.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // dummy total size
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&32_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        // Unknown chunk with odd size (forces padding-byte branch).
        bytes.extend_from_slice(b"JUNK");
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 4]); // 3 body bytes + 1 padding
        // Real data chunk after.
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 4]);
        let header = parse_wav(&bytes, 16_000).expect("unknown chunk should be skipped");
        assert_eq!(header.sample_rate_hz, 16_000);
    }

    #[test]
    fn parse_wav_rejects_missing_data_chunk() {
        // Valid RIFF/WAVE/fmt header followed by EOF — no data chunk.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&32_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        assert_eq!(parse_wav(&bytes, 16_000), Err(WavError::NoDataChunk));
    }

    #[test]
    fn parse_fmt_rejects_short_body() {
        let mut bytes = build_test_wav(16_000, &[]);
        // Replace the fmt-chunk size field to claim only 8 bytes.
        bytes[16] = 8;
        bytes[17] = 0;
        bytes[18] = 0;
        bytes[19] = 0;
        assert_eq!(parse_wav(&bytes, 16_000), Err(WavError::BadFormat));
    }

    #[test]
    fn parse_fmt_rejects_non_pcm_format_tag() {
        let mut bytes = build_test_wav(16_000, &[]);
        // Replace format-tag (offset 20) with IEEE_FLOAT (0x0003).
        bytes[20] = 3;
        bytes[21] = 0;
        assert_eq!(parse_wav(&bytes, 16_000), Err(WavError::UnsupportedFormat));
    }

    #[test]
    fn parse_fmt_rejects_multi_channel() {
        let mut bytes = build_test_wav(16_000, &[]);
        // Channels (offset 22) → 2 (stereo).
        bytes[22] = 2;
        assert_eq!(parse_wav(&bytes, 16_000), Err(WavError::UnsupportedFormat));
    }

    #[test]
    fn parse_fmt_rejects_non_16_bit_depth() {
        let mut bytes = build_test_wav(16_000, &[]);
        // Bits-per-sample (offset 34) → 24.
        bytes[34] = 24;
        assert_eq!(parse_wav(&bytes, 16_000), Err(WavError::UnsupportedFormat));
    }

    #[test]
    fn voicevox_render_returns_unavailable_by_design() {
        // The synchronous trait can't drive async HTTP; the firmware
        // `voicevox` task is the synthesis driver, so render stays
        // BackendUnavailable on purpose.
        use stackchan_core::voice::{Locale, Priority, SpeechStyle};
        let backend = VoiceVoxBackend::new(VoiceVoxConfig::default());
        let r = ContentRef::new(1).expect("non-zero");
        let utterance = Utterance {
            content: SpeechContent::Dynamic(r),
            locale: Locale::Ja,
            style: SpeechStyle::FromEmotion,
            priority: Priority::Normal,
        };
        assert_eq!(
            backend.render(&utterance).map(|_| ()).err(),
            Some(RenderError::BackendUnavailable)
        );
    }
}
