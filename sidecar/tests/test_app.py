import logging
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from stackchan_sidecar.app import create_app
from stackchan_sidecar.config import Settings
from stackchan_sidecar.llm import Emotion, Reply

from .conftest import TEST_TOKEN, FakeLLM, FakeSTT

_AUDIO_CT = "audio/L16;rate=16000;channels=1"


def test_healthz(client: TestClient) -> None:
    r = client.get("/healthz")
    assert r.status_code == 200
    body = r.json()
    assert body["status"] == "ok"
    assert body["providers"]["stt"] == "ready"
    assert body["providers"]["llm"] == "ready"


def test_listen_happy_path(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
    fake_stt: FakeSTT,
    fake_llm: FakeLLM,
) -> None:
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={
            **auth_headers,
            "Content-Type": _AUDIO_CT,
            "X-Session-Id": "11111111-1111-4111-8111-111111111111",
        },
    )
    assert r.status_code == 200
    body = r.json()
    assert body == {"text": "hi friend!", "emotion": "happy"}
    assert len(fake_stt.calls) == 1
    assert fake_stt.calls[0][1] == 16000
    assert len(fake_llm.calls) == 1
    _, _, session_id, history = fake_llm.calls[0]
    assert session_id == "11111111-1111-4111-8111-111111111111"
    assert history == []


def test_listen_wrong_content_type(
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


def test_listen_empty_body(client: TestClient, auth_headers: dict[str, str]) -> None:
    r = client.post(
        "/v1/listen",
        content=b"",
        headers={**auth_headers, "Content-Type": _AUDIO_CT},
    )
    assert r.status_code == 400


def test_listen_truncates_short_to_32(
    settings: Settings,
    fake_stt: FakeSTT,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    long = "x" * 100
    llm = FakeLLM(Reply(short=long, full=long, emotion=Emotion.NEUTRAL))
    app = create_app(settings, fake_stt, llm)
    with TestClient(app) as c:
        r = c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={**auth_headers, "Content-Type": _AUDIO_CT},
        )
    assert r.status_code == 200
    body = r.json()
    assert len(body["text"]) == 32
    assert body["text"] == "x" * 32


def test_listen_strips_embedded_quotes(
    settings: Settings,
    fake_stt: FakeSTT,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    llm = FakeLLM(Reply(short='hi "there"', full="hi there", emotion=Emotion.NEUTRAL))
    app = create_app(settings, fake_stt, llm)
    with TestClient(app) as c:
        r = c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={**auth_headers, "Content-Type": _AUDIO_CT},
        )
    assert r.status_code == 200
    assert '"' not in r.json()["text"]


def test_listen_persona_missing_returns_500(
    tmp_path: Path,
    fake_stt: FakeSTT,
    fake_llm: FakeLLM,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    empty_personas = tmp_path / "no-personas"
    empty_personas.mkdir()
    settings = Settings(
        SIDECAR_BEARER_TOKEN=TEST_TOKEN,
        ANTHROPIC_API_KEY="sk-ant-test",
        personas_dir=empty_personas,
        persona="missing",
    )
    app = create_app(settings, fake_stt, fake_llm)
    with TestClient(app) as c:
        r = c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={**auth_headers, "Content-Type": _AUDIO_CT},
        )
    assert r.status_code == 500
    body = r.json()
    assert body["error"]["code"] == "persona_missing"
    assert body["error"]["stage"] == "system"
    assert body["text"] == "persona missing"


def test_listen_records_session_history(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
    fake_llm: FakeLLM,
) -> None:
    sid = "33333333-3333-4333-8333-333333333333"
    headers = {**auth_headers, "Content-Type": _AUDIO_CT, "X-Session-Id": sid}
    r1 = client.post("/v1/listen", content=pcm_payload, headers=headers)
    assert r1.status_code == 200
    r2 = client.post("/v1/listen", content=pcm_payload, headers=headers)
    assert r2.status_code == 200

    assert len(fake_llm.calls) == 2
    _, _, _, history_first = fake_llm.calls[0]
    _, _, _, history_second = fake_llm.calls[1]
    assert history_first == []
    assert len(history_second) == 1
    assert history_second[0].user == "hello stack chan"
    assert history_second[0].assistant == "Hi friend! Lovely to meet you."
    assert history_second[0].emotion is Emotion.HAPPY


def test_listen_no_session_history_for_empty_session_id(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
    fake_llm: FakeLLM,
) -> None:
    headers = {**auth_headers, "Content-Type": _AUDIO_CT}
    client.post("/v1/listen", content=pcm_payload, headers=headers)
    client.post("/v1/listen", content=pcm_payload, headers=headers)
    assert len(fake_llm.calls) == 2
    assert fake_llm.calls[0][3] == []
    assert fake_llm.calls[1][3] == []


def test_listen_logs_session_id(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
    caplog: pytest.LogCaptureFixture,
) -> None:
    sid = "22222222-2222-4222-8222-222222222222"
    with caplog.at_level(logging.INFO, logger="stackchan_sidecar"):
        r = client.post(
            "/v1/listen",
            content=pcm_payload,
            headers={
                **auth_headers,
                "Content-Type": _AUDIO_CT,
                "X-Session-Id": sid,
            },
        )
    assert r.status_code == 200
    matched = [rec for rec in caplog.records if getattr(rec, "session_id", "") == sid]
    assert matched, "expected an INFO log carrying the X-Session-Id"
    rec = matched[0]
    assert getattr(rec, "stt_ms", None) is not None
    assert getattr(rec, "llm_ms", None) is not None
    assert getattr(rec, "emotion", None) == "happy"
