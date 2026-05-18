"""Behavioural tests for the SessionStatus state holder."""

from __future__ import annotations

import asyncio

import pytest

from stackchan_sidecar.session_status import SessionStatus, snapshot_to_dict


def test_initial_snapshot_is_idle() -> None:
    s = SessionStatus()
    snap = s.get()
    assert snap.state == "idle"
    assert snap.last_turn is None


def test_mark_thinking_updates_state() -> None:
    s = SessionStatus()
    s.mark_thinking(request_id="req1", session_id="sess1")
    snap = s.get()
    assert snap.state == "thinking"
    assert snap.request_id == "req1"
    assert snap.session_id == "sess1"


def test_mark_done_returns_to_idle_with_turn() -> None:
    s = SessionStatus()
    s.mark_thinking(request_id="req1", session_id="sess1")
    s.mark_done(
        request_id="req1",
        session_id="sess1",
        transcript="hello",
        reply_short="hi!",
        emotion="happy",
    )
    snap = s.get()
    assert snap.state == "idle"
    assert snap.last_turn is not None
    assert snap.last_turn.transcript == "hello"
    assert snap.last_turn.reply_short == "hi!"
    assert snap.last_turn.emotion == "happy"


def test_mark_failed_returns_to_idle_with_error() -> None:
    s = SessionStatus()
    s.mark_thinking(request_id="req1", session_id="sess1")
    s.mark_failed(request_id="req1", session_id="sess1", error="stt_timeout")
    snap = s.get()
    assert snap.state == "idle"
    assert snap.error == "stt_timeout"


def test_mark_failed_preserves_prior_turn() -> None:
    s = SessionStatus()
    s.mark_thinking(request_id="r0", session_id="s0")
    s.mark_done(
        request_id="r0",
        session_id="s0",
        transcript="first",
        reply_short="ok",
        emotion="neutral",
    )
    s.mark_thinking(request_id="r1", session_id="s0")
    s.mark_failed(request_id="r1", session_id="s0", error="llm_failed")
    snap = s.get()
    assert snap.error == "llm_failed"
    assert snap.last_turn is not None
    assert snap.last_turn.transcript == "first"


@pytest.mark.asyncio
async def test_changed_wakes_subscribers_on_transition() -> None:
    s = SessionStatus()
    received: list[str] = []

    async def waiter() -> None:
        await s.changed()
        received.append(s.get().state)

    task = asyncio.create_task(waiter())
    await asyncio.sleep(0)  # yield so waiter starts blocking
    s.mark_thinking(request_id="r1", session_id="s1")
    await asyncio.wait_for(task, timeout=0.5)
    assert received == ["thinking"]


@pytest.mark.asyncio
async def test_changed_wakes_multiple_subscribers() -> None:
    s = SessionStatus()
    seen: list[str] = []

    async def waiter(label: str) -> None:
        await s.changed()
        seen.append(label)

    a = asyncio.create_task(waiter("a"))
    b = asyncio.create_task(waiter("b"))
    await asyncio.sleep(0)
    s.mark_thinking(request_id="r", session_id="s")
    await asyncio.wait_for(asyncio.gather(a, b), timeout=0.5)
    assert sorted(seen) == ["a", "b"]


def test_snapshot_to_dict_drops_none_fields() -> None:
    s = SessionStatus()
    raw = snapshot_to_dict(s.get())
    assert raw["state"] == "idle"
    assert "last_turn" not in raw
    assert "error" not in raw

    s.mark_done(
        request_id="r",
        session_id="s",
        transcript="hi",
        reply_short="hi",
        emotion="happy",
    )
    raw = snapshot_to_dict(s.get())
    assert raw["last_turn"]["transcript"] == "hi"
    assert "error" not in raw
