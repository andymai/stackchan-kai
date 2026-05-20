"""TTL-bounded in-memory cache for synthesised PCM bytes.

The sidecar's listen pipeline returns ``audio_url: "/v1/audio/<token>"``
referring to one entry of this cache; the firmware fetches the bytes
within seconds and the entry expires soon after. The cache is
single-use semantically (firmware doesn't replay), but we keep entries
for ``ttl_seconds`` so a redelivery / retry path works without
forcing the sidecar to re-synthesise.

Capacity is bounded so a misbehaving firmware (or a load test) can't
grow the cache without limit; the oldest entry evicts when capacity
is reached.

Concurrency: the cache is touched from the listen handler (writer) and
the audio handler (reader). An ``asyncio.Lock`` serialises mutations;
reads under the lock are fast (dict lookup).
"""

from __future__ import annotations

import asyncio
import logging
import secrets
import time
from collections import OrderedDict
from dataclasses import dataclass

_LOG = logging.getLogger("stackchan_sidecar.audio_cache")

# Default token length: 16 bytes of randomness as hex (32 hex chars).
# Long enough to be unguessable; short enough not to bloat URLs.
_TOKEN_BYTES = 16


@dataclass(frozen=True)
class AudioEntry:
    pcm: bytes
    provider: str
    voice: str
    stored_at: float


class AudioCache:
    def __init__(
        self,
        *,
        ttl_seconds: float = 60.0,
        capacity: int = 32,
    ) -> None:
        if ttl_seconds <= 0:
            raise ValueError(f"ttl_seconds must be positive; got {ttl_seconds}")
        if capacity <= 0:
            raise ValueError(f"capacity must be positive; got {capacity}")
        self._ttl_seconds = ttl_seconds
        self._capacity = capacity
        self._entries: OrderedDict[str, AudioEntry] = OrderedDict()
        self._lock = asyncio.Lock()

    async def put(self, pcm: bytes, *, provider: str, voice: str) -> str:
        """Store ``pcm`` and return an opaque token. Evicts the oldest
        entry when capacity is reached and the expired entries on every
        write (lazy sweep — keeps the API simple and the contention
        window small)."""
        token = secrets.token_hex(_TOKEN_BYTES)
        entry = AudioEntry(
            pcm=pcm,
            provider=provider,
            voice=voice,
            stored_at=time.monotonic(),
        )
        async with self._lock:
            self._evict_expired_locked()
            self._entries[token] = entry
            while len(self._entries) > self._capacity:
                evicted_token, _ = self._entries.popitem(last=False)
                _LOG.info("audio_cache.evict_capacity token=%s", evicted_token)
        return token

    async def get(self, token: str) -> AudioEntry | None:
        """Return the entry for ``token`` or ``None`` if it's missing
        or expired. Caller-friendly: a TTL miss is indistinguishable
        from a never-stored token, both surface as 404 at the route."""
        async with self._lock:
            self._evict_expired_locked()
            return self._entries.get(token)

    def _evict_expired_locked(self) -> None:
        now = time.monotonic()
        expired = [
            tok for tok, entry in self._entries.items() if now - entry.stored_at > self._ttl_seconds
        ]
        for tok in expired:
            del self._entries[tok]
            _LOG.debug("audio_cache.evict_ttl token=%s", tok)

    # Test surface — lets tests assert capacity / TTL behaviour without
    # touching the wire path. Not part of the public API.

    def _len_for_test(self) -> int:
        return len(self._entries)
