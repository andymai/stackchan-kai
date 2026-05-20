import logging
import re
import time
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from uuid import uuid4

from fastapi import Depends, FastAPI, Request
from fastapi.responses import JSONResponse, Response

from .audio_cache import AudioCache
from .auth import make_verifier
from .companion import register_companion
from .config import Settings
from .errors import (
    ErrorCode,
    audio_validation_status,
    build_envelope,
    failure_kind,
)
from .llm import Emotion, LLMProvider, sanitize_short
from .personas import list_personas, load_persona
from .retry import StageDeadlineError, retry_with_timeout
from .session_status import SessionStatus
from .session_store import SessionStore, Turn, session_store_lifespan
from .stt import STTProvider
from .tts import TTSError, TTSProvider

_LOG = logging.getLogger("stackchan_sidecar")
_MAX_BODY_BYTES = 30 * 16000 * 2 + 1024
_MIN_BODY_BYTES = 640  # 20 ms @ 16 kHz mono s16; anything smaller is operator error
_EXPECTED_SAMPLE_RATE = 16000
_RATE_RE = re.compile(r"\brate=(\d+)", re.IGNORECASE)
_CHANNELS_RE = re.compile(r"\bchannels=(\d+)", re.IGNORECASE)


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


def _audio_failure(code: ErrorCode, *, request_id: str, session_id: str) -> JSONResponse:
    # Pre-flight validation failures (bad content type, body size) happen
    # before `mark_thinking` — the state machine never entered "thinking"
    # for this request, so we don't poke session_status here.
    return _failure(
        code,
        request_id=request_id,
        session_id=session_id,
        status=audio_validation_status(code),
    )


def _validate_content_type(content_type: str) -> ErrorCode | None:
    if not content_type.lower().startswith("audio/l16"):
        return ErrorCode.BAD_CONTENT_TYPE
    rate_match = _RATE_RE.search(content_type)
    if rate_match is not None and int(rate_match.group(1)) != _EXPECTED_SAMPLE_RATE:
        return ErrorCode.AUDIO_RATE_UNSUPPORTED
    # `channels` defaults to 1 when absent — matches the firmware
    # `audio/L16;rate=16000;channels=1` Content-Type and the
    # downstream STT contract. A `channels=2` body would otherwise
    # be reinterpreted as mono int16 in `_transcribe_sync`'s
    # `np.frombuffer(pcm, dtype=np.int16)` — interleaved stereo
    # decoded as mono yields garbage transcription.
    channels_match = _CHANNELS_RE.search(content_type)
    if channels_match is not None and int(channels_match.group(1)) != 1:
        return ErrorCode.AUDIO_CHANNELS_UNSUPPORTED
    return None


def create_app(
    settings: Settings,
    stt: STTProvider,
    llm: LLMProvider,
    session_store: SessionStore | None = None,
    tts: TTSProvider | None = None,
    audio_cache: AudioCache | None = None,
) -> FastAPI:
    store = session_store if session_store is not None else SessionStore()
    # `tts is None` → no synthesis; the listen path returns
    # `audio_url: null` and the firmware silently skips the audio
    # fetch. Lets a deployment that just wants STT + LLM (existing
    # behaviour) skip pulling the TTS subpackage at all.
    cache = (
        audio_cache
        if audio_cache is not None
        else AudioCache(
            ttl_seconds=settings.tts_audio_ttl_seconds,
            capacity=settings.tts_audio_cache_capacity,
        )
    )

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

    @app.get("/v1/audio/{token}", dependencies=[Depends(verify_bearer)])
    async def audio_endpoint(token: str) -> Response:
        # Streams the cached PCM the firmware fetches after a /v1/listen
        # reply. Tokens are opaque per-request handles minted in the
        # listen path; the cache holds entries for `tts_audio_ttl_seconds`
        # (default 60 s). A miss could mean unknown token (operator
        # error) or expired entry (firmware took too long to fetch);
        # both surface as 404 since the firmware's recovery is identical
        # in either case.
        entry = await cache.get(token)
        if entry is None:
            return Response(
                content="audio token unknown or expired\n",
                status_code=404,
                media_type="text/plain",
            )
        # PCM is binary; Content-Type matches what the firmware sends
        # *to* the sidecar so the two halves of the audio link
        # symmetrically advertise their format.
        return Response(
            content=entry.pcm,
            media_type="audio/L16;rate=16000;channels=1",
            headers={
                "X-Audio-Provider": entry.provider,
                "X-Audio-Voice": entry.voice,
            },
        )

    @app.get("/v1/personas")
    async def personas_endpoint() -> dict[str, object]:
        # Lists every persona slug under ``settings.personas_dir`` plus
        # which one a request will use when no X-Persona-Name header is
        # set. Operator-facing — lets a curl-based introspector see
        # what's deployed without grepping the filesystem.
        #
        # `default_deployed` distinguishes a healthy install from a
        # misconfig where `settings.persona` names a slug that isn't
        # on disk: in that case the next no-header request would 500.
        # A caller can check this flag to fail fast rather than wait
        # for the first /v1/listen to surface the problem.
        deployed = list_personas(settings.personas_dir)
        return {
            "default": settings.persona,
            "default_deployed": settings.persona in deployed,
            "personas": deployed,
        }

    @app.post("/v1/listen", dependencies=[Depends(verify_bearer)])
    async def listen(request: Request) -> JSONResponse:
        request_id = request.headers.get("x-request-id") or str(uuid4())
        session_id = request.headers.get("x-session-id", "")
        content_type = request.headers.get("content-type", "")

        ct_err = _validate_content_type(content_type)
        if ct_err is not None:
            return _audio_failure(ct_err, request_id=request_id, session_id=session_id)

        declared_length = request.headers.get("content-length")
        if declared_length is not None:
            try:
                declared = int(declared_length)
            except ValueError:
                return _audio_failure(
                    ErrorCode.BAD_CONTENT_LENGTH,
                    request_id=request_id,
                    session_id=session_id,
                )
            if declared > _MAX_BODY_BYTES:
                return _audio_failure(
                    ErrorCode.AUDIO_TOO_LARGE,
                    request_id=request_id,
                    session_id=session_id,
                )

        body = await request.body()
        if not body:
            return _audio_failure(
                ErrorCode.AUDIO_EMPTY, request_id=request_id, session_id=session_id
            )
        if len(body) > _MAX_BODY_BYTES:
            return _audio_failure(
                ErrorCode.AUDIO_TOO_LARGE, request_id=request_id, session_id=session_id
            )
        if len(body) < _MIN_BODY_BYTES:
            return _audio_failure(
                ErrorCode.AUDIO_TOO_SMALL, request_id=request_id, session_id=session_id
            )

        session_status.mark_thinking(request_id=request_id, session_id=session_id)

        t0 = time.perf_counter()
        deadline = time.monotonic() + settings.total_timeout_seconds

        # `X-Persona-Name` lets a per-device firmware pick which
        # persona file to load. Empty / missing header falls back to
        # the sidecar's baked-in `settings.persona` so installs that
        # don't multiplex personas keep working unchanged.
        #
        # The header path and the fallback path map load failures
        # differently:
        #   - Header path: ValueError → 400 (client sent a bad slug),
        #                  FileNotFoundError → 404 (client named a
        #                  persona this sidecar hasn't been deployed
        #                  with).
        #   - Fallback path: any failure → 500 (the sidecar's default
        #                    is misconfigured; the client did nothing
        #                    wrong).
        requested_persona = request.headers.get("x-persona-name", "").strip()
        # `persona_name` is the slug that goes into the SessionStore
        # key so conversation history is partitioned per-persona.
        # `persona` (set inside the branches) is the loaded markdown
        # prompt the LLM sees. Track them both — the slug travels with
        # the key; the prompt travels with the call.
        persona_name = requested_persona or settings.persona
        if requested_persona:
            try:
                persona = load_persona(requested_persona, settings.personas_dir)
            except ValueError as exc:
                _LOG.warning(
                    "persona name rejected",
                    extra={
                        "request_id": request_id,
                        "session_id": session_id,
                        "requested_persona": requested_persona,
                        "reason": str(exc),
                        "status": 400,
                    },
                )
                return _failure(
                    ErrorCode.PERSONA_NAME_INVALID,
                    request_id=request_id,
                    session_id=session_id,
                    status=400,
                    session_status=session_status,
                    extra={"requested_persona": requested_persona},
                )
            except FileNotFoundError:
                _LOG.warning(
                    "requested persona not deployed",
                    extra={
                        "request_id": request_id,
                        "session_id": session_id,
                        "requested_persona": requested_persona,
                        "status": 404,
                    },
                )
                return _failure(
                    ErrorCode.PERSONA_MISSING,
                    request_id=request_id,
                    session_id=session_id,
                    status=404,
                    session_status=session_status,
                    extra={"persona": requested_persona},
                )
        else:
            try:
                persona = load_persona(settings.persona, settings.personas_dir)
            except (ValueError, FileNotFoundError):
                # Both failure modes here are sidecar-side misconfig
                # (`settings.persona` empty or pointing at a missing
                # file). The client didn't supply anything to blame.
                _LOG.exception(
                    "default persona load failed",
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

        history = store.get_history(session_id, persona_name)
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
            persona_name,
            Turn(user=transcript, assistant=reply.full, emotion=emotion),
        )

        # Synthesise the *short* reply (toast-band-sized) rather than
        # the longer `reply.full`. The firmware plays what the avatar
        # would "say" — `full` is more like an internal monologue
        # carried for session memory only.
        #
        # On any TTS failure we degrade gracefully: text + emotion
        # still ship, `audio_url` is null, the firmware skips the
        # audio fetch and the toast band shows the reply as it does
        # today. The TTSError is logged with stage so an operator
        # can diagnose without taking down the reply path.
        audio_url: str | None = None
        if tts is not None:
            t_tts0 = time.perf_counter()
            try:
                result = await tts.synthesize(short)
                token = await cache.put(
                    result.pcm,
                    provider=result.provider,
                    voice=result.voice,
                )
                audio_url = f"/v1/audio/{token}"
                tts_ms = int((time.perf_counter() - t_tts0) * 1000)
                _LOG.info(
                    "listen.tts.ok",
                    extra={
                        "request_id": request_id,
                        "session_id": session_id,
                        "tts_provider": result.provider,
                        "tts_voice": result.voice,
                        "tts_ms": tts_ms,
                        "audio_duration_s": result.duration_seconds,
                    },
                )
            except TTSError as exc:
                _LOG.warning(
                    "listen.tts.fail",
                    extra={
                        "request_id": request_id,
                        "session_id": session_id,
                        "tts_stage": exc.stage,
                        "tts_detail": exc.detail,
                    },
                )
            except Exception:
                # Catch-all for unexpected provider blowups (httpx
                # client errors, dependency import failures, etc.).
                # Logged with stack but still degrades to no-audio
                # so a buggy provider doesn't take down /v1/listen.
                _LOG.exception(
                    "listen.tts.unexpected",
                    extra={"request_id": request_id, "session_id": session_id},
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
                "audio_url": audio_url,
                "status": 200,
            },
        )

        return JSONResponse({"text": short, "emotion": emotion.value, "audio_url": audio_url})

    return app
