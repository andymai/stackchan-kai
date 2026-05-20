"""Unit tests for the eSpeak-NG TTS provider.

These tests require `espeak-ng` on PATH. If it's not installed they
skip with a clear reason rather than fail, so a contributor without
the binary can still run the rest of the suite.
"""

from __future__ import annotations

import shutil

import pytest

from stackchan_sidecar.tts import EspeakProvider, TTSError
from stackchan_sidecar.tts.protocol import PCM_SAMPLE_RATE_HZ

_ESPEAK_AVAILABLE = shutil.which("espeak-ng") is not None
_SKIP_REASON = "espeak-ng binary not installed; provider tests skip on this host"


@pytest.fixture
def provider() -> EspeakProvider:
    return EspeakProvider()


@pytest.mark.asyncio
async def test_setup_error_when_binary_missing(monkeypatch: pytest.MonkeyPatch) -> None:
    # Force the `which` lookup to fail so we exercise the setup-stage
    # error path without requiring the binary to be absent on the
    # host. This is the most important test for environments where
    # espeak-ng IS installed but might not be in CI's PATH.
    monkeypatch.setattr("stackchan_sidecar.tts.espeak.shutil.which", lambda _: None)
    with pytest.raises(TTSError) as exc_info:
        await EspeakProvider().synthesize("hello")
    assert exc_info.value.stage == "setup"


@pytest.mark.asyncio
async def test_rejects_empty_text(provider: EspeakProvider) -> None:
    if not _ESPEAK_AVAILABLE:
        pytest.skip(_SKIP_REASON)
    with pytest.raises(TTSError, match="empty text"):
        await provider.synthesize("")
    with pytest.raises(TTSError, match="empty text"):
        await provider.synthesize("   \n\t")


@pytest.mark.asyncio
async def test_synthesize_returns_16khz_mono_pcm(provider: EspeakProvider) -> None:
    if not _ESPEAK_AVAILABLE:
        pytest.skip(_SKIP_REASON)
    result = await provider.synthesize("Hello stack-chan.")
    assert result.provider == "espeak_ng"
    # Even/whole-sample alignment — bytes are s16, so length must be even.
    assert len(result.pcm) % 2 == 0
    # Sane lower bound: any utterance should produce at least ~50 ms.
    assert result.duration_seconds > 0.05
    # Upper bound — a short sentence shouldn't blow past 5 s.
    assert result.duration_seconds < 5.0


@pytest.mark.asyncio
async def test_duration_property_matches_byte_count(provider: EspeakProvider) -> None:
    if not _ESPEAK_AVAILABLE:
        pytest.skip(_SKIP_REASON)
    result = await provider.synthesize("Hi.")
    expected = (len(result.pcm) // 2) / PCM_SAMPLE_RATE_HZ
    assert abs(result.duration_seconds - expected) < 1e-9


def test_resample_handles_22050_to_16000() -> None:
    # The internal resampler is exercised every synthesis on a default
    # espeak-ng build (it emits 22050 Hz). Pin the maths so a future
    # numpy version that changes np.interp's edge handling doesn't
    # silently shift samples around.
    import numpy as np

    from stackchan_sidecar.tts.espeak import EspeakProvider

    # 22050 Hz mono ramp: values 0..21999 (one second).
    samples_22k = np.arange(22050, dtype=np.int16)
    pcm_in = samples_22k.tobytes()
    out = EspeakProvider._resample_and_downmix(pcm_in, src_rate=22050, channels=1)
    # 22050 → 16000 is a ~72.6% ratio; expect 16000 samples.
    out_samples = np.frombuffer(out, dtype=np.int16)
    assert abs(len(out_samples) - 16000) <= 1
    # Monotonic ramp survives (within rounding) — first sample near 0,
    # last sample near the peak input (~21999).
    assert out_samples[0] >= 0
    assert out_samples[-1] > 20000


def test_resample_downmixes_stereo_to_mono() -> None:
    import numpy as np

    from stackchan_sidecar.tts.espeak import EspeakProvider

    # Stereo: left=100, right=200 interleaved, at the target rate.
    left = np.full(1000, 100, dtype=np.int16)
    right = np.full(1000, 200, dtype=np.int16)
    interleaved = np.stack([left, right], axis=1).reshape(-1).astype(np.int16)
    out = EspeakProvider._resample_and_downmix(
        interleaved.tobytes(), src_rate=PCM_SAMPLE_RATE_HZ, channels=2
    )
    out_samples = np.frombuffer(out, dtype=np.int16)
    assert len(out_samples) == 1000  # no resample, only downmix
    # Mean of 100 + 200 = 150
    assert int(out_samples[0]) == 150


def test_resample_passthrough_when_rate_matches() -> None:
    from stackchan_sidecar.tts.espeak import EspeakProvider

    pcm_in = (b"\x10\x00") * 100  # 100 samples of value 16
    out = EspeakProvider._resample_and_downmix(pcm_in, src_rate=PCM_SAMPLE_RATE_HZ, channels=1)
    # No resample, no downmix → bit-exact passthrough.
    assert out == pcm_in
