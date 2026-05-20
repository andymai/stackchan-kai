"""Text-to-speech providers for the sidecar voice-agent reply path.

Wire contract:

1. ``POST /v1/listen`` includes ``"audio_url": "/v1/audio/<token>"``
   on synthesis success or ``"audio_url": null`` on failure (text +
   emotion still ship).
2. Firmware ``GET /v1/audio/<token>`` returns the cached PCM as one
   ``Content-Length``-framed response body, 16 kHz mono s16 LE.
   Replies are short enough (≲ a few hundred KiB) that streaming
   doesn't materially change first-sample latency on the LAN.

Token lifetime is bounded by the [`AudioCache`] (TTL + capacity).
Public surface: the [`TTSProvider`] protocol and concrete providers
under this package. [`TTSError`] is the canonical failure type.
"""

from __future__ import annotations

from .elevenlabs import ElevenLabsProvider
from .errors import TTSError
from .espeak import EspeakProvider
from .piper import PiperProvider
from .protocol import TTSProvider, TTSResult

__all__ = [
    "ElevenLabsProvider",
    "EspeakProvider",
    "PiperProvider",
    "TTSError",
    "TTSProvider",
    "TTSResult",
]
