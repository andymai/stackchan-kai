"""Companion endpoints: a static 3D mirror app + relays to the firmware.

Mounted under `/companion/` (the bundled webapp). Three relay routes give
the companion everything it needs while keeping the browser bound to
localhost:

- `GET /v1/state-proxy` — SSE relay of the firmware's `/state/stream`.
- `GET /v1/session-status` — SSE stream of the voice agent's state
  machine (`idle` / `thinking`) plus the last completed turn.
- `POST /v1/firmware-cmd/{listen,emotion}` — allow-listed proxies that
  inject the firmware bearer (held by the sidecar, not the browser)
  before forwarding.
"""

from __future__ import annotations

import asyncio
import json
import logging
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import httpx
from fastapi import APIRouter, FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse, StreamingResponse
from fastapi.staticfiles import StaticFiles

from .config import Settings
from .session_status import SessionStatus, snapshot_to_dict

_LOG = logging.getLogger("stackchan_sidecar.companion")

_UPSTREAM_TIMEOUT = httpx.Timeout(connect=5.0, read=None, write=5.0, pool=5.0)
_CMD_TIMEOUT = httpx.Timeout(connect=3.0, read=5.0, write=3.0, pool=3.0)

# Allow-listed firmware POST endpoints reachable via /v1/firmware-cmd/.
# Anything outside this map 404s — the proxy is intentionally not a
# generic relay so a hostile browser can't pivot through it.
_FIRMWARE_CMD_PATHS: dict[str, str] = {
    "listen": "/listen",
    "emotion": "/emotion",
}


def _resolve_static_dir() -> Path | None:
    # sidecar/src/stackchan_sidecar/companion.py -> sidecar/web/dist
    here = Path(__file__).resolve()
    candidate = here.parents[2] / "web" / "dist"
    if candidate.is_dir() and (candidate / "index.html").is_file():
        return candidate
    return None


def register_companion(app: FastAPI, settings: Settings) -> None:
    """Wire companion routes into an existing FastAPI app.

    Idempotent and side-effect-light: if the static bundle is missing the
    mount is skipped and a single warning is logged. The relay routes
    register regardless so the sidecar process is still useful when only
    the firmware bundle is rebuilt.
    """
    if not settings.companion_enabled:
        _LOG.info("companion disabled via SIDECAR_COMPANION_ENABLED=false")
        return

    router = APIRouter()

    @router.get("/v1/state-proxy")
    async def state_proxy(request: Request) -> StreamingResponse:
        upstream = settings.firmware_url.rstrip("/") + "/state/stream"
        headers = {"Accept": "text/event-stream"}
        if settings.firmware_token:
            headers["Authorization"] = f"Bearer {settings.firmware_token}"

        client = httpx.AsyncClient(timeout=_UPSTREAM_TIMEOUT)

        async def stream() -> AsyncIterator[bytes]:
            try:
                async with client.stream("GET", upstream, headers=headers) as resp:
                    if resp.status_code != 200:
                        text = await resp.aread()
                        _LOG.warning(
                            "state-proxy upstream %s returned %s: %s",
                            upstream,
                            resp.status_code,
                            text[:200],
                        )
                        yield (
                            b"event: error\ndata: upstream "
                            + str(resp.status_code).encode()
                            + b"\n\n"
                        )
                        return
                    async for chunk in resp.aiter_raw():
                        if await request.is_disconnected():
                            break
                        if chunk:
                            yield chunk
            except httpx.HTTPError as e:
                _LOG.warning("state-proxy transport error: %s", e)
                yield b"event: error\ndata: transport\n\n"
            finally:
                await client.aclose()

        return StreamingResponse(
            stream(),
            media_type="text/event-stream",
            headers={"Cache-Control": "no-store", "X-Accel-Buffering": "no"},
        )

    @router.get("/v1/session-status")
    async def session_status_stream(request: Request) -> StreamingResponse:
        status: SessionStatus | None = getattr(request.app.state, "session_status", None)
        if status is None:
            raise HTTPException(status_code=503, detail="session-status not initialised")

        async def stream() -> AsyncIterator[bytes]:
            # Initial frame so a fresh subscriber paints the right state
            # without waiting for the next transition.
            yield _format_status(status.get())
            # Short timeout keeps us responsive to client disconnects on
            # ASGI servers that don't immediately cancel the generator;
            # also acts as the SSE keepalive heartbeat for proxies.
            while not await request.is_disconnected():
                try:
                    await asyncio.wait_for(status.changed(), timeout=2.0)
                except TimeoutError:
                    yield b": keepalive\n\n"
                    continue
                yield _format_status(status.get())

        return StreamingResponse(
            stream(),
            media_type="text/event-stream",
            headers={"Cache-Control": "no-store", "X-Accel-Buffering": "no"},
        )

    @router.post("/v1/firmware-cmd/{name}")
    async def firmware_cmd(name: str, request: Request) -> JSONResponse:
        upstream_path = _FIRMWARE_CMD_PATHS.get(name)
        if upstream_path is None:
            raise HTTPException(status_code=404, detail=f"unknown firmware command: {name}")

        upstream = settings.firmware_url.rstrip("/") + upstream_path
        try:
            body = await request.json()
        except json.JSONDecodeError:
            raise HTTPException(status_code=400, detail="body must be JSON") from None

        headers = {"Content-Type": "application/json"}
        if settings.firmware_token:
            headers["Authorization"] = f"Bearer {settings.firmware_token}"

        try:
            async with httpx.AsyncClient(timeout=_CMD_TIMEOUT) as client:
                resp = await client.post(upstream, json=body, headers=headers)
        except httpx.HTTPError as e:
            _LOG.warning("firmware-cmd %s transport error: %s", name, e)
            raise HTTPException(status_code=502, detail="firmware unreachable") from None

        if resp.status_code >= 400:
            _LOG.info("firmware-cmd %s returned %s: %s", name, resp.status_code, resp.text[:200])

        ct = resp.headers.get("content-type", "")
        payload: Any
        if ct.startswith("application/json"):
            try:
                payload = resp.json()
            except json.JSONDecodeError:
                payload = {"raw": resp.text}
        else:
            payload = {"raw": resp.text}
        return JSONResponse(payload, status_code=resp.status_code)

    @router.get("/companion/healthz")
    async def companion_healthz() -> dict[str, object]:
        return {"status": "ok", "firmware_url": settings.firmware_url}

    app.include_router(router)

    static_dir = _resolve_static_dir()
    if static_dir is None:
        _LOG.warning(
            "companion static bundle not found; run `just sidecar-companion-build` to build "
            "sidecar/web/. The /v1/state-proxy SSE relay is still active."
        )
        return

    app.mount("/companion", StaticFiles(directory=static_dir, html=True), name="companion")
    _LOG.info("companion mounted: static=%s upstream=%s", static_dir, settings.firmware_url)


def _format_status(snapshot: object) -> bytes:
    # `SessionStatus.get()` returns a frozen dataclass; reuse the helper
    # so the wire shape stays the test-asserted one.
    from .session_status import Snapshot

    payload = snapshot_to_dict(snapshot) if isinstance(snapshot, Snapshot) else {}
    return b"data: " + json.dumps(payload, separators=(",", ":")).encode() + b"\n\n"
