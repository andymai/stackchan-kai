"""Retry + per-attempt timeout helper for upstream provider calls.

The pipeline runs one STT call followed by one LLM call. Each upstream
can fail in transient ways the user wants masked: Anthropic 529,
OpenAI 5xx, Deepgram 5xx, an Ollama connection blip, a faster-whisper
subprocess hiccup. This helper wraps a single async callable with a
per-attempt `asyncio.wait_for` and re-tries up to `max_attempts` total,
respecting an overall `deadline` (monotonic seconds) shared across
both stages so a slow STT can't starve the LLM call.
"""

from __future__ import annotations

import asyncio
import logging
import time
from collections.abc import Awaitable, Callable

import anthropic
import httpx
import openai

_LOG = logging.getLogger("stackchan_sidecar")

_RETRYABLE_STATUS = frozenset({408, 425, 429, 500, 502, 503, 504, 529})


class StageDeadlineError(Exception):
    """No attempt could fit inside the remaining shared deadline."""


def is_transient(exc: BaseException) -> bool:
    if isinstance(exc, TimeoutError):
        return True
    if isinstance(exc, httpx.TimeoutException):
        return True
    if isinstance(exc, httpx.HTTPStatusError):
        return exc.response.status_code in _RETRYABLE_STATUS
    if isinstance(exc, httpx.HTTPError):
        return True
    if isinstance(exc, anthropic.APIConnectionError | anthropic.APITimeoutError):
        return True
    if isinstance(exc, anthropic.APIStatusError):
        return exc.status_code in _RETRYABLE_STATUS
    if isinstance(exc, openai.APIConnectionError | openai.APITimeoutError):
        return True
    if isinstance(exc, openai.APIStatusError):
        return exc.status_code in _RETRYABLE_STATUS
    return False


async def retry_with_timeout[T](
    call: Callable[[], Awaitable[T]],
    *,
    max_attempts: int,
    per_attempt_timeout: float,
    deadline: float,
    initial_backoff: float,
    label: str,
    is_transient_fn: Callable[[BaseException], bool] = is_transient,
) -> T:
    if max_attempts < 1:
        raise ValueError("max_attempts must be >= 1")

    backoff = initial_backoff
    last_exc: Exception | None = None
    for attempt in range(1, max_attempts + 1):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise StageDeadlineError(label) from last_exc
        attempt_timeout = min(per_attempt_timeout, remaining)
        try:
            return await asyncio.wait_for(call(), timeout=attempt_timeout)
        except Exception as exc:
            if not is_transient_fn(exc):
                raise
            last_exc = exc
            if attempt == max_attempts:
                raise
            sleep_for = min(backoff, max(0.0, deadline - time.monotonic()))
            _LOG.info(
                "%s.retry",
                label,
                extra={
                    "label": label,
                    "attempt": attempt,
                    "max_attempts": max_attempts,
                    "exc": type(exc).__name__,
                    "sleep": sleep_for,
                },
            )
            if sleep_for > 0:
                await asyncio.sleep(sleep_for)
            backoff *= 2

    raise RuntimeError("retry_with_timeout exited loop without return")


__all__ = ["StageDeadlineError", "is_transient", "retry_with_timeout"]
