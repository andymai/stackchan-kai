import asyncio
import time

import httpx
import pytest

from stackchan_sidecar.retry import StageDeadlineError, is_transient, retry_with_timeout


def _deadline(seconds: float) -> float:
    return time.monotonic() + seconds


async def _ok() -> str:
    return "ok"


async def test_succeeds_first_attempt() -> None:
    result = await retry_with_timeout(
        _ok,
        max_attempts=3,
        per_attempt_timeout=1.0,
        deadline=_deadline(5.0),
        initial_backoff=0.0,
        label="t",
    )
    assert result == "ok"


async def test_retries_until_success() -> None:
    calls = {"n": 0}

    async def call() -> str:
        calls["n"] += 1
        if calls["n"] < 3:
            raise httpx.ConnectError("boom")
        return "ok"

    result = await retry_with_timeout(
        call,
        max_attempts=3,
        per_attempt_timeout=1.0,
        deadline=_deadline(5.0),
        initial_backoff=0.0,
        label="t",
    )
    assert result == "ok"
    assert calls["n"] == 3


async def test_does_not_retry_non_transient() -> None:
    calls = {"n": 0}

    async def call() -> str:
        calls["n"] += 1
        raise ValueError("permanent")

    with pytest.raises(ValueError):
        await retry_with_timeout(
            call,
            max_attempts=3,
            per_attempt_timeout=1.0,
            deadline=_deadline(5.0),
            initial_backoff=0.0,
            label="t",
        )
    assert calls["n"] == 1


async def test_exhausts_and_raises_last_transient() -> None:
    calls = {"n": 0}

    async def call() -> str:
        calls["n"] += 1
        raise httpx.ConnectError("always")

    with pytest.raises(httpx.ConnectError):
        await retry_with_timeout(
            call,
            max_attempts=2,
            per_attempt_timeout=1.0,
            deadline=_deadline(5.0),
            initial_backoff=0.0,
            label="t",
        )
    assert calls["n"] == 2


async def test_per_attempt_timeout_fires() -> None:
    async def call() -> str:
        await asyncio.sleep(10)
        return "never"

    with pytest.raises(TimeoutError):
        await retry_with_timeout(
            call,
            max_attempts=1,
            per_attempt_timeout=0.05,
            deadline=_deadline(5.0),
            initial_backoff=0.0,
            label="t",
        )


async def test_global_deadline_caps_attempts() -> None:
    calls = {"n": 0}

    async def call() -> str:
        calls["n"] += 1
        raise httpx.ConnectError("boom")

    with pytest.raises((httpx.ConnectError, StageDeadlineError)):
        await retry_with_timeout(
            call,
            max_attempts=10,
            per_attempt_timeout=0.01,
            deadline=_deadline(0.05),
            initial_backoff=0.1,
            label="t",
        )
    assert calls["n"] < 10


async def test_stage_deadline_when_past_deadline() -> None:
    with pytest.raises(StageDeadlineError):
        await retry_with_timeout(
            _ok,
            max_attempts=3,
            per_attempt_timeout=1.0,
            deadline=time.monotonic() - 1.0,
            initial_backoff=0.0,
            label="t",
        )


async def test_max_attempts_rejects_zero() -> None:
    with pytest.raises(ValueError, match="max_attempts"):
        await retry_with_timeout(
            _ok,
            max_attempts=0,
            per_attempt_timeout=1.0,
            deadline=_deadline(5.0),
            initial_backoff=0.0,
            label="t",
        )


def test_is_transient_httpx_5xx() -> None:
    req = httpx.Request("POST", "http://x")
    resp = httpx.Response(503, request=req)
    err = httpx.HTTPStatusError("boom", request=req, response=resp)
    assert is_transient(err) is True


def test_is_transient_httpx_4xx_false() -> None:
    req = httpx.Request("POST", "http://x")
    resp = httpx.Response(400, request=req)
    err = httpx.HTTPStatusError("boom", request=req, response=resp)
    assert is_transient(err) is False


def test_is_transient_anthropic_overloaded() -> None:
    import anthropic

    req = httpx.Request("POST", "http://x")
    resp = httpx.Response(529, request=req)
    err = anthropic.APIStatusError("overloaded", response=resp, body=None)
    assert is_transient(err) is True


def test_is_transient_openai_5xx() -> None:
    import openai

    req = httpx.Request("POST", "http://x")
    resp = httpx.Response(503, request=req)
    err = openai.APIStatusError("server", response=resp, body=None)
    assert is_transient(err) is True


def test_is_transient_value_error_false() -> None:
    assert is_transient(ValueError("nope")) is False


def test_is_transient_timeout_error_true() -> None:
    assert is_transient(TimeoutError("slow")) is True
