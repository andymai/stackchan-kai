"""Provider-agnostic TTS protocol. All providers return 16 kHz mono
s16 LE PCM so the firmware playback path can consume the bytes
directly."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol, runtime_checkable

# Matches the firmware's `AUDIO_TX_QUEUE` expectation
# (see `crates/stackchan-firmware/src/audio.rs`).
PCM_SAMPLE_RATE_HZ = 16_000
PCM_CHANNELS = 1
PCM_SAMPLE_WIDTH_BYTES = 2


@dataclass(frozen=True)
class TTSResult:
    pcm: bytes
    provider: str
    voice: str

    @property
    def duration_seconds(self) -> float:
        samples = len(self.pcm) // PCM_SAMPLE_WIDTH_BYTES
        return samples / PCM_SAMPLE_RATE_HZ


@runtime_checkable
class TTSProvider(Protocol):
    """Async synthesis. CPU-bound engines bridge through
    ``asyncio.to_thread``; network providers ``await`` natively.
    Failures raise [`TTSError`]; ``/v1/listen`` catches and falls
    back to ``audio_url: null``."""

    name: str

    async def synthesize(self, text: str) -> TTSResult: ...
