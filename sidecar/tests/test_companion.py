from __future__ import annotations

import typing
from collections.abc import AsyncIterator
from pathlib import Path

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

from stackchan_sidecar.companion import register_companion
from stackchan_sidecar.config import Settings
from stackchan_sidecar.session_status import SessionStatus

from .conftest import TEST_TOKEN


def _settings(**overrides: object) -> Settings:
    base: dict[str, object] = {
        "SIDECAR_BEARER_TOKEN": TEST_TOKEN,
        "ANTHROPIC_API_KEY": "sk-ant-test",
    }
    base.update(overrides)
    return Settings(**base)  # type: ignore[arg-type]


def test_register_companion_noop_when_disabled() -> None:
    app = FastAPI()
    register_companion(app, _settings(SIDECAR_COMPANION_ENABLED=False))
    paths = {r.path for r in app.routes}  # type: ignore[attr-defined]
    assert "/v1/state-proxy" not in paths
    assert "/companion/healthz" not in paths


def test_register_companion_registers_routes_when_enabled() -> None:
    app = FastAPI()
    register_companion(app, _settings())
    paths = {r.path for r in app.routes}  # type: ignore[attr-defined]
    assert "/v1/state-proxy" in paths
    assert "/v1/session-status" in paths
    assert "/v1/firmware-cmd/{name}" in paths
    assert "/companion/healthz" in paths


def test_companion_healthz_returns_firmware_url() -> None:
    app = FastAPI()
    register_companion(app, _settings(STACKCHAN_FIRMWARE_URL="http://example.local"))
    with TestClient(app) as c:
        r = c.get("/companion/healthz")
    assert r.status_code == 200
    body = r.json()
    assert body["status"] == "ok"
    assert body["firmware_url"] == "http://example.local"


def test_state_proxy_relays_upstream_error_envelope(monkeypatch: pytest.MonkeyPatch) -> None:
    # If the firmware refuses, the sidecar must surface a parseable SSE error
    # frame rather than tearing down the operator's connection silently.
    class FakeResp:
        status_code = 503

        async def aread(self) -> bytes:
            return b"upstream offline"

        async def aiter_raw(self) -> AsyncIterator[bytes]:  # pragma: no cover
            if False:
                yield b""

    class FakeStream:
        async def __aenter__(self) -> FakeResp:
            return FakeResp()

        async def __aexit__(self, *_: object) -> None:
            return None

    class FakeClient:
        def __init__(self, *_: object, **__: object) -> None:
            pass

        def stream(self, *_: object, **__: object) -> FakeStream:
            return FakeStream()

        async def aclose(self) -> None:
            return None

    monkeypatch.setattr("stackchan_sidecar.companion.httpx.AsyncClient", FakeClient)

    app = FastAPI()
    register_companion(app, _settings())
    with TestClient(app) as c:
        r = c.get("/v1/state-proxy")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/event-stream")
    assert b"event: error" in r.content
    assert b"upstream 503" in r.content


def test_register_companion_skips_static_mount_when_dist_missing(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr("stackchan_sidecar.companion._resolve_static_dir", lambda: None)
    app = FastAPI()
    register_companion(app, _settings())
    # SSE route still registers regardless of bundle presence.
    paths = {r.path for r in app.routes}  # type: ignore[attr-defined]
    assert "/v1/state-proxy" in paths
    # No StaticFiles mount under /companion.
    mounts = [r for r in app.routes if getattr(r, "name", None) == "companion"]
    assert mounts == []


def test_register_companion_mounts_static_when_dist_exists(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    fake_dist = tmp_path / "web" / "dist"
    fake_dist.mkdir(parents=True)
    (fake_dist / "index.html").write_text("<html><body>companion</body></html>", encoding="utf-8")
    monkeypatch.setattr("stackchan_sidecar.companion._resolve_static_dir", lambda: fake_dist)

    app = FastAPI()
    register_companion(app, _settings())
    with TestClient(app) as c:
        r = c.get("/companion/")
    assert r.status_code == 200
    assert "companion" in r.text


def _app_with_status(status: SessionStatus, **settings_overrides: object) -> FastAPI:
    app = FastAPI()
    app.state.session_status = status
    register_companion(app, _settings(**settings_overrides))
    return app


def test_session_status_sse_route_resolves_against_app_state() -> None:
    # Direct streaming-body assertions hang under httpx's ASGI transport
    # (it doesn't propagate http.disconnect to request.is_disconnected(),
    # so the keepalive loop never exits). The SessionStatus state machine
    # and `_format_status` are exercised by test_session_status.py; here
    # we just confirm the route reads `app.state.session_status` and 503s
    # cleanly when it's absent.
    app = FastAPI()
    register_companion(app, _settings())
    with TestClient(app) as c:
        r = c.get("/v1/session-status")
    # No app.state.session_status configured → 503, no hang.
    assert r.status_code == 503

    # With state wired the route reports text/event-stream on first byte.
    # We can't safely consume the body in TestClient, so just check the
    # response object's media_type via a HEAD-style probe is impossible;
    # the route-registered assertion above covers the wire-up.


def test_firmware_cmd_unknown_name_404s() -> None:
    status = SessionStatus()
    app = _app_with_status(status)
    with TestClient(app) as c:
        r = c.post("/v1/firmware-cmd/unknown", json={})
    assert r.status_code == 404


def test_firmware_cmd_forwards_body_and_bearer(monkeypatch: pytest.MonkeyPatch) -> None:
    captured: dict[str, object] = {}

    class FakeResp:
        status_code = 200
        text = '{"ok":true}'
        headers = {"content-type": "application/json"}

        def json(self) -> dict[str, object]:
            return {"ok": True}

    class FakeClient:
        def __init__(self, *_: object, **__: object) -> None:
            pass

        async def __aenter__(self) -> "FakeClient":
            return self

        async def __aexit__(self, *_: object) -> None:
            return None

        async def post(self, url: str, json: object, headers: dict[str, str]) -> FakeResp:
            captured["url"] = url
            captured["json"] = json
            captured["headers"] = headers
            return FakeResp()

    monkeypatch.setattr("stackchan_sidecar.companion.httpx.AsyncClient", FakeClient)

    status = SessionStatus()
    app = _app_with_status(
        status,
        STACKCHAN_FIRMWARE_URL="http://stackchan.example",
        STACKCHAN_FIRMWARE_TOKEN="fw-secret",
    )
    with TestClient(app) as c:
        r = c.post(
            "/v1/firmware-cmd/emotion",
            json={"emotion": "happy", "hold_ms": 30000},
        )
    assert r.status_code == 200
    assert r.json() == {"ok": True}
    assert captured["url"] == "http://stackchan.example/emotion"
    assert captured["json"] == {"emotion": "happy", "hold_ms": 30000}
    assert captured["headers"]["Authorization"] == "Bearer fw-secret"  # type: ignore[index]


def test_firmware_cmd_502_when_firmware_unreachable(monkeypatch: pytest.MonkeyPatch) -> None:
    import httpx

    class FakeClient:
        def __init__(self, *_: object, **__: object) -> None:
            pass

        async def __aenter__(self) -> "FakeClient":
            return self

        async def __aexit__(self, *_: object) -> None:
            return None

        async def post(self, *_: object, **__: object) -> None:
            raise httpx.ConnectError("nope")

    monkeypatch.setattr("stackchan_sidecar.companion.httpx.AsyncClient", FakeClient)

    status = SessionStatus()
    app = _app_with_status(status)
    with TestClient(app) as c:
        r = c.post("/v1/firmware-cmd/listen", json={"duration_ms": 4000})
    assert r.status_code == 502
