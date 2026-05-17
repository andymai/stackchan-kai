from typing import Protocol, runtime_checkable


@runtime_checkable
class STTProvider(Protocol):
    async def transcribe(self, pcm: bytes, sample_rate: int = 16000) -> str: ...
