"""Shared audio post-processing for subprocess-driven TTS providers.

Both [`EspeakProvider`] and [`PiperProvider`] shell out to a binary
that writes a WAV file at the model's native sample rate (22050 Hz for
both, in practice), then read that WAV back as raw bytes and convert
it to the firmware's wire format: **16 kHz mono s16 little-endian PCM**.

The two paths share three concerns:

1. WAV parse + sanity check (16-bit sample width, non-empty payload).
2. Multi-channel → mono via per-frame mean (both providers are mono
   in practice; the guard is cheap and future-proofs us against a
   provider that defaults to stereo).
3. Sample-rate resample to 16 kHz via linear interpolation on
   NumPy arrays.

Linear interpolation is "good enough" for a robot-voice desk toy.
A more demanding application would reach for polyphase resampling
(`scipy.signal.resample_poly`, `libsoxr`); for this product, the
quality gap is invisible next to the model itself.
"""

from __future__ import annotations

import wave
from pathlib import Path

import numpy as np

from .errors import TTSError
from .protocol import PCM_SAMPLE_RATE_HZ, PCM_SAMPLE_WIDTH_BYTES


def wav_to_pcm(wav_path: Path, *, provider: str) -> bytes:
    """Read a WAV file written by a TTS subprocess and return raw
    16 kHz mono s16 LE bytes.

    ``provider`` is folded into [`TTSError`] messages so a failure
    in the espeak path doesn't blame piper (and vice versa).

    Raises [`TTSError`] with stage ``"synthesize"`` for empty output
    and ``"transcode"`` for everything else (unreadable WAV, wrong
    sample width, pathologically short payload after resample).
    """
    try:
        with wave.open(str(wav_path), "rb") as r:
            channels = r.getnchannels()
            width = r.getsampwidth()
            src_rate = r.getframerate()
            frames = r.readframes(r.getnframes())
    except (wave.Error, FileNotFoundError) as e:
        raise TTSError("transcode", f"couldn't read {provider} WAV: {e}") from e
    if not frames:
        raise TTSError("synthesize", f"{provider} produced empty audio")
    if width != PCM_SAMPLE_WIDTH_BYTES:
        raise TTSError(
            "transcode",
            f"{provider} WAV width {width} != expected {PCM_SAMPLE_WIDTH_BYTES}",
        )
    pcm = resample_and_downmix(frames, src_rate, channels)
    if not pcm:
        # Pathological input (one source sample at a rate that rounds
        # down to < 1 destination sample). Catch it here so the audio
        # cache never holds a zero-byte entry; the firmware would
        # otherwise receive a 200 + empty body and have to invent its
        # own "is this really audio?" check.
        raise TTSError("transcode", f"{provider} resampled audio is empty")
    return pcm


def resample_and_downmix(frames: bytes, src_rate: int, channels: int) -> bytes:
    """Convert raw s16 LE frames at ``src_rate`` x ``channels`` to
    16 kHz mono s16 LE bytes. Pure NumPy; no audioop / scipy / libsoxr
    dependency.
    """
    samples = np.frombuffer(frames, dtype=np.int16)
    if channels > 1:
        samples = samples.reshape(-1, channels).mean(axis=1).astype(np.int16)
    if src_rate != PCM_SAMPLE_RATE_HZ:
        src_len = samples.size
        dst_len = round(src_len * PCM_SAMPLE_RATE_HZ / src_rate)
        if dst_len <= 0:
            return b""
        src_positions = np.linspace(0.0, 1.0, src_len, dtype=np.float64)
        dst_positions = np.linspace(0.0, 1.0, dst_len, dtype=np.float64)
        resampled = np.interp(dst_positions, src_positions, samples.astype(np.float64))
        samples = np.clip(resampled, -32768, 32767).astype(np.int16)
    return samples.tobytes()
