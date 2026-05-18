import asyncio
from collections.abc import Callable
from pathlib import Path

import httpx
import pytest
from fastapi.testclient import TestClient

from stackchan_sidecar.app import create_app
from stackchan_sidecar.config import Settings
from stackchan_sidecar.llm import Emotion, Reply
from stackchan_sidecar.session_store import Turn

from .conftest import TEST_TOKEN, FakeLLM, FakeSTT

_AUDIO_CT = "audio/L16;rate=16000;channels=1"


@pytest.fixture
def fast_settings(personas_dir: Path) -> Settings:
    return Settings(
        SIDECAR_BEARER_TOKEN=TEST_TOKEN,
        ANTHROPIC_API_KEY="sk-ant-test",
        personas_dir=personas_dir,
        stt_timeout_seconds=0.1,
        llm_timeout_seconds=0.1,
        total_timeout_seconds=0.5,
        stt_max_attempts=2,
        llm_max_attempts=2,
        retry_initial_backoff_seconds=0.0,
    )


class SlowSTT:
    async def transcribe(self, pcm: bytes, sample_rate: int = 16000) -> str:
        await asyncio.sleep(10)
        return "never"


class SlowLLM:
    async def reply(
        self,
        transcript: str,
        persona: str,
        session_id: str,
        history: list[Turn] | None = None,
    ) -> Reply:
        await asyncio.sleep(10)
        return Reply(short="never", full="never", emotion=Emotion.NEUTRAL)


class FlakySTT:
    def __init__(self, fail_count: int, exc_factory: Callable[[], BaseException]) -> None:
        self.fail_count = fail_count
        self.exc_factory = exc_factory
        self.calls = 0

    async def transcribe(self, pcm: bytes, sample_rate: int = 16000) -> str:
        self.calls += 1
        if self.calls <= self.fail_count:
            raise self.exc_factory()
        return "hello recovered"


class CrashSTT:
    async def transcribe(self, pcm: bytes, sample_rate: int = 16000) -> str:
        raise RuntimeError("subprocess crashed")


class ParseFailLLM:
    async def reply(
        self,
        transcript: str,
        persona: str,
        session_id: str,
        history: list[Turn] | None = None,
    ) -> Reply:
        raise ValueError("did not include respond tool_use block")


def _conn_error() -> httpx.ConnectError:
    return httpx.ConnectError("upstream unavailable")


def test_stt_timeout_returns_envelope(
    fast_settings: Settings,
    fake_llm: FakeLLM,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    app = create_app(fast_settings, SlowSTT(), fake_llm)
    with TestClient(app) as c:
        r = c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={**auth_headers, "Content-Type": _AUDIO_CT},
        )
    assert r.status_code == 200
    body = r.json()
    assert body["error"]["code"] == "stt_timeout"
    assert body["error"]["stage"] == "stt"
    assert body["emotion"] == "sad"
    assert body["text"]


def test_llm_timeout_returns_envelope(
    fast_settings: Settings,
    fake_stt: FakeSTT,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    app = create_app(fast_settings, fake_stt, SlowLLM())
    with TestClient(app) as c:
        r = c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={**auth_headers, "Content-Type": _AUDIO_CT},
        )
    assert r.status_code == 200
    body = r.json()
    assert body["error"]["code"] == "llm_timeout"
    assert body["error"]["stage"] == "llm"


def test_stt_transient_then_success(
    fast_settings: Settings,
    fake_llm: FakeLLM,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    flaky = FlakySTT(fail_count=1, exc_factory=_conn_error)
    app = create_app(fast_settings, flaky, fake_llm)
    with TestClient(app) as c:
        r = c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={**auth_headers, "Content-Type": _AUDIO_CT},
        )
    assert r.status_code == 200
    body = r.json()
    assert "error" not in body
    assert flaky.calls == 2


def test_stt_permanent_failure(
    fast_settings: Settings,
    fake_llm: FakeLLM,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    app = create_app(fast_settings, CrashSTT(), fake_llm)
    with TestClient(app) as c:
        r = c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={**auth_headers, "Content-Type": _AUDIO_CT},
        )
    assert r.status_code == 200
    body = r.json()
    assert body["error"]["code"] == "stt_failed"


def test_llm_parse_failure(
    fast_settings: Settings,
    fake_stt: FakeSTT,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    app = create_app(fast_settings, fake_stt, ParseFailLLM())
    with TestClient(app) as c:
        r = c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={**auth_headers, "Content-Type": _AUDIO_CT},
        )
    assert r.status_code == 200
    body = r.json()
    assert body["error"]["code"] == "llm_parse_failed"


def test_audio_rate_unsupported(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={**auth_headers, "Content-Type": "audio/L16;rate=8000;channels=1"},
    )
    assert r.status_code == 415
    body = r.json()
    assert body["error"]["code"] == "audio_rate_unsupported"
    assert body["error"]["stage"] == "audio"


def test_audio_rate_omitted_accepted(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={**auth_headers, "Content-Type": "audio/L16"},
    )
    assert r.status_code == 200


def test_audio_bitrate_does_not_mask_missing_rate(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={**auth_headers, "Content-Type": "audio/L16;bitrate=128000"},
    )
    assert r.status_code == 200


def test_audio_body_too_small(
    client: TestClient,
    auth_headers: dict[str, str],
) -> None:
    too_small = b"\x00" * 100
    r = client.post(
        "/v1/listen",
        content=too_small,
        headers={**auth_headers, "Content-Type": _AUDIO_CT},
    )
    assert r.status_code == 400
    body = r.json()
    assert body["error"]["code"] == "audio_too_small"


def test_envelope_shape_on_validation_error(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={**auth_headers, "Content-Type": "application/octet-stream"},
    )
    assert r.status_code == 415
    body = r.json()
    assert set(body.keys()) == {"text", "emotion", "error"}
    assert set(body["error"].keys()) == {"code", "stage"}
    assert body["error"]["code"] == "bad_content_type"


def test_envelope_shape_on_provider_failure(
    fast_settings: Settings,
    fake_llm: FakeLLM,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    app = create_app(fast_settings, CrashSTT(), fake_llm)
    with TestClient(app) as c:
        r = c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={**auth_headers, "Content-Type": _AUDIO_CT},
        )
    body = r.json()
    assert set(body.keys()) == {"text", "emotion", "error"}
    assert set(body["error"].keys()) == {"code", "stage"}
