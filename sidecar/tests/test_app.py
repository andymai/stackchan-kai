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


def test_listen_rejects_non_mono_audio(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    # `channels=2` body would otherwise reinterpret interleaved
    # stereo int16 as mono, producing garbage transcription. Catch
    # at the content-type gate with a dedicated error code so the
    # operator gets clear feedback.
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={
            **auth_headers,
            "Content-Type": "audio/L16;rate=16000;channels=2",
        },
    )
    assert r.status_code == 415
    body = r.json()
    assert body["error"]["code"] == "audio_channels_unsupported"


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


def test_listen_uses_x_persona_name_header_when_set(
    personas_dir: Path,
    settings: Settings,
    fake_stt: FakeSTT,
    fake_llm: FakeLLM,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    # `personas_dir` fixture seeds `stack-chan.md`; add a second
    # persona the firmware can opt into via header.
    (personas_dir / "desk-buddy.md").write_text(
        "---\nname: desk-buddy\n---\nYou are a quiet desk companion.\n",
        encoding="utf-8",
    )
    app = create_app(settings, fake_stt, fake_llm)
    with TestClient(app) as c:
        r = c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={
                **auth_headers,
                "Content-Type": _AUDIO_CT,
                "X-Persona-Name": "desk-buddy",
            },
        )
    assert r.status_code == 200
    # The LLM saw the desk-buddy persona text, not the default
    # stack-chan one — confirms the header took priority.
    _, persona, _, _ = fake_llm.calls[0]
    assert "quiet desk companion" in persona


def test_listen_empty_x_persona_name_falls_back_to_default(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
    fake_llm: FakeLLM,
) -> None:
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={
            **auth_headers,
            "Content-Type": _AUDIO_CT,
            "X-Persona-Name": "",
        },
    )
    assert r.status_code == 200
    _, persona, _, _ = fake_llm.calls[0]
    # Default `stack-chan.md` was loaded.
    assert "helpful robot" in persona


def test_listen_unknown_x_persona_name_returns_404(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={
            **auth_headers,
            "Content-Type": _AUDIO_CT,
            "X-Persona-Name": "no-such-persona",
        },
    )
    # Distinct from the sidecar-default-misconfig case (500): the
    # caller asked for a specific persona that doesn't exist here.
    assert r.status_code == 404
    body = r.json()
    assert body["error"]["code"] == "persona_missing"


def test_listen_invalid_x_persona_name_returns_400(
    client: TestClient,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    for bad in ["../etc/passwd", "foo/bar", "foo\\bar"]:
        r = client.post(
            "/v1/listen",
            content=pcm_payload,
            headers={
                **auth_headers,
                "Content-Type": _AUDIO_CT,
                "X-Persona-Name": bad,
            },
        )
        assert r.status_code == 400, f"expected 400 for {bad!r}, got {r.status_code}"
        assert r.json()["error"]["code"] == "persona_name_invalid"


def test_listen_empty_default_persona_returns_500_not_400(
    tmp_path: Path,
    fake_stt: FakeSTT,
    fake_llm: FakeLLM,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    # Misconfigured sidecar: settings.persona is empty AND the caller
    # didn't send X-Persona-Name. The client did nothing wrong, so
    # this must be 500 (server misconfig), not 400 (client error).
    # The old catch-all `except ValueError` would have collapsed this
    # into 400; the header-vs-fallback split keeps the two cases
    # distinct.
    empty_personas = tmp_path / "no-personas"
    empty_personas.mkdir()
    settings = Settings(
        SIDECAR_BEARER_TOKEN=TEST_TOKEN,
        ANTHROPIC_API_KEY="sk-ant-test",
        personas_dir=empty_personas,
        persona="",
    )
    app = create_app(settings, fake_stt, fake_llm)
    with TestClient(app) as c:
        r = c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={**auth_headers, "Content-Type": _AUDIO_CT},
        )
    assert r.status_code == 500
    assert r.json()["error"]["code"] == "persona_missing"


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


def test_listen_history_does_not_leak_across_personas(
    personas_dir: Path,
    settings: Settings,
    fake_stt: FakeSTT,
    fake_llm: FakeLLM,
    auth_headers: dict[str, str],
    pcm_payload: bytes,
) -> None:
    # End-to-end through the wire: a device that switches personas
    # under the same session_id must NOT see the prior persona's
    # turns. Pins the per-persona partition all the way from the
    # request handler through the SessionStore.
    (personas_dir / "desk-buddy.md").write_text(
        "---\nname: desk-buddy\n---\nYou are a quiet desk companion.\n",
        encoding="utf-8",
    )
    sid = "44444444-4444-4444-8444-444444444444"
    app = create_app(settings, fake_stt, fake_llm)
    with TestClient(app) as c:
        # Two turns under stack-chan to build up history.
        c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={
                **auth_headers,
                "Content-Type": _AUDIO_CT,
                "X-Session-Id": sid,
                "X-Persona-Name": "stack-chan",
            },
        )
        c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={
                **auth_headers,
                "Content-Type": _AUDIO_CT,
                "X-Session-Id": sid,
                "X-Persona-Name": "stack-chan",
            },
        )
        # Switch to desk-buddy on the SAME session.
        c.post(
            "/v1/listen",
            content=pcm_payload,
            headers={
                **auth_headers,
                "Content-Type": _AUDIO_CT,
                "X-Session-Id": sid,
                "X-Persona-Name": "desk-buddy",
            },
        )

    # First two LLM calls were stack-chan, third was desk-buddy.
    histories = [call[3] for call in fake_llm.calls]
    assert len(histories[0]) == 0, "first call: no prior history"
    assert len(histories[1]) == 1, "second call: one stack-chan turn"
    assert len(histories[2]) == 0, "persona switch: no inherited history"


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
