"""eSpeak-NG TTS provider.

Always-available fallback — subprocess call to ``espeak-ng`` writes a
WAV; the shared transcoder produces 16 kHz mono s16 LE PCM. No API
key, no model file.
"""

from __future__ import annotations

import asyncio
import logging
import shutil
from pathlib import Path

from ._audio import synthesize_via_subprocess
from .errors import TTSError
from .protocol import TTSResult

_LOG = logging.getLogger("stackchan_sidecar.tts.espeak")

# `175 wpm` is the espeak-ng default; comfortable for a desk toy.
_DEFAULT_VOICE = "en"
_DEFAULT_WPM = 175


class EspeakProvider:
    """Shells out to ``espeak-ng`` and transcodes the resulting WAV.

    Construction probes for the binary lazily so a sidecar that runs
    a different provider doesn't need espeak-ng installed.
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
        return await asyncio.to_thread(self._synthesize_sync, binary, text)

    def _synthesize_sync(self, binary: str, text: str) -> TTSResult:
        # `--stdin` reads the text from stdin, dodging shell-escape
        # gotchas if the LLM reply contains quotes or backticks.
        def argv(wav: Path) -> list[str]:
            return [binary, "--stdin", "-v", self._voice, "-s", str(self._wpm), "-w", str(wav)]

        pcm = synthesize_via_subprocess(
            provider="espeak-ng",
            argv_for_wav=argv,
            text=text,
            timeout_seconds=10.0,
        )
        _LOG.debug("espeak-ng synthesized %d bytes", len(pcm))
        return TTSResult(pcm=pcm, provider=self.name, voice=self._voice)
