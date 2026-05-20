import time

import pytest

from stackchan_sidecar.llm import Emotion
from stackchan_sidecar.session_store import SessionStore, Turn


def _turn(user: str = "hi", assistant: str = "hello") -> Turn:
    return Turn(user=user, assistant=assistant, emotion=Emotion.HAPPY)


# Default persona slug used when the test isn't exercising the
# per-persona partition. Pulled out as a constant so a future rename
# (e.g. "default" → "") only changes one line.
_P = "stack-chan"


def test_get_history_empty_when_unknown() -> None:
    store = SessionStore()
    assert store.get_history("sid-a", _P) == []


def test_record_then_get_history() -> None:
    store = SessionStore()
    store.record("sid-a", _P, _turn("u1", "a1"))
    store.record("sid-a", _P, _turn("u2", "a2"))
    history = store.get_history("sid-a", _P)
    assert len(history) == 2
    assert history[0].user == "u1"
    assert history[1].user == "u2"


def test_get_history_is_isolated_per_session() -> None:
    store = SessionStore()
    store.record("sid-a", _P, _turn("a-only", "a-resp"))
    store.record("sid-b", _P, _turn("b-only", "b-resp"))
    history_a = store.get_history("sid-a", _P)
    history_b = store.get_history("sid-b", _P)
    assert len(history_a) == 1 and history_a[0].user == "a-only"
    assert len(history_b) == 1 and history_b[0].user == "b-only"


def test_get_history_returns_copy() -> None:
    store = SessionStore()
    store.record("sid", _P, _turn("u1", "a1"))
    history = store.get_history("sid", _P)
    history.append(_turn("forged", "forged"))
    assert len(store.get_history("sid", _P)) == 1


def test_max_turns_eviction() -> None:
    store = SessionStore(max_turns=3)
    for i in range(5):
        store.record("sid", _P, _turn(f"u{i}", f"a{i}"))
    history = store.get_history("sid", _P)
    assert len(history) == 3
    assert history[0].user == "u2"
    assert history[-1].user == "u4"


def test_sweep_evicts_expired(monkeypatch: pytest.MonkeyPatch) -> None:
    now = [1000.0]

    def fake_monotonic() -> float:
        return now[0]

    monkeypatch.setattr(time, "monotonic", fake_monotonic)
    store = SessionStore(ttl_seconds=60.0)
    store.record("sid-old", _P, _turn())
    now[0] += 120.0
    store.record("sid-new", _P, _turn())
    removed = store.sweep()
    assert removed == 1
    assert store.get_history("sid-old", _P) == []
    assert len(store.get_history("sid-new", _P)) == 1


def test_sweep_returns_zero_when_nothing_expired(monkeypatch: pytest.MonkeyPatch) -> None:
    now = [1000.0]
    monkeypatch.setattr(time, "monotonic", lambda: now[0])
    store = SessionStore(ttl_seconds=60.0)
    store.record("sid", _P, _turn())
    now[0] += 30.0
    assert store.sweep() == 0


def test_get_history_touches_last_seen(monkeypatch: pytest.MonkeyPatch) -> None:
    now = [1000.0]
    monkeypatch.setattr(time, "monotonic", lambda: now[0])
    store = SessionStore(ttl_seconds=60.0)
    store.record("sid", _P, _turn())
    now[0] += 50.0
    store.get_history("sid", _P)
    now[0] += 30.0
    assert store.sweep() == 0
    assert len(store.get_history("sid", _P)) == 1


def test_empty_session_id_is_noop() -> None:
    store = SessionStore()
    store.record("", _P, _turn())
    assert store.get_history("", _P) == []


def test_history_partitioned_per_persona() -> None:
    # Same session_id, two personas: the second voice doesn't inherit
    # the first voice's turns. This is the core invariant of the
    # per-persona partition.
    store = SessionStore()
    store.record("sid", "stack-chan", _turn("hi stack", "hello"))
    store.record("sid", "desk-buddy", _turn("hi buddy", "yo"))
    stack_history = store.get_history("sid", "stack-chan")
    buddy_history = store.get_history("sid", "desk-buddy")
    assert len(stack_history) == 1 and stack_history[0].user == "hi stack"
    assert len(buddy_history) == 1 and buddy_history[0].user == "hi buddy"


def test_persona_partition_survives_round_trip() -> None:
    # Switching personas mid-deployment and back returns the original
    # history — neither bucket overwrites the other.
    store = SessionStore()
    store.record("sid", "a", _turn("a1", "ra1"))
    store.record("sid", "a", _turn("a2", "ra2"))
    store.record("sid", "b", _turn("b1", "rb1"))
    store.record("sid", "a", _turn("a3", "ra3"))
    a_history = store.get_history("sid", "a")
    b_history = store.get_history("sid", "b")
    assert [t.user for t in a_history] == ["a1", "a2", "a3"]
    assert [t.user for t in b_history] == ["b1"]


def test_sweep_evicts_per_persona_bucket_independently(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # An idle persona on a still-active session times out on its own
    # clock — the partition's TTL is per-(session, persona), not
    # per-session. Without this, a chatty persona-A would keep
    # persona-B's stale history alive forever on the same device.
    now = [1000.0]
    monkeypatch.setattr(time, "monotonic", lambda: now[0])
    store = SessionStore(ttl_seconds=60.0)
    store.record("sid", "a", _turn("a-old", "ra"))
    now[0] += 120.0
    store.record("sid", "b", _turn("b-fresh", "rb"))
    assert store.sweep() == 1
    assert store.get_history("sid", "a") == []
    assert len(store.get_history("sid", "b")) == 1
