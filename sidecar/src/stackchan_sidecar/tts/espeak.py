"""eSpeak-NG TTS provider.

The always-available fallback — ships in 2 MB on any Linux/macOS host,
no API key, no model file. Synthesis is a subprocess call to the
``espeak-ng`` binary, which writes a WAV file we strip down to raw PCM
via the shared [`wav_to_pcm`] helper.

The robot-voice aesthetic is intentional; if an operator wants quality
they pick Piper or ElevenLabs.
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

_LOG = logging.getLogger("stackchan_sidecar.tts.espeak")

# espeak-ng's default voice & rate. The voice selector controls
# language/accent; the rate selector controls words-per-minute.
# `175 wpm` is the espeak-ng default; comfortable for a desk toy.
_DEFAULT_VOICE = "en"
_DEFAULT_WPM = 175


class EspeakProvider:
    """Provider that shells out to ``espeak-ng`` and post-processes
    the resulting WAV into 16 kHz mono PCM.

    Construction probes for the binary in PATH; if missing,
    [`synthesize`] raises [`TTSError`] with stage ``"setup"`` on the
    first call. Probe is lazy so a sidecar that uses ElevenLabs or
    Piper as its real provider doesn't need espeak-ng installed.
    """

    name: str = "espeak_ng"

    def __init__(self, *, voice: str = _DEFAULT_VOICE, wpm: int = _DEFAULT_WPM) -> None:
        self._voice = voice
        self._wpm = wpm

    async def synthesize(self, text: str) -> TTSResult:
        binary = shutil.which("espeak-ng")
        if binary is None:
            raise TTSError("setup", "espeak-ng binary not found in PATH")
        if not text.strip():
            raise TTSError("synthesize", "empty text")
        # Subprocess runs to completion in a thread so the FastAPI event
        # loop stays responsive. Synthesis is fast enough (< 200 ms for
        # toast-band-length text) that streaming isn't worth the
        # complexity for this provider.
        return await asyncio.to_thread(self._synthesize_sync, binary, text)

    def _synthesize_sync(self, binary: str, text: str) -> TTSResult:
        with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp:
            wav_path = Path(tmp.name)
        try:
            try:
                # `--stdin` reads the text from stdin, dodging shell-escape
                # gotchas if the LLM reply contains quotes or backticks.
                subprocess.run(
                    [
                        binary,
                        "--stdin",
                        "-v",
                        self._voice,
                        "-s",
                        str(self._wpm),
                        "-w",
                        str(wav_path),
                    ],
                    input=text,
                    text=True,
                    check=True,
                    timeout=10.0,
                    capture_output=True,
                )
            except subprocess.CalledProcessError as e:
                raise TTSError(
                    "synthesize",
                    f"espeak-ng exited {e.returncode}: {e.stderr.strip()[:120]}",
                ) from e
            except subprocess.TimeoutExpired as e:
                raise TTSError("synthesize", "espeak-ng timed out") from e
            pcm = wav_to_pcm(wav_path, provider="espeak-ng")
            _LOG.debug("espeak-ng synthesized %d bytes", len(pcm))
            return TTSResult(pcm=pcm, provider=self.name, voice=self._voice)
        finally:
            wav_path.unlink(missing_ok=True)
