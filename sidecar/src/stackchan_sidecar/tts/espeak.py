"""eSpeak-NG TTS provider.

The always-available fallback — ships in 2 MB on any Linux/macOS host,
no API key, no model file. Synthesis is a subprocess call to the
``espeak-ng`` binary, which writes a WAV file we strip down to raw PCM.
The robot-voice aesthetic is intentional; if an operator wants quality
they pick Piper or ElevenLabs.

Sample rate is configurable at compile time via espeak-ng's
``--voice=`` selector; the default is 22050 Hz, so this provider does
an in-memory 22050 → 16000 resample with linear interpolation via
NumPy (already a sidecar dep). Audio quality for a robot voice on a
desk toy doesn't justify dragging scipy or libsoxr in for higher-order
polyphase resampling.
"""

from __future__ import annotations

import asyncio
import logging
import shutil
import subprocess
import tempfile
import wave
from pathlib import Path

import numpy as np

from .errors import TTSError
from .protocol import (
    PCM_CHANNELS,
    PCM_SAMPLE_RATE_HZ,
    PCM_SAMPLE_WIDTH_BYTES,
    TTSResult,
)

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
            return self._wav_to_pcm(wav_path)
        finally:
            wav_path.unlink(missing_ok=True)

    def _wav_to_pcm(self, wav_path: Path) -> TTSResult:
        try:
            with wave.open(str(wav_path), "rb") as r:
                channels = r.getnchannels()
                width = r.getsampwidth()
                src_rate = r.getframerate()
                frames = r.readframes(r.getnframes())
        except (wave.Error, FileNotFoundError) as e:
            raise TTSError("transcode", f"couldn't read espeak-ng WAV: {e}") from e
        if not frames:
            raise TTSError("synthesize", "espeak-ng produced empty audio")
        if width != PCM_SAMPLE_WIDTH_BYTES:
            raise TTSError(
                "transcode",
                f"espeak-ng WAV width {width} != expected {PCM_SAMPLE_WIDTH_BYTES}",
            )
        pcm = self._resample_and_downmix(frames, src_rate, channels)
        if not pcm:
            # The resampler returns empty for pathological inputs
            # (e.g. one source sample at a rate that rounds down to
            # < 1 destination sample). Catch it here so the audio
            # cache never holds a zero-byte entry; the firmware
            # would otherwise receive a 200 + empty body and have to
            # invent its own "is this really audio?" check.
            raise TTSError("transcode", "resampled audio is empty")
        _LOG.debug(
            "espeak-ng synthesized %d bytes (src_rate=%d, channels=%d)",
            len(pcm),
            src_rate,
            channels,
        )
        return TTSResult(pcm=pcm, provider=self.name, voice=self._voice)

    @staticmethod
    def _resample_and_downmix(frames: bytes, src_rate: int, channels: int) -> bytes:
        """Convert raw s16 LE frames at ``src_rate`` x ``channels`` to
        16 kHz mono s16 LE bytes. Pure NumPy; no audioop / scipy /
        libsoxr dependency.

        Resampling uses linear interpolation, which is fine for the
        robot-voice quality bar this provider targets. A more demanding
        provider (Piper, ElevenLabs) emits the right rate natively or
        does its own resample upstream — so this approximation is
        scoped to the fallback path.
        """
        samples = np.frombuffer(frames, dtype=np.int16)
        # Multi-channel → mono via mean. espeak-ng is mono by default
        # so this is a no-op in practice; the guard is cheap.
        if channels > 1:
            samples = samples.reshape(-1, channels).mean(axis=1).astype(np.int16)
        if src_rate != PCM_SAMPLE_RATE_HZ:
            src_len = samples.size
            dst_len = round(src_len * PCM_SAMPLE_RATE_HZ / src_rate)
            if dst_len <= 0:
                # Pathological input (one sample at 22 kHz, < 1 sample
                # at 16 kHz). Fall through to a zero-length payload --
                # the empty-output guard in `_wav_to_pcm` will surface
                # it as a TTSError on the next layer.
                return b""
            # np.interp does linear interpolation. The src/dst sample
            # positions are evenly spaced across [0, 1].
            src_positions = np.linspace(0.0, 1.0, src_len, dtype=np.float64)
            dst_positions = np.linspace(0.0, 1.0, dst_len, dtype=np.float64)
            resampled = np.interp(dst_positions, src_positions, samples.astype(np.float64))
            # Clip + cast — interp can produce out-of-range floats if
            # the input had peaks near i16 bounds.
            samples = np.clip(resampled, -32768, 32767).astype(np.int16)
        _ = PCM_CHANNELS  # mono target — guarded above
        return samples.tobytes()
