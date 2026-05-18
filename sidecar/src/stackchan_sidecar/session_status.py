"""Voice-agent state snapshot consumed by the 3D companion.

The sidecar marks `thinking` on `/v1/listen` entry and `idle` on completion
or failure. Subscribers (SSE clients) receive the current snapshot on
connect and every subsequent state change. The holder is intentionally
small: a dataclass + an `asyncio.Event` that flips on change so a single
broadcast wakes every subscriber.
"""

from __future__ import annotations

import asyncio
import time
from dataclasses import asdict, dataclass, field
from typing import Literal

State = Literal["idle", "thinking"]


@dataclass(frozen=True)
class Turn:
    """The latest completed listen→reply round-trip."""

    transcript: str
    reply_short: str
    emotion: str
    completed_at: float  # seconds since epoch; useful for subtitle TTL


@dataclass(frozen=True)
class Snapshot:
    state: State
    request_id: str | None = None
    session_id: str | None = None
    last_turn: Turn | None = None
    error: str | None = None
    updated_at: float = field(default_factory=time.time)


class SessionStatus:
    """Mutable holder with a broadcast change-event.

    Read-from-anywhere via [`get`][SessionStatus.get]; writes happen only
    from the `/v1/listen` handler. SSE subscribers `await changed()` then
    re-read.
    """

    def __init__(self) -> None:
        self._snapshot = Snapshot(state="idle")
        self._event = asyncio.Event()

    def get(self) -> Snapshot:
        return self._snapshot

    async def changed(self) -> None:
        """Block until the next state change. Each subscriber awaits its own
        view of the change; we recreate the event on every set so concurrent
        subscribers all wake exactly once per transition."""
        await self._event.wait()

    def _broadcast(self) -> None:
        self._event.set()
        self._event = asyncio.Event()

    def mark_thinking(self, *, request_id: str, session_id: str) -> None:
        self._snapshot = Snapshot(
            state="thinking",
            request_id=request_id,
            session_id=session_id,
            last_turn=self._snapshot.last_turn,
        )
        self._broadcast()

    def mark_done(
        self,
        *,
        request_id: str,
        session_id: str,
        transcript: str,
        reply_short: str,
        emotion: str,
    ) -> None:
        self._snapshot = Snapshot(
            state="idle",
            request_id=request_id,
            session_id=session_id,
            last_turn=Turn(
                transcript=transcript,
                reply_short=reply_short,
                emotion=emotion,
                completed_at=time.time(),
            ),
        )
        self._broadcast()

    def mark_failed(self, *, request_id: str, session_id: str, error: str) -> None:
        self._snapshot = Snapshot(
            state="idle",
            request_id=request_id,
            session_id=session_id,
            last_turn=self._snapshot.last_turn,
            error=error,
        )
        self._broadcast()


def snapshot_to_dict(s: Snapshot) -> dict[str, object]:
    """Serialize for the SSE wire (drop None turn cleanly)."""
    raw = asdict(s)
    if raw.get("last_turn") is None:
        raw.pop("last_turn", None)
    if raw.get("error") is None:
        raw.pop("error", None)
    return raw
