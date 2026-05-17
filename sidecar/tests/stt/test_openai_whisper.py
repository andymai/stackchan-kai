import io
import wave
from dataclasses import dataclass
from typing import Any
from unittest.mock import AsyncMock, patch

from stackchan_sidecar.stt.openai_whisper import OpenAIWhisperSTT


@dataclass
class _TranscriptionResponse:
    text: str


async def test_transcribe_returns_text_and_strips_whitespace() -> None:
    stt = OpenAIWhisperSTT(api_key="sk-test", model="whisper-1")
    mock = AsyncMock(return_value=_TranscriptionResponse(text="  hello there  "))
    pcm = b"\x00\x00" * 1600
    with patch.object(stt._client.audio.transcriptions, "create", mock):
        result = await stt.transcribe(pcm, sample_rate=16000)
    assert result == "hello there"
    mock.assert_awaited_once()


async def test_transcribe_empty_input_returns_empty_string() -> None:
    stt = OpenAIWhisperSTT(api_key="sk-test")
    mock = AsyncMock(return_value=_TranscriptionResponse(text="should not be returned"))
    with patch.object(stt._client.audio.transcriptions, "create", mock):
        assert await stt.transcribe(b"", sample_rate=16000) == ""
    mock.assert_not_awaited()


async def test_transcribe_wraps_pcm_in_valid_wav() -> None:
    stt = OpenAIWhisperSTT(api_key="sk-test")
    captured: dict[str, Any] = {}

    async def fake_create(**kwargs: Any) -> _TranscriptionResponse:
        captured.update(kwargs)
        return _TranscriptionResponse(text="ok")

    pcm = b"\x10\x20" * 800
    with patch.object(stt._client.audio.transcriptions, "create", side_effect=fake_create):
        await stt.transcribe(pcm, sample_rate=16000)

    file_tuple = captured["file"]
    assert file_tuple[0] == "audio.wav"
    wav_bytes = file_tuple[1]
    assert file_tuple[2] == "audio/wav"

    with wave.open(io.BytesIO(wav_bytes), "rb") as w:
        assert w.getnchannels() == 1
        assert w.getsampwidth() == 2
        assert w.getframerate() == 16000
        assert w.readframes(w.getnframes()) == pcm


async def test_transcribe_passes_model() -> None:
    stt = OpenAIWhisperSTT(api_key="sk-test", model="whisper-2")
    captured: dict[str, Any] = {}

    async def fake_create(**kwargs: Any) -> _TranscriptionResponse:
        captured.update(kwargs)
        return _TranscriptionResponse(text="ok")

    with patch.object(stt._client.audio.transcriptions, "create", side_effect=fake_create):
        await stt.transcribe(b"\x00\x00" * 800, sample_rate=16000)

    assert captured["model"] == "whisper-2"
