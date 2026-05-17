from dataclasses import dataclass
from enum import StrEnum
from typing import TYPE_CHECKING, Protocol, runtime_checkable

if TYPE_CHECKING:
    from ..session_store import Turn

SHORT_MAX = 32
"""Width of the firmware's toast band. The `Reply.short` field MUST fit."""


class Emotion(StrEnum):
    NEUTRAL = "neutral"
    HAPPY = "happy"
    SLEEPY = "sleepy"
    SURPRISED = "surprised"
    SAD = "sad"
    ANGRY = "angry"

    @classmethod
    def coerce(cls, value: str | None) -> "Emotion":
        if value is None:
            return cls.NEUTRAL
        try:
            return cls(value)
        except ValueError:
            return cls.NEUTRAL


@dataclass(frozen=True)
class Reply:
    short: str
    full: str
    emotion: Emotion


@runtime_checkable
class LLMProvider(Protocol):
    async def reply(
        self,
        transcript: str,
        persona: str,
        session_id: str,
        history: "list[Turn] | None" = None,
    ) -> Reply: ...


def sanitize_short(text: str) -> str:
    cleaned = text.replace('"', "'")
    if len(cleaned) <= SHORT_MAX:
        return cleaned
    return cleaned[:SHORT_MAX].rstrip()
