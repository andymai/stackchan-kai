import pytest

from stackchan_sidecar.config import load_settings


def test_load_settings_requires_bearer_token(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("SIDECAR_BEARER_TOKEN", raising=False)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    with pytest.raises(RuntimeError, match="SIDECAR_BEARER_TOKEN"):
        load_settings()


def test_load_settings_requires_anthropic_key_when_anthropic_selected(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("SIDECAR_BEARER_TOKEN", "tok")
    monkeypatch.delenv("ANTHROPIC_API_KEY", raising=False)
    with pytest.raises(RuntimeError, match="ANTHROPIC_API_KEY"):
        load_settings()


def test_load_settings_returns_settings_when_both_set(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("SIDECAR_BEARER_TOKEN", "tok")
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    settings = load_settings()
    assert settings.bearer_token == "tok"
    assert settings.anthropic_api_key == "sk-ant-test"


def test_load_settings_no_anthropic_key_when_ollama_selected(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("SIDECAR_BEARER_TOKEN", "tok")
    monkeypatch.delenv("ANTHROPIC_API_KEY", raising=False)
    monkeypatch.setenv("LLM_PROVIDER", "ollama")
    settings = load_settings()
    assert settings.llm_provider == "ollama"


def test_load_settings_requires_openai_key_when_openai_llm_selected(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("SIDECAR_BEARER_TOKEN", "tok")
    monkeypatch.setenv("LLM_PROVIDER", "openai")
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    with pytest.raises(RuntimeError, match="OPENAI_API_KEY"):
        load_settings()


def test_load_settings_requires_openai_key_when_openai_stt_selected(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("SIDECAR_BEARER_TOKEN", "tok")
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    monkeypatch.setenv("STT_PROVIDER", "openai")
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    with pytest.raises(RuntimeError, match="OPENAI_API_KEY"):
        load_settings()


def test_load_settings_requires_deepgram_key_when_deepgram_selected(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("SIDECAR_BEARER_TOKEN", "tok")
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    monkeypatch.setenv("STT_PROVIDER", "deepgram")
    monkeypatch.delenv("DEEPGRAM_API_KEY", raising=False)
    with pytest.raises(RuntimeError, match="DEEPGRAM_API_KEY"):
        load_settings()
