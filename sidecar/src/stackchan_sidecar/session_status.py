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
from collections import OrderedDict
from dataclasses import asdict, dataclass, field
from typing import Literal

State = Literal["idle", "thinking"]

# Cap on the number of distinct sessions tracked simultaneously.
# In a deployment where firmware units rotate through unique
# session IDs (re-flash → new MAC-derived ID) the dict would
# otherwise grow without bound. LRU eviction past this cap drops
# the snapshot least-recently *updated* — which usually means a
# session that hasn't `mark_thinking`'d or `mark_done`'d in
# however long it took the cap to fill. 64 covers any reasonable
# fleet size with margin.
_MAX_TRACKED_SESSIONS = 64


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
    """Per-session-keyed snapshot holder with a broadcast change-event.

    Maintains one [`Snapshot`][Snapshot] per `session_id` so concurrent
    PTT captures from multiple firmware units don't clobber each other:
    unit B's `mark_thinking` no longer overwrites unit A's in-flight
    state, and unit A's `mark_done` writes only against its own
    session.

    Read via [`get`][SessionStatus.get]; without a `session_id`
    returns the most-recently-updated snapshot (matches the
    single-unit setup the companion app was built for). With an
    explicit `session_id` returns that unit's view. SSE subscribers
    await [`changed`][SessionStatus.changed] then re-read.
    """

    def __init__(self) -> None:
        # `OrderedDict` gives us LRU eviction in O(1) on each
        # update — `move_to_end` after every write keeps the most
        # recently updated session at the right end; `popitem(last
        # =False)` drops the least-recently updated when the cap
        # is exceeded.
        self._snapshots: OrderedDict[str, Snapshot] = OrderedDict()
        self._latest_session_id: str | None = None
        self._idle = Snapshot(state="idle")
        self._event = asyncio.Event()

    def get(self, session_id: str | None = None) -> Snapshot:
        """Return the snapshot for `session_id`, or the most-recently-
        updated snapshot when omitted. Falls back to the default-idle
        snapshot when nothing has been recorded yet — preserves the
        pre-multi-session shape where subscribers always saw *some*
        snapshot on connect."""
        if session_id is not None:
            return self._snapshots.get(session_id, self._idle)
        if self._latest_session_id is None:
            return self._idle
        return self._snapshots.get(self._latest_session_id, self._idle)

    def all(self) -> dict[str, Snapshot]:
        """Snapshot of every known session_id, for diagnostic surfaces
        that want a full picture across units."""
        return dict(self._snapshots)

    async def changed(self) -> None:
        """Block until any snapshot transitions. Each subscriber awaits
        its own view of the change; the event is recreated on every set
        so concurrent subscribers all wake exactly once per transition."""
        await self._event.wait()

    def _broadcast(self) -> None:
        self._event.set()
        self._event = asyncio.Event()

    def _put(self, session_id: str, snapshot: Snapshot) -> None:
        self._snapshots[session_id] = snapshot
        self._snapshots.move_to_end(session_id)
        while len(self._snapshots) > _MAX_TRACKED_SESSIONS:
            self._snapshots.popitem(last=False)
        self._latest_session_id = session_id
        self._broadcast()

    def mark_thinking(self, *, request_id: str, session_id: str) -> None:
        prior = self._snapshots.get(session_id, self._idle)
        self._put(
            session_id,
            Snapshot(
                state="thinking",
                request_id=request_id,
                session_id=session_id,
                last_turn=prior.last_turn,
            ),
        )

    def mark_done(
        self,
        *,
        request_id: str,
        session_id: str,
        transcript: str,
        reply_short: str,
        emotion: str,
    ) -> None:
        self._put(
            session_id,
            Snapshot(
                state="idle",
                request_id=request_id,
                session_id=session_id,
                last_turn=Turn(
                    transcript=transcript,
                    reply_short=reply_short,
                    emotion=emotion,
                    completed_at=time.time(),
                ),
            ),
        )

    def mark_failed(self, *, request_id: str, session_id: str, error: str) -> None:
        prior = self._snapshots.get(session_id, self._idle)
        self._put(
            session_id,
            Snapshot(
                state="idle",
                request_id=request_id,
                session_id=session_id,
                last_turn=prior.last_turn,
                error=error,
            ),
        )


def snapshot_to_dict(s: Snapshot) -> dict[str, object]:
    """Serialize for the SSE wire (drop None turn cleanly)."""
    raw = asdict(s)
    if raw.get("last_turn") is None:
        raw.pop("last_turn", None)
    if raw.get("error") is None:
        raw.pop("error", None)
    return raw
