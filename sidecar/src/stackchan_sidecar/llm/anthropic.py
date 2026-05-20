from typing import TYPE_CHECKING, Any, cast

import anthropic

from . import SHORT_MAX, Emotion, Reply, sanitize_short

if TYPE_CHECKING:
    from ..session_store import Turn

_RESPOND_TOOL: dict[str, Any] = {
    "name": "respond",
    "description": (
        "Send your reply to the user. `short` is shown on the avatar's 32-char "
        "toast band — it MUST be 32 characters or fewer. `full` is the longer "
        "version logged server-side. `emotion` drives the avatar's face."
    ),
    "input_schema": {
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
}


class AnthropicLLM:
    def __init__(
        self,
        api_key: str,
        model: str = "claude-haiku-4-5",
        max_tokens: int = 400,
    ) -> None:
        # `max_retries=0` disables the SDK's internal retry loop:
        # our `retry_with_timeout` already owns the retry policy,
        # and stacking SDK retries on top would produce up to 2x2
        # attempts per call and make the per-attempt timeout
        # math opaque.
        self._client = anthropic.AsyncAnthropic(api_key=api_key, max_retries=0)
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
        messages: list[dict[str, str]] = []
        for turn in history or []:
            messages.append({"role": "user", "content": turn.user})
            messages.append({"role": "assistant", "content": turn.assistant})
        messages.append({"role": "user", "content": transcript})
        response = await self._client.messages.create(
            model=self._model,
            system=persona,
            max_tokens=self._max_tokens,
            tools=[cast(Any, _RESPOND_TOOL)],
            tool_choice=cast(Any, {"type": "tool", "name": "respond"}),
            messages=cast(Any, messages),
        )
        return _parse_response(response)


def _parse_response(response: Any) -> Reply:
    for block in getattr(response, "content", []) or []:
        if getattr(block, "type", None) != "tool_use":
            continue
        if getattr(block, "name", None) != "respond":
            continue
        raw = getattr(block, "input", None) or {}
        short_raw = str(raw.get("short", "")).strip()
        full_raw = str(raw.get("full", "")).strip() or short_raw
        emotion = Emotion.coerce(raw.get("emotion"))
        short = sanitize_short(short_raw)
        return Reply(short=short, full=full_raw, emotion=emotion)
    raise ValueError("anthropic response did not include a respond tool_use block")
