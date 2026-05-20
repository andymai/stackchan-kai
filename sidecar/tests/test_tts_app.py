"""End-to-end wire tests for the TTS integration.

Uses a `FakeTTS` so the tests don't depend on espeak-ng being
installed — covers the listen-handler-to-audio-endpoint round-trip
plus the failure paths.
"""

from __future__ import annotations

from collections.abc import Iterator

import pytest
from fastapi.testclient import TestClient

from stackchan_sidecar.app import create_app
from stackchan_sidecar.audio_cache import AudioCache
from stackchan_sidecar.config import Settings
from stackchan_sidecar.tts import TTSError, TTSResult

from .conftest import TEST_TOKEN, FakeLLM, FakeSTT

_AUDIO_CT = "audio/L16;rate=16000;channels=1"


class FakeTTS:
    """Test double that returns a deterministic PCM payload."""

    name = "fake"

    def __init__(self, *, pcm: bytes = b"\x12\x34" * 100, voice: str = "fake-voice") -> None:
        self.pcm = pcm
        self.voice = voice
        self.calls: list[str] = []
        self.fail_with: TTSError | None = None

    async def synthesize(self, text: str) -> TTSResult:
        self.calls.append(text)
        if self.fail_with is not None:
            raise self.fail_with
        return TTSResult(pcm=self.pcm, provider=self.name, voice=self.voice)


@pytest.fixture
def fake_tts() -> FakeTTS:
    return FakeTTS()


@pytest.fixture
def tts_client(
    settings: Settings,
    fake_stt: FakeSTT,
    fake_llm: FakeLLM,
    fake_tts: FakeTTS,
) -> Iterator[TestClient]:
    app = create_app(settings, fake_stt, fake_llm, tts=fake_tts)
    with TestClient(app) as c:
        yield c


def test_listen_returns_audio_url_when_tts_succeeds(
    tts_client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
    fake_tts: FakeTTS,
) -> None:
    r = tts_client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={**auth_headers, "Content-Type": _AUDIO_CT},
    )
    assert r.status_code == 200
    body = r.json()
    assert body["text"] == "hi friend!"
    assert body["emotion"] == "happy"
    assert body["audio_url"] is not None
    assert body["audio_url"].startswith("/v1/audio/")
    # TTS was called with the *short* (toast-band-sized) text, not the
    # longer `reply.full`.
    assert fake_tts.calls == ["hi friend!"]


def test_audio_endpoint_returns_cached_pcm(
    tts_client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
    fake_tts: FakeTTS,
) -> None:
    r = tts_client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={**auth_headers, "Content-Type": _AUDIO_CT},
    )
    audio_url = r.json()["audio_url"]
    audio_r = tts_client.get(audio_url)
    assert audio_r.status_code == 200
    assert audio_r.content == fake_tts.pcm
    # Format header signals the firmware can stream straight in.
    assert audio_r.headers["content-type"].startswith("audio/L16")
    assert audio_r.headers["x-audio-provider"] == "fake"
    assert audio_r.headers["x-audio-voice"] == "fake-voice"


def test_audio_endpoint_404s_for_unknown_token(tts_client: TestClient) -> None:
    r = tts_client.get("/v1/audio/deadbeefdeadbeefdeadbeefdeadbeef")
    assert r.status_code == 404


def test_audio_endpoint_404s_after_ttl_expiry(
    settings: Settings,
    fake_stt: FakeSTT,
    fake_llm: FakeLLM,
    fake_tts: FakeTTS,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import time as _time

    # Stand up an app with a fast-expiring cache so the TTL path is
    # exercisable without sleeping in real time.
    now = [1000.0]
    monkeypatch.setattr(_time, "monotonic", lambda: now[0])
    cache = AudioCache(ttl_seconds=2.0, capacity=8)
    app = create_app(settings, fake_stt, fake_llm, tts=fake_tts, audio_cache=cache)
    with TestClient(app) as c:
        r = c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={**auth_headers, "Content-Type": _AUDIO_CT},
        )
        audio_url = r.json()["audio_url"]
        # Immediate fetch hits.
        assert c.get(audio_url).status_code == 200
        # Past TTL — eviction on next access returns 404.
        now[0] += 10.0
        assert c.get(audio_url).status_code == 404


def test_listen_null_audio_url_when_tts_fails(
    tts_client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
    fake_tts: FakeTTS,
) -> None:
    fake_tts.fail_with = TTSError("synthesize", "provider blew up")
    r = tts_client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={**auth_headers, "Content-Type": _AUDIO_CT},
    )
    # Graceful degradation: text + emotion still ship, listen
    # responded 200, only audio_url is null.
    assert r.status_code == 200
    body = r.json()
    assert body["text"] == "hi friend!"
    assert body["emotion"] == "happy"
    assert body["audio_url"] is None


def test_listen_null_audio_url_when_tts_provider_unset(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    # The default `client` fixture wires the app with `tts=None`.
    # The reply must still include the `audio_url` field (as null) so
    # the firmware's wire-format expectation is stable across deploy
    # configurations.
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={**auth_headers, "Content-Type": _AUDIO_CT},
    )
    assert r.status_code == 200
    body = r.json()
    assert body["audio_url"] is None


def test_listen_null_audio_url_when_tts_raises_unexpected(
    tts_client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
    fake_tts: FakeTTS,
) -> None:
    # A bug in the provider (RuntimeError, ImportError, etc.) must
    # not take down the reply path — the catch-all logs but degrades
    # to no-audio.
    class _Boom:
        name = "boom"

        async def synthesize(self, text: str) -> TTSResult:
            raise RuntimeError("unexpected")

    settings = Settings(SIDECAR_BEARER_TOKEN=TEST_TOKEN, ANTHROPIC_API_KEY="sk-ant-test")
    app = create_app(
        settings,
        FakeSTT(),
        FakeLLM(),
        tts=_Boom(),
    )
    # Drop a persona so the listen path doesn't 500 on missing default.
    settings.personas_dir.mkdir(exist_ok=True)
    persona_path = settings.personas_dir / f"{settings.persona}.md"
    if not persona_path.exists():
        persona_path.write_text("---\n---\nfallback", encoding="utf-8")
    try:
        with TestClient(app) as c:
            r = c.post(
                "/v1/listen",
                content=pcm_payload,
                headers={**auth_headers, "Content-Type": _AUDIO_CT},
            )
        assert r.status_code == 200
        assert r.json()["audio_url"] is None
    finally:
        # Clean up the persona file we maybe just created.
        if persona_path.exists() and persona_path.read_text(encoding="utf-8").strip() == "fallback":
            persona_path.unlink()
