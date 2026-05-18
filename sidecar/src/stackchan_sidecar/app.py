import logging
import re
import time
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from uuid import uuid4

from fastapi import Depends, FastAPI, Request
from fastapi.responses import JSONResponse

from .auth import make_verifier
from .companion import register_companion
from .config import Settings
from .session_status import SessionStatus
from .errors import (
    ErrorCode,
    audio_validation_status,
    build_envelope,
    failure_kind,
)
from .llm import Emotion, LLMProvider, sanitize_short
from .personas import load_persona
from .retry import StageDeadlineError, retry_with_timeout
from .session_store import SessionStore, Turn, session_store_lifespan
from .stt import STTProvider

_LOG = logging.getLogger("stackchan_sidecar")
_MAX_BODY_BYTES = 30 * 16000 * 2 + 1024
_MIN_BODY_BYTES = 640  # 20 ms @ 16 kHz mono s16; anything smaller is operator error
_EXPECTED_SAMPLE_RATE = 16000
_RATE_RE = re.compile(r"\brate=(\d+)", re.IGNORECASE)


def _failure(
    code: ErrorCode,
    *,
    request_id: str,
    session_id: str,
    status: int,
    session_status: SessionStatus | None = None,
    extra: dict[str, object] | None = None,
) -> JSONResponse:
    kind = failure_kind(code)
    log_extra: dict[str, object] = {
        "request_id": request_id,
        "session_id": session_id,
        "code": kind.code.value,
        "stage": kind.stage.value,
        "status": status,
    }
    if extra:
        log_extra.update(extra)
    _LOG.warning("listen.fail", extra=log_extra)
    if session_status is not None:
        session_status.mark_failed(
            request_id=request_id,
            session_id=session_id,
            error=kind.code.value,
        )
    return JSONResponse(build_envelope(kind), status_code=status)


def _audio_failure(
    code: ErrorCode,
    *,
    request_id: str,
    session_id: str,
    session_status: SessionStatus | None = None,
) -> JSONResponse:
    return _failure(
        code,
        request_id=request_id,
        session_id=session_id,
        status=audio_validation_status(code),
        session_status=session_status,
    )


def _validate_content_type(content_type: str) -> ErrorCode | None:
    if not content_type.lower().startswith("audio/l16"):
        return ErrorCode.BAD_CONTENT_TYPE
    match = _RATE_RE.search(content_type)
    if match is None:
        return None
    if int(match.group(1)) != _EXPECTED_SAMPLE_RATE:
        return ErrorCode.AUDIO_RATE_UNSUPPORTED
    return None


def create_app(
    settings: Settings,
    stt: STTProvider,
    llm: LLMProvider,
    session_store: SessionStore | None = None,
) -> FastAPI:
    store = session_store if session_store is not None else SessionStore()

    @asynccontextmanager
    async def lifespan(app: FastAPI) -> AsyncIterator[None]:
        async with session_store_lifespan(app, store):
            yield

    app = FastAPI(title="stackchan-sidecar", version="0.1.0", lifespan=lifespan)
    session_status = SessionStatus()
    app.state.session_status = session_status
    verify_bearer = make_verifier(settings.bearer_token)
    register_companion(app, settings)

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

        ct_err = _validate_content_type(content_type)
        if ct_err is not None:
            return _audio_failure(
                ct_err,
                request_id=request_id,
                session_id=session_id,
                session_status=session_status,
            )

        declared_length = request.headers.get("content-length")
        if declared_length is not None:
            try:
                declared = int(declared_length)
            except ValueError:
                return _audio_failure(
                    ErrorCode.BAD_CONTENT_LENGTH,
                    request_id=request_id,
                    session_id=session_id,
                    session_status=session_status,
                )
            if declared > _MAX_BODY_BYTES:
                return _audio_failure(
                    ErrorCode.AUDIO_TOO_LARGE,
                    request_id=request_id,
                    session_id=session_id,
                    session_status=session_status,
                )

        body = await request.body()
        if not body:
            return _audio_failure(
                ErrorCode.AUDIO_EMPTY,
                request_id=request_id,
                session_id=session_id,
                session_status=session_status,
            )
        if len(body) > _MAX_BODY_BYTES:
            return _audio_failure(
                ErrorCode.AUDIO_TOO_LARGE,
                request_id=request_id,
                session_id=session_id,
                session_status=session_status,
            )
        if len(body) < _MIN_BODY_BYTES:
            return _audio_failure(
                ErrorCode.AUDIO_TOO_SMALL,
                request_id=request_id,
                session_id=session_id,
                session_status=session_status,
            )

        session_status.mark_thinking(request_id=request_id, session_id=session_id)

        t0 = time.perf_counter()
        deadline = time.monotonic() + settings.total_timeout_seconds

        try:
            persona = load_persona(settings.persona, settings.personas_dir)
        except FileNotFoundError:
            _LOG.exception(
                "persona load failed",
                extra={
                    "request_id": request_id,
                    "session_id": session_id,
                    "persona": settings.persona,
                    "status": 500,
                },
            )
            return _failure(
                ErrorCode.PERSONA_MISSING,
                request_id=request_id,
                session_id=session_id,
                status=500,
                session_status=session_status,
                extra={"persona": settings.persona},
            )

        t_stt0 = time.perf_counter()
        try:
            transcript = await retry_with_timeout(
                lambda: stt.transcribe(body, sample_rate=_EXPECTED_SAMPLE_RATE),
                max_attempts=settings.stt_max_attempts,
                per_attempt_timeout=settings.stt_timeout_seconds,
                deadline=deadline,
                initial_backoff=settings.retry_initial_backoff_seconds,
                label="stt",
            )
        except (TimeoutError, StageDeadlineError):
            return _failure(
                ErrorCode.STT_TIMEOUT,
                request_id=request_id,
                session_id=session_id,
                status=200,
                session_status=session_status,
            )
        except Exception:
            _LOG.exception(
                "stt failed",
                extra={"request_id": request_id, "session_id": session_id, "stage": "stt"},
            )
            return _failure(
                ErrorCode.STT_FAILED,
                request_id=request_id,
                session_id=session_id,
                status=200,
                session_status=session_status,
            )
        stt_ms = int((time.perf_counter() - t_stt0) * 1000)

        history = store.get_history(session_id)
        t_llm0 = time.perf_counter()
        try:
            reply = await retry_with_timeout(
                lambda: llm.reply(transcript, persona, session_id, history),
                max_attempts=settings.llm_max_attempts,
                per_attempt_timeout=settings.llm_timeout_seconds,
                deadline=deadline,
                initial_backoff=settings.retry_initial_backoff_seconds,
                label="llm",
            )
        except (TimeoutError, StageDeadlineError):
            return _failure(
                ErrorCode.LLM_TIMEOUT,
                request_id=request_id,
                session_id=session_id,
                status=200,
                session_status=session_status,
            )
        except ValueError:
            _LOG.exception(
                "llm parse failed",
                extra={"request_id": request_id, "session_id": session_id, "stage": "llm"},
            )
            return _failure(
                ErrorCode.LLM_PARSE_FAILED,
                request_id=request_id,
                session_id=session_id,
                status=200,
                session_status=session_status,
            )
        except Exception:
            _LOG.exception(
                "llm failed",
                extra={"request_id": request_id, "session_id": session_id, "stage": "llm"},
            )
            return _failure(
                ErrorCode.LLM_FAILED,
                request_id=request_id,
                session_id=session_id,
                status=200,
                session_status=session_status,
            )
        llm_ms = int((time.perf_counter() - t_llm0) * 1000)

        short = sanitize_short(reply.short)
        emotion = reply.emotion if isinstance(reply.emotion, Emotion) else Emotion.NEUTRAL
        total_ms = int((time.perf_counter() - t0) * 1000)

        store.record(
            session_id,
            Turn(user=transcript, assistant=reply.full, emotion=emotion),
        )

        session_status.mark_done(
            request_id=request_id,
            session_id=session_id,
            transcript=transcript,
            reply_short=short,
            emotion=emotion.value,
        )

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
