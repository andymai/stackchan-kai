"""Provider-agnostic TTS protocol.

All providers return [`TTSResult`] — raw PCM bytes plus the sample
rate and channel layout that produced them. The shared rate target
is 16 kHz mono s16 LE so the firmware playback path can consume the
bytes directly; providers whose engine emits something else
resample down to 16 kHz before returning.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol, runtime_checkable

# Target wire-rate. Matches the firmware's `AUDIO_TX_QUEUE` expectation
# (see `crates/stackchan-firmware/src/audio.rs`).
PCM_SAMPLE_RATE_HZ = 16_000
PCM_CHANNELS = 1
PCM_SAMPLE_WIDTH_BYTES = 2


@dataclass(frozen=True)
class TTSResult:
    """Output of one synthesis call.

    ``pcm`` is the raw 16 kHz mono s16 LE bytes the firmware streams
    into the AW88298 amp. ``provider`` and ``voice`` are surfaced in
    logs so an operator can tell which engine made the noise.
    """

    pcm: bytes
    provider: str
    voice: str

    @property
    def duration_seconds(self) -> float:
        samples = len(self.pcm) // PCM_SAMPLE_WIDTH_BYTES
        return samples / PCM_SAMPLE_RATE_HZ


@runtime_checkable
class TTSProvider(Protocol):
    """Async-friendly synthesis interface.

    Implementations are expected to ``await`` rather than block; CPU-bound
    engines (Piper, eSpeak-NG subprocess) bridge through
    ``asyncio.to_thread``. Network providers (ElevenLabs) use ``httpx``
    natively.

    Failures raise [`stackchan_sidecar.tts.errors.TTSError`]; the
    /v1/listen handler catches it and falls back to ``audio_url: null``.
    """

    name: str

    async def synthesize(self, text: str) -> TTSResult: ...
