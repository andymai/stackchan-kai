"""Piper TTS provider.

Local neural TTS via the ``piper`` binary plus an ONNX voice model
(``<model>.onnx`` plus the side-car ``<model>.onnx.json``). Subprocess
writes a 22050 Hz mono s16 WAV; the shared transcoder produces
16 kHz mono s16 LE PCM.
"""

from __future__ import annotations

import asyncio
import logging
import shutil
from pathlib import Path

from ._audio import synthesize_via_subprocess
from .errors import TTSError
from .protocol import TTSResult

_LOG = logging.getLogger("stackchan_sidecar.tts.piper")


class PiperProvider:
    """Shells out to ``piper`` and transcodes the resulting WAV.

    ``model_path`` is the ``.onnx`` file; ``piper`` looks for the
    accompanying ``.onnx.json`` automatically. ``speaker_id`` selects
    one of the speakers in a multi-speaker model; ignored for
    single-speaker models.
    """

    name: str = "piper"

    def __init__(self, *, model_path: Path, speaker_id: int | None = None) -> None:
        self._model_path = model_path
        self._speaker_id = speaker_id

    async def synthesize(self, text: str) -> TTSResult:
        binary = shutil.which("piper")
        if binary is None:
            raise TTSError("setup", "piper binary not found in PATH")
        if not self._model_path.is_file():
            raise TTSError("setup", f"piper model not found at {self._model_path}")
        if not text.strip():
            raise TTSError("synthesize", "empty text")
        return await asyncio.to_thread(self._synthesize_sync, binary, text)

    def _synthesize_sync(self, binary: str, text: str) -> TTSResult:
        def argv(wav: Path) -> list[str]:
            base = [binary, "--model", str(self._model_path), "--output_file", str(wav)]
            if self._speaker_id is not None:
                base += ["--speaker", str(self._speaker_id)]
            return base

        # Neural inference can be slow on cold CPU; the 30 s ceiling
        # bounds a hung process without truncating a typical reply.
        pcm = synthesize_via_subprocess(
            provider="piper",
            argv_for_wav=argv,
            text=text,
            timeout_seconds=30.0,
        )
        _LOG.debug("piper synthesized %d bytes", len(pcm))
        return TTSResult(pcm=pcm, provider=self.name, voice=self._model_path.stem)
