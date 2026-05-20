import asyncio
import logging
import time
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager, suppress
from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from .llm import Emotion

if TYPE_CHECKING:
    from fastapi import FastAPI

_LOG = logging.getLogger("stackchan_sidecar")
_SWEEP_INTERVAL_SECONDS = 60.0


@dataclass(frozen=True)
class Turn:
    user: str
    assistant: str
    emotion: Emotion


@dataclass
class SessionEntry:
    last_seen: float
    turns: list[Turn] = field(default_factory=list)


class SessionStore:
    """In-memory turn history keyed by ``(session_id, persona)``.

    Persona is part of the key so a device that switches personas
    mid-deployment doesn't carry context from the old voice into the
    new one. The same session_id + different persona is treated as
    two independent conversations; a future tool that swaps persona
    mid-session gets a clean slate for the new voice while leaving
    the old voice's history intact for a possible revert.
    """

    def __init__(self, *, max_turns: int = 8, ttl_seconds: float = 600.0) -> None:
        self._max_turns = max_turns
        self._ttl_seconds = ttl_seconds
        self._entries: dict[tuple[str, str], SessionEntry] = {}

    def get_history(self, session_id: str, persona: str) -> list[Turn]:
        # Both halves of the key must be non-empty for the partition
        # to be meaningful. Today's app.py callers can't produce an
        # empty `persona` (load_persona rejects the empty slug before
        # we get here), but a future internal caller bypassing app.py
        # would silently merge everything into one bucket; the
        # explicit guard makes that fail closed.
        if not session_id or not persona:
            return []
        entry = self._entries.get((session_id, persona))
        if entry is None:
            return []
        entry.last_seen = time.monotonic()
        return list(entry.turns)

    def record(self, session_id: str, persona: str, turn: Turn) -> None:
        if not session_id or not persona:
            return
        key = (session_id, persona)
        entry = self._entries.get(key)
        if entry is None:
            entry = SessionEntry(last_seen=time.monotonic())
            self._entries[key] = entry
        entry.turns.append(turn)
        while len(entry.turns) > self._max_turns:
            entry.turns.pop(0)
        entry.last_seen = time.monotonic()

    def sweep(self) -> int:
        now = time.monotonic()
        expired = [key for key, e in self._entries.items() if now - e.last_seen > self._ttl_seconds]
        for key in expired:
            del self._entries[key]
        return len(expired)


async def _sweeper_loop(store: SessionStore) -> None:
    while True:
        try:
            await asyncio.sleep(_SWEEP_INTERVAL_SECONDS)
            removed = store.sweep()
            if removed:
                _LOG.info("session_store.sweep", extra={"removed": removed})
        except asyncio.CancelledError:
            raise
        except Exception:
            _LOG.exception("session_store.sweep_failed")


@asynccontextmanager
async def session_store_lifespan(app: "FastAPI", store: SessionStore) -> AsyncIterator[None]:
    _ = app
    task = asyncio.create_task(_sweeper_loop(store))
    try:
        yield
    finally:
        task.cancel()
        with suppress(asyncio.CancelledError):
            await task
