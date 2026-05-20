"""Unit tests for the shared TTS audio helpers (resample + downmix
+ WAV transcode).

These exercise `tts/_audio.py` directly so the maths is pinned
independently of any particular provider. The provider tests
(`test_tts_espeak.py`, `test_tts_piper.py`) still cover the
end-to-end synthesis path.
"""

from __future__ import annotations

import struct
import wave
from pathlib import Path

import numpy as np
import pytest

from stackchan_sidecar.tts._audio import resample_and_downmix, wav_to_pcm
from stackchan_sidecar.tts.errors import TTSError
from stackchan_sidecar.tts.protocol import PCM_SAMPLE_RATE_HZ


def _write_wav(path: Path, samples: np.ndarray, *, rate: int, channels: int) -> None:
    with wave.open(str(path), "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(samples.astype(np.int16).tobytes())


def test_resample_handles_22050_to_16000() -> None:
    # Pin the maths so a future numpy version that changes
    # np.interp's edge handling doesn't silently shift samples.
    samples_22k = np.arange(22050, dtype=np.int16)
    out = resample_and_downmix(samples_22k.tobytes(), src_rate=22050, channels=1)
    out_samples = np.frombuffer(out, dtype=np.int16)
    # 22050 → 16000 is a ~72.6% ratio; expect 16000 samples.
    assert abs(len(out_samples) - 16000) <= 1
    # Monotonic ramp survives (within rounding) — first sample near
    # 0, last sample near the peak input (~21999).
    assert out_samples[0] >= 0
    assert out_samples[-1] > 20000


def test_resample_downmixes_stereo_to_mono() -> None:
    left = np.full(1000, 100, dtype=np.int16)
    right = np.full(1000, 200, dtype=np.int16)
    interleaved = np.stack([left, right], axis=1).reshape(-1).astype(np.int16)
    out = resample_and_downmix(interleaved.tobytes(), src_rate=PCM_SAMPLE_RATE_HZ, channels=2)
    out_samples = np.frombuffer(out, dtype=np.int16)
    assert len(out_samples) == 1000  # no resample, only downmix
    assert int(out_samples[0]) == 150  # mean(100, 200)


def test_resample_passthrough_when_rate_matches() -> None:
    pcm_in = (b"\x10\x00") * 100  # 100 samples of value 16
    out = resample_and_downmix(pcm_in, src_rate=PCM_SAMPLE_RATE_HZ, channels=1)
    assert out == pcm_in  # bit-exact passthrough


def test_resample_returns_empty_for_pathologically_short_input() -> None:
    # One sample at 22050 Hz → round(1 * 16000 / 22050) = 1, so the
    # path triggers only when there's literally zero samples or the
    # ratio rounds to 0. Two samples at a very high rate gives < 1
    # destination sample.
    pcm_in = (b"\x00\x00") * 2
    out = resample_and_downmix(pcm_in, src_rate=10_000_000, channels=1)
    assert out == b""


def test_wav_to_pcm_round_trips_22050_mono(tmp_path: Path) -> None:
    samples = (np.sin(np.linspace(0, 2 * np.pi * 440, 22050)) * 10000).astype(np.int16)
    wav = tmp_path / "tone.wav"
    _write_wav(wav, samples, rate=22050, channels=1)
    pcm = wav_to_pcm(wav, provider="test")
    # 22050 → 16000 resample should be very close to 16000 samples.
    assert abs(len(pcm) // 2 - 16000) <= 1


def test_wav_to_pcm_rejects_empty_payload(tmp_path: Path) -> None:
    wav = tmp_path / "empty.wav"
    _write_wav(wav, np.zeros(0, dtype=np.int16), rate=22050, channels=1)
    with pytest.raises(TTSError) as exc_info:
        wav_to_pcm(wav, provider="test")
    assert exc_info.value.stage == "synthesize"


def test_wav_to_pcm_rejects_wrong_sample_width(tmp_path: Path) -> None:
    wav = tmp_path / "wide.wav"
    with wave.open(str(wav), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(4)  # 32-bit — wrong
        w.setframerate(22050)
        # Four bytes per sample.
        w.writeframes(struct.pack("<i", 1234) * 100)
    with pytest.raises(TTSError) as exc_info:
        wav_to_pcm(wav, provider="test")
    assert exc_info.value.stage == "transcode"
    assert "width 4" in exc_info.value.detail


def test_wav_to_pcm_includes_provider_in_error(tmp_path: Path) -> None:
    # Missing file: provider name must appear in the error so the
    # log line points at the right offender when two providers are
    # configured.
    missing = tmp_path / "nope.wav"
    with pytest.raises(TTSError) as exc_info:
        wav_to_pcm(missing, provider="piper")
    assert "piper" in exc_info.value.detail
