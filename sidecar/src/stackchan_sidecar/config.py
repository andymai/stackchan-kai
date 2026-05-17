from pathlib import Path
from typing import Literal

from pydantic import Field
from pydantic_settings import BaseSettings, SettingsConfigDict


class Settings(BaseSettings):
    model_config = SettingsConfigDict(
        env_file=".env",
        env_file_encoding="utf-8",
        extra="ignore",
    )

    host: str = "0.0.0.0"
    port: int = 8080
    stt_model: str = "base.en"
    llm_model: str = "claude-haiku-4-5"
    persona: str = "stack-chan"
    personas_dir: Path = Path("./personas")
    log_level: str = "INFO"

    stt_provider: Literal["faster_whisper", "openai", "deepgram"] = "faster_whisper"
    llm_provider: Literal["anthropic", "openai", "ollama"] = "anthropic"
    ollama_host: str = "http://localhost:11434"

    bearer_token: str = Field(default="", alias="SIDECAR_BEARER_TOKEN")
    anthropic_api_key: str = Field(default="", alias="ANTHROPIC_API_KEY")
    openai_api_key: str = Field(default="", alias="OPENAI_API_KEY")
    deepgram_api_key: str = Field(default="", alias="DEEPGRAM_API_KEY")


def load_settings() -> Settings:
    settings = Settings()
    if not settings.bearer_token:
        raise RuntimeError(
            "SIDECAR_BEARER_TOKEN is not set. Refusing to start without a bearer "
            "token — set it in .env or the process environment."
        )
    if settings.stt_provider == "openai" and not settings.openai_api_key:
        raise RuntimeError(
            "OPENAI_API_KEY is required when stt_provider=openai. "
            "Set it in .env or the process environment."
        )
    if settings.stt_provider == "deepgram" and not settings.deepgram_api_key:
        raise RuntimeError(
            "DEEPGRAM_API_KEY is required when stt_provider=deepgram. "
            "Set it in .env or the process environment."
        )
    if settings.llm_provider == "anthropic" and not settings.anthropic_api_key:
        raise RuntimeError(
            "ANTHROPIC_API_KEY is required when llm_provider=anthropic. "
            "Set it in .env or the process environment."
        )
    if settings.llm_provider == "openai" and not settings.openai_api_key:
        raise RuntimeError(
            "OPENAI_API_KEY is required when llm_provider=openai. "
            "Set it in .env or the process environment."
        )
    return settings
