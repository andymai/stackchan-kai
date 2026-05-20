"""Shared audio post-processing for subprocess-driven TTS providers.

WAV-to-PCM transcode (16-bit width check, multi-channel → mono via
per-frame mean, linear resample to 16 kHz) plus a small subprocess
helper that runs a binary writing a WAV file and reads it back.
"""

from __future__ import annotations

import subprocess
import tempfile
import wave
from collections.abc import Callable, Sequence
from pathlib import Path

import numpy as np

from .errors import TTSError
from .protocol import PCM_SAMPLE_RATE_HZ, PCM_SAMPLE_WIDTH_BYTES


def synthesize_via_subprocess(
    *,
    provider: str,
    argv_for_wav: Callable[[Path], Sequence[str]],
    text: str,
    timeout_seconds: float,
) -> bytes:
    """Run a TTS subprocess that writes a WAV file and return its
    16 kHz mono s16 LE PCM bytes.

    ``argv_for_wav`` is called with the temp WAV path and returns the
    full ``argv`` for ``subprocess.run`` (binary + flags). ``text`` is
    piped through stdin so an LLM reply with quotes / backticks can't
    break shell escaping.
    """
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp:
        wav_path = Path(tmp.name)
    try:
        try:
            subprocess.run(
                argv_for_wav(wav_path),
                input=text,
                text=True,
                check=True,
                timeout=timeout_seconds,
                capture_output=True,
            )
        except subprocess.CalledProcessError as e:
            raise TTSError(
                "synthesize",
                f"{provider} exited {e.returncode}: {e.stderr.strip()[:120]}",
            ) from e
        except subprocess.TimeoutExpired as e:
            raise TTSError("synthesize", f"{provider} timed out") from e
        return wav_to_pcm(wav_path, provider=provider)
    finally:
        wav_path.unlink(missing_ok=True)


def wav_to_pcm(wav_path: Path, *, provider: str) -> bytes:
    """Read a WAV written by a TTS subprocess and return 16 kHz mono
    s16 LE bytes. ``provider`` is folded into [`TTSError`] messages
    so the failing engine is identifiable in logs."""
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
    dependency."""
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
