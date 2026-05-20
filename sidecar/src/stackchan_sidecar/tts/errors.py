"""Canonical TTS failure type. Catch-and-convert at the ``/v1/listen``
boundary so a TTS outage degrades to ``audio_url: null`` instead of
taking down the whole reply path."""

from __future__ import annotations


class TTSError(Exception):
    """Synthesis couldn't produce well-shaped PCM.

    ``stage`` is one of ``"setup"`` (missing binary / model / key),
    ``"synthesize"`` (subprocess or HTTP failure, empty output), or
    ``"transcode"`` (resample / format conversion failed). The
    underlying exception (if any) is the ``__cause__``.
    """

    def __init__(self, stage: str, detail: str) -> None:
        super().__init__(f"tts {stage}: {detail}")
        self.stage = stage
        self.detail = detail
