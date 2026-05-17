from pathlib import Path

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

    bearer_token: str = Field(default="", alias="SIDECAR_BEARER_TOKEN")
    anthropic_api_key: str = Field(default="", alias="ANTHROPIC_API_KEY")


def load_settings() -> Settings:
    settings = Settings()
    if not settings.bearer_token:
        raise RuntimeError(
            "SIDECAR_BEARER_TOKEN is not set. Refusing to start without a bearer "
            "token — set it in .env or the process environment."
        )
    if not settings.anthropic_api_key:
        raise RuntimeError(
            "ANTHROPIC_API_KEY is not set. Refusing to start without an Anthropic "
            "API key — set it in .env or the process environment."
        )
    return settings
