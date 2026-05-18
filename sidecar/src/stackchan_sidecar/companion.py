"""Companion endpoints: a static 3D mirror app + an SSE relay from the firmware.

Mounted under `/companion/` (the bundled webapp) and `/v1/state-proxy` (a
relay of the firmware's `/state/stream`). The firmware exposes SSE only on
its own origin; rather than ask it for CORS, we proxy through the sidecar,
keeping a single auth surface for the operator and letting the companion
talk to localhost.
"""

from __future__ import annotations

import logging
from collections.abc import AsyncIterator
from pathlib import Path

import httpx
from fastapi import APIRouter, FastAPI, HTTPException, Request
from fastapi.responses import StreamingResponse
from fastapi.staticfiles import StaticFiles

from .config import Settings

_LOG = logging.getLogger("stackchan_sidecar.companion")

_UPSTREAM_TIMEOUT = httpx.Timeout(connect=5.0, read=None, write=5.0, pool=5.0)


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
    mount is skipped and a single warning is logged. The state-proxy SSE
    route registers regardless so the sidecar process is still useful when
    only the firmware bundle is rebuilt.
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
