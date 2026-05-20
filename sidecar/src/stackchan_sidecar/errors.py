"""Wire-level error envelope returned to the kai firmware.

Successful replies are `{"text", "emotion"}` and the firmware ignores
unknown keys, so failure replies use the same envelope plus an
`error: {code, stage}` member. The firmware drops non-2xx responses
on the floor (see `crates/stackchan-firmware/src/agent_sidecar.rs`),
so graceful failures (provider timeout, parse error) return 200 with
this envelope to land a meaningful toast on the avatar. Validation
failures stay as 4xx — kai never sees the body, the envelope is for
operator logs and curl smoke tests.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum

from .llm import Emotion, sanitize_short


class Stage(StrEnum):
    AUDIO = "audio"
    STT = "stt"
    LLM = "llm"
    SYSTEM = "system"


class ErrorCode(StrEnum):
    BAD_CONTENT_TYPE = "bad_content_type"
    BAD_CONTENT_LENGTH = "bad_content_length"
    AUDIO_RATE_UNSUPPORTED = "audio_rate_unsupported"
    AUDIO_CHANNELS_UNSUPPORTED = "audio_channels_unsupported"
    AUDIO_EMPTY = "audio_empty"
    AUDIO_TOO_SMALL = "audio_too_small"
    AUDIO_TOO_LARGE = "audio_too_large"
    STT_TIMEOUT = "stt_timeout"
    STT_FAILED = "stt_failed"
    LLM_TIMEOUT = "llm_timeout"
    LLM_FAILED = "llm_failed"
    LLM_PARSE_FAILED = "llm_parse_failed"
    PERSONA_MISSING = "persona_missing"
    PERSONA_NAME_INVALID = "persona_name_invalid"
    INTERNAL = "internal"


@dataclass(frozen=True)
class FailureKind:
    code: ErrorCode
    stage: Stage
    fallback_text: str
    emotion: Emotion = Emotion.SAD


_FAILURES: dict[ErrorCode, FailureKind] = {
    ErrorCode.BAD_CONTENT_TYPE: FailureKind(
        ErrorCode.BAD_CONTENT_TYPE, Stage.AUDIO, "wrong audio format", Emotion.NEUTRAL
    ),
    ErrorCode.BAD_CONTENT_LENGTH: FailureKind(
        ErrorCode.BAD_CONTENT_LENGTH, Stage.AUDIO, "bad content-length", Emotion.NEUTRAL
    ),
    ErrorCode.AUDIO_RATE_UNSUPPORTED: FailureKind(
        ErrorCode.AUDIO_RATE_UNSUPPORTED, Stage.AUDIO, "need 16kHz audio", Emotion.NEUTRAL
    ),
    ErrorCode.AUDIO_CHANNELS_UNSUPPORTED: FailureKind(
        ErrorCode.AUDIO_CHANNELS_UNSUPPORTED, Stage.AUDIO, "need mono audio", Emotion.NEUTRAL
    ),
    ErrorCode.AUDIO_EMPTY: FailureKind(
        ErrorCode.AUDIO_EMPTY, Stage.AUDIO, "no audio received", Emotion.NEUTRAL
    ),
    ErrorCode.AUDIO_TOO_SMALL: FailureKind(
        ErrorCode.AUDIO_TOO_SMALL, Stage.AUDIO, "audio too short", Emotion.NEUTRAL
    ),
    ErrorCode.AUDIO_TOO_LARGE: FailureKind(
        ErrorCode.AUDIO_TOO_LARGE, Stage.AUDIO, "audio too long", Emotion.NEUTRAL
    ),
    ErrorCode.STT_TIMEOUT: FailureKind(
        ErrorCode.STT_TIMEOUT, Stage.STT, "didn't catch that, try again"
    ),
    ErrorCode.STT_FAILED: FailureKind(
        ErrorCode.STT_FAILED, Stage.STT, "hearing's wonky, try again"
    ),
    ErrorCode.LLM_TIMEOUT: FailureKind(
        ErrorCode.LLM_TIMEOUT, Stage.LLM, "thinking too hard, try again"
    ),
    ErrorCode.LLM_FAILED: FailureKind(ErrorCode.LLM_FAILED, Stage.LLM, "brain hiccup, try again"),
    ErrorCode.LLM_PARSE_FAILED: FailureKind(
        ErrorCode.LLM_PARSE_FAILED, Stage.LLM, "got confused, try again"
    ),
    ErrorCode.PERSONA_MISSING: FailureKind(
        ErrorCode.PERSONA_MISSING, Stage.SYSTEM, "persona missing", Emotion.NEUTRAL
    ),
    ErrorCode.PERSONA_NAME_INVALID: FailureKind(
        ErrorCode.PERSONA_NAME_INVALID,
        Stage.SYSTEM,
        "invalid persona name",
        Emotion.NEUTRAL,
    ),
    ErrorCode.INTERNAL: FailureKind(
        ErrorCode.INTERNAL, Stage.SYSTEM, "internal error", Emotion.NEUTRAL
    ),
}


def failure_kind(code: ErrorCode) -> FailureKind:
    return _FAILURES[code]


def build_envelope(kind: FailureKind) -> dict[str, object]:
    text = sanitize_short(kind.fallback_text)
    return {
        "text": text,
        "emotion": kind.emotion.value,
        "error": {"code": kind.code.value, "stage": kind.stage.value},
    }


_HTTP_STATUS_BY_CODE: dict[ErrorCode, int] = {
    ErrorCode.BAD_CONTENT_TYPE: 415,
    ErrorCode.BAD_CONTENT_LENGTH: 400,
    ErrorCode.AUDIO_RATE_UNSUPPORTED: 415,
    ErrorCode.AUDIO_CHANNELS_UNSUPPORTED: 415,
    ErrorCode.AUDIO_EMPTY: 400,
    ErrorCode.AUDIO_TOO_SMALL: 400,
    ErrorCode.AUDIO_TOO_LARGE: 413,
}


def audio_validation_status(code: ErrorCode) -> int:
    return _HTTP_STATUS_BY_CODE[code]


__all__ = [
    "ErrorCode",
    "FailureKind",
    "Stage",
    "audio_validation_status",
    "build_envelope",
    "failure_kind",
]
