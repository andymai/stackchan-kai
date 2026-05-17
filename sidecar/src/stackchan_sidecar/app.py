import logging
import time
from uuid import uuid4

from fastapi import Depends, FastAPI, HTTPException, Request, status
from fastapi.responses import JSONResponse

from .auth import make_verifier
from .config import Settings
from .llm import Emotion, LLMProvider
from .personas import load_persona
from .stt import STTProvider

_LOG = logging.getLogger("stackchan_sidecar")
_MAX_BODY_BYTES = 30 * 16000 * 2 + 1024
_SHORT_MAX = 32


def create_app(
    settings: Settings,
    stt: STTProvider,
    llm: LLMProvider,
) -> FastAPI:
    app = FastAPI(title="stackchan-sidecar", version="0.1.0")
    verify_bearer = make_verifier(settings.bearer_token)

    @app.get("/healthz")
    async def healthz() -> dict[str, object]:
        return {
            "status": "ok",
            "providers": {"stt": "ready", "llm": "ready"},
        }

    @app.post("/v1/listen", dependencies=[Depends(verify_bearer)])
    async def listen(request: Request) -> JSONResponse:
        request_id = request.headers.get("x-request-id") or str(uuid4())
        session_id = request.headers.get("x-session-id", "")
        content_type = request.headers.get("content-type", "")

        if not content_type.lower().startswith("audio/l16"):
            _LOG.warning(
                "rejecting non-audio content-type",
                extra={
                    "request_id": request_id,
                    "session_id": session_id,
                    "content_type": content_type,
                    "status": 415,
                },
            )
            raise HTTPException(
                status_code=status.HTTP_415_UNSUPPORTED_MEDIA_TYPE,
                detail="expected Content-Type: audio/L16;rate=16000;channels=1",
            )

        body = await request.body()
        if not body:
            _LOG.warning(
                "rejecting empty body",
                extra={
                    "request_id": request_id,
                    "session_id": session_id,
                    "status": 400,
                },
            )
            raise HTTPException(
                status_code=status.HTTP_400_BAD_REQUEST,
                detail="request body is empty",
            )
        if len(body) > _MAX_BODY_BYTES:
            raise HTTPException(
                status_code=status.HTTP_413_REQUEST_ENTITY_TOO_LARGE,
                detail=f"audio payload exceeds {_MAX_BODY_BYTES} bytes",
            )

        t0 = time.perf_counter()
        try:
            persona = load_persona(settings.persona, settings.personas_dir)
        except FileNotFoundError as e:
            _LOG.exception(
                "persona load failed",
                extra={
                    "request_id": request_id,
                    "session_id": session_id,
                    "persona": settings.persona,
                    "status": 500,
                },
            )
            raise HTTPException(
                status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
                detail=f"persona unavailable: {e}",
            ) from e

        try:
            t_stt0 = time.perf_counter()
            transcript = await stt.transcribe(body, sample_rate=16000)
            stt_ms = int((time.perf_counter() - t_stt0) * 1000)

            t_llm0 = time.perf_counter()
            reply = await llm.reply(transcript, persona, session_id)
            llm_ms = int((time.perf_counter() - t_llm0) * 1000)
        except HTTPException:
            raise
        except Exception:
            _LOG.exception(
                "pipeline failure",
                extra={
                    "request_id": request_id,
                    "session_id": session_id,
                    "status": 500,
                },
            )
            raise HTTPException(
                status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
                detail="internal error",
            ) from None

        short = reply.short[:_SHORT_MAX].replace('"', "'")
        emotion = reply.emotion if isinstance(reply.emotion, Emotion) else Emotion.NEUTRAL
        total_ms = int((time.perf_counter() - t0) * 1000)

        _LOG.info(
            "listen.ok",
            extra={
                "request_id": request_id,
                "session_id": session_id,
                "stt_ms": stt_ms,
                "llm_ms": llm_ms,
                "total_ms": total_ms,
                "emotion": emotion.value,
                "text_len": len(short),
                "text_short": short,
                "status": 200,
            },
        )

        return JSONResponse({"text": short, "emotion": emotion.value})

    return app
