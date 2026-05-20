"""Unit tests for the ElevenLabs TTS provider.

These tests never hit the real ElevenLabs API — we monkeypatch the
`httpx.AsyncClient` constructor in the provider module to a stub
that returns canned responses. That covers every wrapping concern
(setup error, auth, non-200, empty body, success) without an API
key or network call.
"""

from __future__ import annotations

from typing import Any

import httpx
import pytest

from stackchan_sidecar.tts import ElevenLabsProvider, TTSError


class _FakeAsyncClient:
    """Stand-in for `httpx.AsyncClient` used in the provider.

    Stores the last call's args + a list of canned responses or
    exceptions. The provider uses it via `async with` so we need
    both async context manager and `.post` methods.
    """

    def __init__(self, response: httpx.Response | Exception, **_kw: Any) -> None:
        self._response = response
        self.last_call: dict[str, Any] | None = None

    async def __aenter__(self) -> _FakeAsyncClient:
        return self

    async def __aexit__(self, *exc_info: Any) -> None:
        return None

    async def post(self, url: str, **kwargs: Any) -> httpx.Response:
        self.last_call = {"url": url, **kwargs}
        if isinstance(self._response, Exception):
            raise self._response
        return self._response


def _install_fake(monkeypatch: pytest.MonkeyPatch, fake: _FakeAsyncClient) -> _FakeAsyncClient:
    """Patch httpx.AsyncClient in the provider module to return `fake`.
    Returned reference is the same as `fake` for caller convenience.
    """

    def factory(**_kw: Any) -> _FakeAsyncClient:
        return fake

    monkeypatch.setattr("stackchan_sidecar.tts.elevenlabs.httpx.AsyncClient", factory)
    return fake


def _mk_response(content: bytes, status_code: int = 200) -> httpx.Response:
    return httpx.Response(
        status_code,
        content=content,
        request=httpx.Request("POST", "https://api.elevenlabs.io/x"),
    )


@pytest.mark.asyncio
async def test_setup_error_when_api_key_missing(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    provider = ElevenLabsProvider(api_key="", voice_id="any")
    with pytest.raises(TTSError) as exc_info:
        await provider.synthesize("hello")
    assert exc_info.value.stage == "setup"


@pytest.mark.asyncio
async def test_rejects_empty_text() -> None:
    provider = ElevenLabsProvider(api_key="sk-test", voice_id="any")
    with pytest.raises(TTSError, match="empty text"):
        await provider.synthesize("")
    with pytest.raises(TTSError, match="empty text"):
        await provider.synthesize("   \n\t")


@pytest.mark.asyncio
async def test_successful_synthesis_returns_raw_pcm(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # 100 even bytes — matches the s16 "byte count must be even" check.
    pcm = b"\x10\x00" * 50
    fake = _install_fake(monkeypatch, _FakeAsyncClient(_mk_response(pcm)))
    result = await ElevenLabsProvider(
        api_key="sk-test", voice_id="V1", model_id="eleven_turbo_v2_5"
    ).synthesize("hello")
    assert result.provider == "elevenlabs"
    assert result.voice == "V1"
    assert result.pcm == pcm
    # Audit the request: voice id in the URL, output_format param,
    # api-key header, model id in the body.
    assert fake.last_call is not None
    assert "V1" in fake.last_call["url"]
    assert fake.last_call["params"]["output_format"] == "pcm_16000"
    assert fake.last_call["headers"]["xi-api-key"] == "sk-test"
    assert fake.last_call["json"]["model_id"] == "eleven_turbo_v2_5"
    assert fake.last_call["json"]["text"] == "hello"


@pytest.mark.asyncio
async def test_non_200_status_raises_synthesize(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _install_fake(
        monkeypatch,
        _FakeAsyncClient(_mk_response(b'{"detail": "quota exhausted"}', 401)),
    )
    with pytest.raises(TTSError) as exc_info:
        await ElevenLabsProvider(api_key="sk-test", voice_id="V1").synthesize("hi")
    assert exc_info.value.stage == "synthesize"
    assert "401" in exc_info.value.detail


@pytest.mark.asyncio
async def test_httpx_connect_error_raises_synthesize(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _install_fake(monkeypatch, _FakeAsyncClient(httpx.ConnectError("dns")))
    with pytest.raises(TTSError) as exc_info:
        await ElevenLabsProvider(api_key="sk-test", voice_id="V1").synthesize("hi")
    assert exc_info.value.stage == "synthesize"
    assert "http error" in exc_info.value.detail


@pytest.mark.asyncio
async def test_empty_body_raises_synthesize(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _install_fake(monkeypatch, _FakeAsyncClient(_mk_response(b"")))
    with pytest.raises(TTSError) as exc_info:
        await ElevenLabsProvider(api_key="sk-test", voice_id="V1").synthesize("hi")
    assert exc_info.value.stage == "synthesize"
    assert "empty" in exc_info.value.detail


@pytest.mark.asyncio
async def test_odd_length_body_raises_transcode(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # s16 → byte count must be even. Odd-length is a clear protocol
    # mismatch (the API returned a different audio container) and
    # we want it surfaced as a transcode-stage error so logs point
    # at the right layer.
    _install_fake(monkeypatch, _FakeAsyncClient(_mk_response(b"\x00\x01\x02")))
    with pytest.raises(TTSError) as exc_info:
        await ElevenLabsProvider(api_key="sk-test", voice_id="V1").synthesize("hi")
    assert exc_info.value.stage == "transcode"
    assert "odd-length" in exc_info.value.detail
