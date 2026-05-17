from typing import Any
from unittest.mock import AsyncMock, patch

import httpx

from stackchan_sidecar.stt.deepgram import DeepgramSTT


def _dg_response(payload: dict[str, Any]) -> httpx.Response:
    return httpx.Response(
        status_code=200,
        json=payload,
        request=httpx.Request("POST", "https://api.deepgram.com/v1/listen"),
    )


async def test_transcribe_extracts_transcript() -> None:
    stt = DeepgramSTT(api_key="dg-key")
    payload = {
        "results": {
            "channels": [
                {"alternatives": [{"transcript": "hello stack chan"}]},
            ]
        }
    }
    mock = AsyncMock(return_value=_dg_response(payload))
    with patch.object(httpx.AsyncClient, "post", mock):
        result = await stt.transcribe(b"\x00\x00" * 1600, sample_rate=16000)
    assert result == "hello stack chan"


async def test_transcribe_empty_input_short_circuits() -> None:
    stt = DeepgramSTT(api_key="dg-key")
    mock = AsyncMock()
    with patch.object(httpx.AsyncClient, "post", mock):
        assert await stt.transcribe(b"", sample_rate=16000) == ""
    mock.assert_not_awaited()


async def test_transcribe_request_shape() -> None:
    stt = DeepgramSTT(api_key="dg-key", model="nova-2", language="en")
    payload = {"results": {"channels": [{"alternatives": [{"transcript": "x"}]}]}}
    pcm = b"\x01\x02" * 800
    mock = AsyncMock(return_value=_dg_response(payload))
    with patch.object(httpx.AsyncClient, "post", mock):
        await stt.transcribe(pcm, sample_rate=16000)

    mock.assert_awaited_once()
    args = mock.await_args.args  # type: ignore[union-attr]
    kwargs = mock.await_args.kwargs  # type: ignore[union-attr]
    url = args[0] if args else kwargs.get("url")
    assert url == "https://api.deepgram.com/v1/listen"
    headers = kwargs["headers"]
    assert headers["Authorization"] == "Token dg-key"
    assert headers["Content-Type"] == "audio/L16;rate=16000;channels=1"
    assert kwargs["params"] == {"model": "nova-2", "language": "en"}
    assert kwargs["content"] == pcm


async def test_transcribe_missing_channels_returns_empty() -> None:
    stt = DeepgramSTT(api_key="dg-key")
    mock = AsyncMock(return_value=_dg_response({"results": {"channels": []}}))
    with patch.object(httpx.AsyncClient, "post", mock):
        assert await stt.transcribe(b"\x00\x00" * 800, sample_rate=16000) == ""


async def test_transcribe_missing_alternatives_returns_empty() -> None:
    stt = DeepgramSTT(api_key="dg-key")
    payload: dict[str, Any] = {"results": {"channels": [{"alternatives": []}]}}
    mock = AsyncMock(return_value=_dg_response(payload))
    with patch.object(httpx.AsyncClient, "post", mock):
        assert await stt.transcribe(b"\x00\x00" * 800, sample_rate=16000) == ""
