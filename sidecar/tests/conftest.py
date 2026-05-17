from collections.abc import Iterator
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from stackchan_sidecar.app import create_app
from stackchan_sidecar.config import Settings
from stackchan_sidecar.llm import Emotion, Reply
from stackchan_sidecar.session_store import Turn

TEST_TOKEN = "test-token-do-not-use-in-prod"


class FakeSTT:
    def __init__(self, transcript: str = "hello stack chan") -> None:
        self.transcript = transcript
        self.calls: list[tuple[bytes, int]] = []

    async def transcribe(self, pcm: bytes, sample_rate: int = 16000) -> str:
        self.calls.append((pcm, sample_rate))
        return self.transcript


class FakeLLM:
    def __init__(self, reply: Reply | None = None) -> None:
        self.reply_value = reply or Reply(
            short="hi friend!",
            full="Hi friend! Lovely to meet you.",
            emotion=Emotion.HAPPY,
        )
        self.calls: list[tuple[str, str, str, list[Turn]]] = []

    async def reply(
        self,
        transcript: str,
        persona: str,
        session_id: str,
        history: list[Turn] | None = None,
    ) -> Reply:
        self.calls.append((transcript, persona, session_id, list(history or [])))
        return self.reply_value


@pytest.fixture
def personas_dir(tmp_path: Path) -> Path:
    d = tmp_path / "personas"
    d.mkdir()
    (d / "stack-chan.md").write_text(
        "---\nname: stack-chan\n---\nYou are a small helpful robot.\n",
        encoding="utf-8",
    )
    return d


@pytest.fixture
def settings(personas_dir: Path) -> Settings:
    return Settings(
        SIDECAR_BEARER_TOKEN=TEST_TOKEN,
        ANTHROPIC_API_KEY="sk-ant-test",
        personas_dir=personas_dir,
    )


@pytest.fixture
def fake_stt() -> FakeSTT:
    return FakeSTT()


@pytest.fixture
def fake_llm() -> FakeLLM:
    return FakeLLM()


@pytest.fixture
def client(
    settings: Settings,
    fake_stt: FakeSTT,
    fake_llm: FakeLLM,
) -> Iterator[TestClient]:
    app = create_app(settings, fake_stt, fake_llm)
    with TestClient(app) as c:
        yield c


@pytest.fixture
def auth_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {TEST_TOKEN}"}


@pytest.fixture
def pcm_payload() -> bytes:
    return b"\x00\x00" * 1600
