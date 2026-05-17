import json
from dataclasses import dataclass
from typing import Any
from unittest.mock import AsyncMock, patch

import pytest

from stackchan_sidecar.llm import Emotion
from stackchan_sidecar.llm.openai import OpenAILLM
from stackchan_sidecar.session_store import Turn


@dataclass
class _Function:
    arguments: str


@dataclass
class _ToolCall:
    function: _Function


@dataclass
class _Message:
    tool_calls: list[_ToolCall]


@dataclass
class _Choice:
    message: _Message


@dataclass
class _Response:
    choices: list[_Choice]


def _make_response(short: str, full: str, emotion: str) -> _Response:
    payload = json.dumps({"short": short, "full": full, "emotion": emotion})
    return _Response(
        choices=[_Choice(message=_Message(tool_calls=[_ToolCall(function=_Function(payload))]))]
    )


async def test_reply_parses_tool_call() -> None:
    llm = OpenAILLM(api_key="sk-test", model="gpt-4o-mini")
    mock = AsyncMock(return_value=_make_response("hi there", "Hi there friend!", "happy"))
    with patch.object(llm._client.chat.completions, "create", mock):
        reply = await llm.reply("hello", "you are a robot", "session-abc")
    assert reply.short == "hi there"
    assert reply.full == "Hi there friend!"
    assert reply.emotion is Emotion.HAPPY
    mock.assert_awaited_once()
    kwargs = mock.await_args.kwargs  # type: ignore[union-attr]
    assert kwargs["model"] == "gpt-4o-mini"
    assert kwargs["tool_choice"] == {"type": "function", "function": {"name": "respond"}}
    assert kwargs["messages"][0] == {"role": "system", "content": "you are a robot"}
    assert kwargs["messages"][-1] == {"role": "user", "content": "hello"}


async def test_reply_clamps_short_to_32() -> None:
    llm = OpenAILLM(api_key="sk-test")
    long = "a" * 100
    mock = AsyncMock(return_value=_make_response(long, "full body", "neutral"))
    with patch.object(llm._client.chat.completions, "create", mock):
        reply = await llm.reply("hi", "persona", "sid")
    assert len(reply.short) == 32


async def test_reply_unknown_emotion_falls_back_to_neutral() -> None:
    llm = OpenAILLM(api_key="sk-test")
    mock = AsyncMock(return_value=_make_response("ok", "ok", "ecstatic"))
    with patch.object(llm._client.chat.completions, "create", mock):
        reply = await llm.reply("hi", "persona", "sid")
    assert reply.emotion is Emotion.NEUTRAL


async def test_reply_strips_embedded_quotes() -> None:
    llm = OpenAILLM(api_key="sk-test")
    mock = AsyncMock(return_value=_make_response('hi "there"', "full body", "happy"))
    with patch.object(llm._client.chat.completions, "create", mock):
        reply = await llm.reply("hi", "persona", "sid")
    assert '"' not in reply.short


async def test_reply_includes_history_in_messages() -> None:
    llm = OpenAILLM(api_key="sk-test")
    mock = AsyncMock(return_value=_make_response("ok", "ok", "neutral"))
    history = [Turn(user="first user", assistant="first assistant", emotion=Emotion.NEUTRAL)]
    with patch.object(llm._client.chat.completions, "create", mock):
        await llm.reply("second user", "persona", "sid", history=history)
    kwargs = mock.await_args.kwargs  # type: ignore[union-attr]
    msgs = kwargs["messages"]
    assert msgs[0]["role"] == "system"
    assert msgs[1] == {"role": "user", "content": "first user"}
    assert msgs[2] == {"role": "assistant", "content": "first assistant"}
    assert msgs[3] == {"role": "user", "content": "second user"}


async def test_reply_missing_tool_calls_raises() -> None:
    llm = OpenAILLM(api_key="sk-test")
    empty = _Response(choices=[_Choice(message=_Message(tool_calls=[]))])
    mock = AsyncMock(return_value=empty)
    with (
        patch.object(llm._client.chat.completions, "create", mock),
        pytest.raises(ValueError, match="tool_call"),
    ):
        await llm.reply("hi", "persona", "sid")


async def test_reply_bad_json_arguments_raises() -> None:
    llm = OpenAILLM(api_key="sk-test")
    bad: Any = _Response(
        choices=[_Choice(message=_Message(tool_calls=[_ToolCall(function=_Function("not-json"))]))]
    )
    mock = AsyncMock(return_value=bad)
    with (
        patch.object(llm._client.chat.completions, "create", mock),
        pytest.raises(ValueError, match="JSON"),
    ):
        await llm.reply("hi", "persona", "sid")
