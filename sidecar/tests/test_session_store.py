import time

import pytest

from stackchan_sidecar.llm import Emotion
from stackchan_sidecar.session_store import SessionStore, Turn


def _turn(user: str = "hi", assistant: str = "hello") -> Turn:
    return Turn(user=user, assistant=assistant, emotion=Emotion.HAPPY)


def test_get_history_empty_when_unknown() -> None:
    store = SessionStore()
    assert store.get_history("sid-a") == []


def test_record_then_get_history() -> None:
    store = SessionStore()
    store.record("sid-a", _turn("u1", "a1"))
    store.record("sid-a", _turn("u2", "a2"))
    history = store.get_history("sid-a")
    assert len(history) == 2
    assert history[0].user == "u1"
    assert history[1].user == "u2"


def test_get_history_is_isolated_per_session() -> None:
    store = SessionStore()
    store.record("sid-a", _turn("a-only", "a-resp"))
    store.record("sid-b", _turn("b-only", "b-resp"))
    history_a = store.get_history("sid-a")
    history_b = store.get_history("sid-b")
    assert len(history_a) == 1 and history_a[0].user == "a-only"
    assert len(history_b) == 1 and history_b[0].user == "b-only"


def test_get_history_returns_copy() -> None:
    store = SessionStore()
    store.record("sid", _turn("u1", "a1"))
    history = store.get_history("sid")
    history.append(_turn("forged", "forged"))
    assert len(store.get_history("sid")) == 1


def test_max_turns_eviction() -> None:
    store = SessionStore(max_turns=3)
    for i in range(5):
        store.record("sid", _turn(f"u{i}", f"a{i}"))
    history = store.get_history("sid")
    assert len(history) == 3
    assert history[0].user == "u2"
    assert history[-1].user == "u4"


def test_sweep_evicts_expired(monkeypatch: pytest.MonkeyPatch) -> None:
    now = [1000.0]

    def fake_monotonic() -> float:
        return now[0]

    monkeypatch.setattr(time, "monotonic", fake_monotonic)
    store = SessionStore(ttl_seconds=60.0)
    store.record("sid-old", _turn())
    now[0] += 120.0
    store.record("sid-new", _turn())
    removed = store.sweep()
    assert removed == 1
    assert store.get_history("sid-old") == []
    assert len(store.get_history("sid-new")) == 1


def test_sweep_returns_zero_when_nothing_expired(monkeypatch: pytest.MonkeyPatch) -> None:
    now = [1000.0]
    monkeypatch.setattr(time, "monotonic", lambda: now[0])
    store = SessionStore(ttl_seconds=60.0)
    store.record("sid", _turn())
    now[0] += 30.0
    assert store.sweep() == 0


def test_get_history_touches_last_seen(monkeypatch: pytest.MonkeyPatch) -> None:
    now = [1000.0]
    monkeypatch.setattr(time, "monotonic", lambda: now[0])
    store = SessionStore(ttl_seconds=60.0)
    store.record("sid", _turn())
    now[0] += 50.0
    store.get_history("sid")
    now[0] += 30.0
    assert store.sweep() == 0
    assert len(store.get_history("sid")) == 1


def test_empty_session_id_is_noop() -> None:
    store = SessionStore()
    store.record("", _turn())
    assert store.get_history("") == []
