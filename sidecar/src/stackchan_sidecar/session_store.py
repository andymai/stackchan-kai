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
    def __init__(self, *, max_turns: int = 8, ttl_seconds: float = 600.0) -> None:
        self._max_turns = max_turns
        self._ttl_seconds = ttl_seconds
        self._entries: dict[str, SessionEntry] = {}

    def get_history(self, session_id: str) -> list[Turn]:
        if not session_id:
            return []
        entry = self._entries.get(session_id)
        if entry is None:
            return []
        entry.last_seen = time.monotonic()
        return list(entry.turns)

    def record(self, session_id: str, turn: Turn) -> None:
        if not session_id:
            return
        entry = self._entries.get(session_id)
        if entry is None:
            entry = SessionEntry(last_seen=time.monotonic())
            self._entries[session_id] = entry
        entry.turns.append(turn)
        while len(entry.turns) > self._max_turns:
            entry.turns.pop(0)
        entry.last_seen = time.monotonic()

    def sweep(self) -> int:
        now = time.monotonic()
        expired = [sid for sid, e in self._entries.items() if now - e.last_seen > self._ttl_seconds]
        for sid in expired:
            del self._entries[sid]
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
