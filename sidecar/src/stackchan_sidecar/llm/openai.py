import json
from typing import TYPE_CHECKING, Any, cast

import openai

from . import SHORT_MAX, Emotion, Reply, sanitize_short

if TYPE_CHECKING:
    from ..session_store import Turn

_RESPOND_TOOL: dict[str, Any] = {
    "type": "function",
    "function": {
        "name": "respond",
        "description": (
            "Send your reply to the user. `short` is shown on the avatar's 32-char "
            "toast band — it MUST be 32 characters or fewer. `full` is the longer "
            "version logged server-side. `emotion` drives the avatar's face."
        ),
        "parameters": {
            "type": "object",
            "properties": {
                "short": {
                    "type": "string",
                    "maxLength": SHORT_MAX,
                    "description": (
                        "Reply for the avatar's 32-char toast band — must fit. "
                        "No embedded double-quote characters."
                    ),
                },
                "full": {
                    "type": "string",
                    "description": "Longer form of the reply, logged but not displayed.",
                },
                "emotion": {
                    "type": "string",
                    "enum": [e.value for e in Emotion],
                    "description": "Face/voice emotion to render with the reply.",
                },
            },
            "required": ["short", "full", "emotion"],
        },
    },
}


class OpenAILLM:
    def __init__(
        self,
        api_key: str,
        model: str = "gpt-4o-mini",
        max_tokens: int = 400,
    ) -> None:
        # `max_retries=0`: see `AnthropicLLM.__init__` — our
        # `retry_with_timeout` owns the retry policy. Stacking SDK
        # retries on top would burn the per-attempt timeout budget
        # invisibly.
        self._client = openai.AsyncOpenAI(api_key=api_key, max_retries=0)
        self._model = model
        self._max_tokens = max_tokens

    async def reply(
        self,
        transcript: str,
        persona: str,
        session_id: str,
        history: "list[Turn] | None" = None,
    ) -> Reply:
        _ = session_id
        messages: list[dict[str, str]] = [{"role": "system", "content": persona}]
        for turn in history or []:
            messages.append({"role": "user", "content": turn.user})
            messages.append({"role": "assistant", "content": turn.assistant})
        messages.append({"role": "user", "content": transcript})
        response = await self._client.chat.completions.create(
            model=self._model,
            max_tokens=self._max_tokens,
            messages=cast(Any, messages),
            tools=cast(Any, [_RESPOND_TOOL]),
            tool_choice=cast(Any, {"type": "function", "function": {"name": "respond"}}),
        )
        return _parse_response(response)


def _parse_response(response: Any) -> Reply:
    choices = getattr(response, "choices", None) or []
    if not choices:
        raise ValueError("openai response had no choices")
    message = getattr(choices[0], "message", None)
    tool_calls = getattr(message, "tool_calls", None) or []
    if not tool_calls:
        raise ValueError("openai response did not include a respond tool_call")
    call = tool_calls[0]
    fn = getattr(call, "function", None)
    args_raw = getattr(fn, "arguments", None) or "{}"
    try:
        raw = json.loads(args_raw)
    except json.JSONDecodeError as e:
        raise ValueError(f"openai tool_call arguments were not JSON: {e}") from e
    short_raw = str(raw.get("short", "")).strip()
    full_raw = str(raw.get("full", "")).strip() or short_raw
    emotion = Emotion.coerce(raw.get("emotion"))
    short = sanitize_short(short_raw)
    return Reply(short=short, full=full_raw, emotion=emotion)
