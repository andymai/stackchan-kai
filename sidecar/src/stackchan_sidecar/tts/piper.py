"""Piper TTS provider.

Piper is a fast, local, neural TTS that runs from a model file on
CPU — no API key, no GPU. Voice quality is "competent neural" rather
than "indistinguishable from human", which is the right spot for a
desk toy: clearer than espeak-ng's robot voice, no per-utterance
network cost or rate limit.

Setup is *not* zero-touch: the operator must install the
``piper`` binary and download an ONNX voice model + JSON metadata
(both `.onnx` and `.onnx.json` must sit side-by-side). Without those,
``PiperProvider`` raises [`TTSError`] with stage ``"setup"`` on the
first synthesis call.

Wire shape is identical to espeak-ng: subprocess writes a 22050 Hz
mono s16 WAV, we transcode to 16 kHz mono s16 LE PCM via the shared
[`wav_to_pcm`] helper.
"""

from __future__ import annotations

import asyncio
import logging
import shutil
import subprocess
import tempfile
from pathlib import Path

from ._audio import wav_to_pcm
from .errors import TTSError
from .protocol import TTSResult

_LOG = logging.getLogger("stackchan_sidecar.tts.piper")


class PiperProvider:
    """Provider that shells out to ``piper`` and post-processes the
    resulting WAV into 16 kHz mono PCM.

    The model path is supplied at construction; ``piper`` will look
    for ``<model_path>.json`` automatically (the voice's metadata),
    so callers only pass the ``.onnx`` path. ``speaker_id`` selects
    one of the speakers in a multi-speaker model; for single-speaker
    models it is ignored.
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
            raise TTSError(
                "setup",
                f"piper model not found at {self._model_path}",
            )
        if not text.strip():
            raise TTSError("synthesize", "empty text")
        return await asyncio.to_thread(self._synthesize_sync, binary, text)

    def _synthesize_sync(self, binary: str, text: str) -> TTSResult:
        with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp:
            wav_path = Path(tmp.name)
        try:
            args = [
                binary,
                "--model",
                str(self._model_path),
                "--output_file",
                str(wav_path),
            ]
            if self._speaker_id is not None:
                args += ["--speaker", str(self._speaker_id)]
            try:
                subprocess.run(
                    args,
                    input=text,
                    text=True,
                    check=True,
                    # Neural inference can be slow on cold CPU; give it
                    # enough runway for a long-ish reply but bound it
                    # so a hung process doesn't pin the request.
                    timeout=30.0,
                    capture_output=True,
                )
            except subprocess.CalledProcessError as e:
                raise TTSError(
                    "synthesize",
                    f"piper exited {e.returncode}: {e.stderr.strip()[:120]}",
                ) from e
            except subprocess.TimeoutExpired as e:
                raise TTSError("synthesize", "piper timed out") from e
            pcm = wav_to_pcm(wav_path, provider="piper")
            _LOG.debug("piper synthesized %d bytes", len(pcm))
            voice = self._model_path.stem
            return TTSResult(pcm=pcm, provider=self.name, voice=voice)
        finally:
            wav_path.unlink(missing_ok=True)
