import asyncio

import numpy as np
from faster_whisper import WhisperModel


class FasterWhisperSTT:
    def __init__(self, model_name: str = "base.en", device: str = "cpu") -> None:
        self._model = WhisperModel(model_name, device=device, compute_type="int8")

    async def transcribe(self, pcm: bytes, sample_rate: int = 16000) -> str:
        return await asyncio.to_thread(self._transcribe_sync, pcm, sample_rate)

    def _transcribe_sync(self, pcm: bytes, sample_rate: int) -> str:
        if not pcm:
            return ""
        samples = np.frombuffer(pcm, dtype=np.int16).astype(np.float32) / 32768.0
        segments, _info = self._model.transcribe(
            samples,
            language="en",
            beam_size=1,
            vad_filter=True,
        )
        return " ".join(seg.text.strip() for seg in segments).strip()
