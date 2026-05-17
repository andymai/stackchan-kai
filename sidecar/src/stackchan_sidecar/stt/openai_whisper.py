import io
import wave

import openai


def _pcm_to_wav(pcm: bytes, sample_rate: int) -> bytes:
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sample_rate)
        w.writeframes(pcm)
    return buf.getvalue()


class OpenAIWhisperSTT:
    def __init__(self, api_key: str, model: str = "whisper-1") -> None:
        self._client = openai.AsyncOpenAI(api_key=api_key)
        self._model = model

    async def transcribe(self, pcm: bytes, sample_rate: int = 16000) -> str:
        if not pcm:
            return ""
        wav_bytes = _pcm_to_wav(pcm, sample_rate)
        response = await self._client.audio.transcriptions.create(
            model=self._model,
            file=("audio.wav", wav_bytes, "audio/wav"),
            language="en",
        )
        return (getattr(response, "text", "") or "").strip()
