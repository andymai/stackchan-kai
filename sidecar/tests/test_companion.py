from __future__ import annotations

from collections.abc import AsyncIterator
from pathlib import Path

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

from stackchan_sidecar.companion import register_companion
from stackchan_sidecar.config import Settings

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
