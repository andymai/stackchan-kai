import json
import logging
from typing import TYPE_CHECKING

import httpx

from . import Emotion, Reply, sanitize_short

if TYPE_CHECKING:
    from ..session_store import Turn

_LOG = logging.getLogger("stackchan_sidecar")


class OllamaLLM:
    def __init__(
        self,
        host: str = "http://localhost:11434",
        model: str = "llama3.2:3b",
        timeout_seconds: float = 60.0,
    ) -> None:
        self._host = host.rstrip("/")
        self._model = model
        self._timeout_seconds = timeout_seconds

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

        async with httpx.AsyncClient(timeout=self._timeout_seconds) as client:
            response = await client.post(
                f"{self._host}/api/chat",
                json={
                    "model": self._model,
                    "messages": messages,
                    "format": "json",
                    "stream": False,
                },
            )
            response.raise_for_status()
            payload = response.json()

        content = (payload.get("message") or {}).get("content", "") or ""
        try:
            raw = json.loads(content)
        except json.JSONDecodeError:
            _LOG.error("ollama.parse_failed", extra={"raw_content": content})
            raise ValueError("ollama returned non-JSON content") from None

        short_raw = str(raw.get("short", "")).strip()
        full_raw = str(raw.get("full", "")).strip() or short_raw
        emotion = Emotion.coerce(raw.get("emotion"))
        short = sanitize_short(short_raw)
        return Reply(short=short, full=full_raw, emotion=emotion)
