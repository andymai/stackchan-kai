"""Unit tests for the in-memory audio cache."""

from __future__ import annotations

import asyncio
import time

import pytest

from stackchan_sidecar.audio_cache import AudioCache


async def _put(cache: AudioCache, n: int = 1) -> list[str]:
    tokens: list[str] = []
    for i in range(n):
        tokens.append(await cache.put(b"\x00\x00" * 10, provider="test", voice=f"v{i}"))
    return tokens


@pytest.mark.asyncio
async def test_put_then_get_round_trip() -> None:
    cache = AudioCache()
    token = await cache.put(b"\x01\x02\x03\x04", provider="espeak_ng", voice="en")
    entry = await cache.get(token)
    assert entry is not None
    assert entry.pcm == b"\x01\x02\x03\x04"
    assert entry.provider == "espeak_ng"
    assert entry.voice == "en"


@pytest.mark.asyncio
async def test_get_unknown_token_returns_none() -> None:
    cache = AudioCache()
    assert await cache.get("not-a-real-token") is None


@pytest.mark.asyncio
async def test_tokens_are_unique_per_put() -> None:
    cache = AudioCache(capacity=100)
    tokens = await _put(cache, n=20)
    assert len(set(tokens)) == 20


@pytest.mark.asyncio
async def test_capacity_evicts_oldest() -> None:
    cache = AudioCache(capacity=3)
    tokens = await _put(cache, n=5)
    # First two evicted; last three retained.
    assert await cache.get(tokens[0]) is None
    assert await cache.get(tokens[1]) is None
    assert await cache.get(tokens[2]) is not None
    assert await cache.get(tokens[3]) is not None
    assert await cache.get(tokens[4]) is not None
    assert cache._len_for_test() == 3


@pytest.mark.asyncio
async def test_ttl_expires_entries(monkeypatch: pytest.MonkeyPatch) -> None:
    now = [1000.0]
    monkeypatch.setattr(time, "monotonic", lambda: now[0])
    cache = AudioCache(ttl_seconds=10.0)
    token = await cache.put(b"\x00\x00", provider="test", voice="x")
    now[0] += 5.0
    assert await cache.get(token) is not None
    now[0] += 10.0  # +15s total — past TTL
    assert await cache.get(token) is None


@pytest.mark.asyncio
async def test_ttl_evicted_on_subsequent_put(monkeypatch: pytest.MonkeyPatch) -> None:
    # Lazy sweep: expired entries get removed on the next put() and
    # don't count against capacity. Pins the invariant so a slow
    # operator who lets an old token age out doesn't lose capacity.
    now = [1000.0]
    monkeypatch.setattr(time, "monotonic", lambda: now[0])
    cache = AudioCache(ttl_seconds=5.0, capacity=2)
    await cache.put(b"a", provider="t", voice="v")
    await cache.put(b"b", provider="t", voice="v")
    assert cache._len_for_test() == 2
    now[0] += 10.0  # both expired
    await cache.put(b"c", provider="t", voice="v")
    # Lazy sweep ran on the put — old two are gone, only `c` remains.
    assert cache._len_for_test() == 1


def test_rejects_zero_ttl() -> None:
    with pytest.raises(ValueError, match="ttl"):
        AudioCache(ttl_seconds=0)


def test_rejects_zero_capacity() -> None:
    with pytest.raises(ValueError, match="capacity"):
        AudioCache(capacity=0)


@pytest.mark.asyncio
async def test_concurrent_puts_are_serialised() -> None:
    # Hammer the cache from many concurrent tasks; the lock should
    # serialise without losing entries or producing duplicate tokens.
    cache = AudioCache(capacity=100)
    tokens = await asyncio.gather(*[_put(cache, n=1) for _ in range(20)])
    flat = [t for sublist in tokens for t in sublist]
    assert len(set(flat)) == 20
