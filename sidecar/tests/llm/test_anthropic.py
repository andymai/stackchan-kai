from dataclasses import dataclass
from typing import Any
from unittest.mock import AsyncMock, patch

import pytest

from stackchan_sidecar.llm import Emotion
from stackchan_sidecar.llm.anthropic import AnthropicLLM


@dataclass
class _ToolBlock:
    type: str
    name: str
    input: dict[str, Any]


@dataclass
class _TextBlock:
    type: str
    text: str


@dataclass
class _Response:
    content: list[Any]


def _make_response(short: str, full: str, emotion: str) -> _Response:
    return _Response(
        content=[
            _TextBlock(type="text", text="ignored preamble"),
            _ToolBlock(
                type="tool_use",
                name="respond",
                input={"short": short, "full": full, "emotion": emotion},
            ),
        ]
    )


async def test_reply_parses_tool_use_block() -> None:
    llm = AnthropicLLM(api_key="sk-ant-test", model="claude-haiku-4-5")
    mock = AsyncMock(return_value=_make_response("hi there", "Hi there friend!", "happy"))
    with patch.object(llm._client.messages, "create", mock):
        reply = await llm.reply("hello", "you are a robot", "session-abc")
    assert reply.short == "hi there"
    assert reply.full == "Hi there friend!"
    assert reply.emotion is Emotion.HAPPY
    mock.assert_awaited_once()
    await_args = mock.await_args
    assert await_args is not None
    kwargs = await_args.kwargs
    assert kwargs["model"] == "claude-haiku-4-5"
    assert kwargs["system"] == "you are a robot"
    assert kwargs["tool_choice"] == {"type": "tool", "name": "respond"}
    assert kwargs["messages"] == [{"role": "user", "content": "hello"}]


async def test_reply_clamps_short_to_32() -> None:
    llm = AnthropicLLM(api_key="sk-ant-test")
    long = "a" * 100
    mock = AsyncMock(return_value=_make_response(long, "full body", "neutral"))
    with patch.object(llm._client.messages, "create", mock):
        reply = await llm.reply("hi", "persona", "sid")
    assert len(reply.short) == 32


async def test_reply_unknown_emotion_falls_back_to_neutral() -> None:
    llm = AnthropicLLM(api_key="sk-ant-test")
    mock = AsyncMock(return_value=_make_response("ok", "ok", "ecstatic"))
    with patch.object(llm._client.messages, "create", mock):
        reply = await llm.reply("hi", "persona", "sid")
    assert reply.emotion is Emotion.NEUTRAL


async def test_reply_strips_embedded_quotes() -> None:
    llm = AnthropicLLM(api_key="sk-ant-test")
    mock = AsyncMock(return_value=_make_response('hi "there"', "full body", "happy"))
    with patch.object(llm._client.messages, "create", mock):
        reply = await llm.reply("hi", "persona", "sid")
    assert '"' not in reply.short


async def test_reply_missing_tool_use_raises() -> None:
    llm = AnthropicLLM(api_key="sk-ant-test")
    mock = AsyncMock(return_value=_Response(content=[_TextBlock(type="text", text="oops")]))
    with (
        patch.object(llm._client.messages, "create", mock),
        pytest.raises(ValueError, match="tool_use"),
    ):
        await llm.reply("hi", "persona", "sid")
