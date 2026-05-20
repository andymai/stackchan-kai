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
