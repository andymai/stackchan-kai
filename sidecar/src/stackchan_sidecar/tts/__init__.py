"""Text-to-speech providers for the sidecar voice-agent reply path.

Every provider emits **raw 16 kHz mono s16 little-endian PCM** so the
firmware's `AUDIO_TX_QUEUE` can consume it without any transcode. The
wire contract between sidecar and firmware is:

1. ``POST /v1/listen`` LLM-reply path includes ``"audio_url":
   "/v1/audio/<token>"`` when synthesis succeeded, or
   ``"audio_url": null`` when it failed (graceful degradation —
   text + emotion still ship).
2. Firmware ``GET /v1/audio/<token>`` returns the cached PCM as
   ``Transfer-Encoding: chunked`` so playback can start on the
   first chunk.

Token lifetime is bounded by the in-memory audio cache (TTL +
capacity, see ``audio_cache.py``); the firmware is expected to fetch
within a few seconds.

This package's public surface is the [`TTSProvider`] protocol and the
three concrete providers ([`EspeakProvider`], [`PiperProvider`],
[`ElevenLabsProvider`]). [`TTSError`] is the canonical failure type;
callers catch it and fall back to ``audio_url: null``.
"""

from __future__ import annotations

from .errors import TTSError
from .espeak import EspeakProvider
from .protocol import TTSProvider, TTSResult

__all__ = [
    "EspeakProvider",
    "TTSError",
    "TTSProvider",
    "TTSResult",
]
