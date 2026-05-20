"""Canonical TTS failure type. Catch-and-convert at the /v1/listen
boundary so a TTS outage degrades to ``audio_url: null`` instead of
taking down the whole reply path."""

from __future__ import annotations


class TTSError(Exception):
    """Raised by a [`TTSProvider`] when synthesis can't produce
    well-shaped PCM. Carries a short slug usable in logs / error
    envelopes; the underlying exception (if any) is the ``__cause__``.

    Stage is one of:
    - ``"setup"`` — provider couldn't initialise (missing binary,
      missing model file, missing API key)
    - ``"synthesize"`` — provider's synthesis call failed (subprocess
      crash, HTTP 5xx, timeout, empty output)
    - ``"transcode"`` — bytes returned but the resample / format
      conversion to 16 kHz mono s16 LE failed
    """

    def __init__(self, stage: str, detail: str) -> None:
        super().__init__(f"tts {stage}: {detail}")
        self.stage = stage
        self.detail = detail
