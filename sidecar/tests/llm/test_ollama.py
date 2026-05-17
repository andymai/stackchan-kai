import json
from typing import Any
from unittest.mock import AsyncMock, patch

import httpx
import pytest

from stackchan_sidecar.llm import Emotion
from stackchan_sidecar.llm.ollama import OllamaLLM
from stackchan_sidecar.session_store import Turn


def _ollama_response(payload: dict[str, Any]) -> httpx.Response:
    return httpx.Response(
        status_code=200,
        json=payload,
        request=httpx.Request("POST", "http://localhost:11434/api/chat"),
    )


async def test_reply_parses_json_content() -> None:
    llm = OllamaLLM(host="http://localhost:11434", model="llama3.2:3b")
    content = json.dumps({"short": "hi friend", "full": "Hi friend!", "emotion": "happy"})
    mock = AsyncMock(return_value=_ollama_response({"message": {"content": content}}))
    with patch.object(httpx.AsyncClient, "post", mock):
        reply = await llm.reply("hello", "be friendly", "sid")
    assert reply.short == "hi friend"
    assert reply.full == "Hi friend!"
    assert reply.emotion is Emotion.HAPPY

    mock.assert_awaited_once()
    args, kwargs = mock.await_args.args, mock.await_args.kwargs  # type: ignore[union-attr]
    url = args[0] if args else kwargs.get("url")
    assert url == "http://localhost:11434/api/chat"
    body = kwargs["json"]
    assert body["model"] == "llama3.2:3b"
    assert body["format"] == "json"
    assert body["stream"] is False
    assert body["messages"][0] == {"role": "system", "content": "be friendly"}
    assert body["messages"][-1] == {"role": "user", "content": "hello"}


async def test_reply_clamps_short() -> None:
    llm = OllamaLLM()
    long = "x" * 100
    content = json.dumps({"short": long, "full": "full", "emotion": "neutral"})
    mock = AsyncMock(return_value=_ollama_response({"message": {"content": content}}))
    with patch.object(httpx.AsyncClient, "post", mock):
        reply = await llm.reply("hi", "p", "sid")
    assert len(reply.short) == 32


async def test_reply_unknown_emotion_falls_back_to_neutral() -> None:
    llm = OllamaLLM()
    content = json.dumps({"short": "ok", "full": "ok", "emotion": "ecstatic"})
    mock = AsyncMock(return_value=_ollama_response({"message": {"content": content}}))
    with patch.object(httpx.AsyncClient, "post", mock):
        reply = await llm.reply("hi", "p", "sid")
    assert reply.emotion is Emotion.NEUTRAL


async def test_reply_includes_history() -> None:
    llm = OllamaLLM()
    content = json.dumps({"short": "ok", "full": "ok", "emotion": "neutral"})
    mock = AsyncMock(return_value=_ollama_response({"message": {"content": content}}))
    history = [Turn(user="u1", assistant="a1", emotion=Emotion.NEUTRAL)]
    with patch.object(httpx.AsyncClient, "post", mock):
        await llm.reply("u2", "p", "sid", history=history)
    body = mock.await_args.kwargs["json"]  # type: ignore[union-attr]
    msgs = body["messages"]
    assert msgs[1] == {"role": "user", "content": "u1"}
    assert msgs[2] == {"role": "assistant", "content": "a1"}
    assert msgs[3] == {"role": "user", "content": "u2"}


async def test_reply_raises_on_non_json_content() -> None:
    llm = OllamaLLM()
    mock = AsyncMock(return_value=_ollama_response({"message": {"content": "not-json"}}))
    with (
        patch.object(httpx.AsyncClient, "post", mock),
        pytest.raises(ValueError, match="non-JSON"),
    ):
        await llm.reply("hi", "p", "sid")
