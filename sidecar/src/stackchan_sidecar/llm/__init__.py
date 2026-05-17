from dataclasses import dataclass
from enum import StrEnum
from typing import Protocol, runtime_checkable

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
    async def reply(self, transcript: str, persona: str, session_id: str) -> Reply: ...
