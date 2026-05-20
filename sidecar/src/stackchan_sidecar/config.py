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
    # `espeak_ng` is the zero-config default — ships in every Linux/
    # macOS distro and produces audible (if synthetic) output without
    # an API key or model file. `piper` and `elevenlabs` are quality
    # opt-ins; see docs/sidecar.md for the setup steps each needs.
    tts_provider: Literal["espeak_ng", "piper", "elevenlabs"] = "espeak_ng"
    ollama_host: str = "http://localhost:11434"

    stt_timeout_seconds: float = Field(default=10.0, gt=0.0)
    llm_timeout_seconds: float = Field(default=20.0, gt=0.0)
    total_timeout_seconds: float = Field(default=30.0, gt=0.0)
    stt_max_attempts: int = Field(default=2, ge=1)
    llm_max_attempts: int = Field(default=2, ge=1)
    retry_initial_backoff_seconds: float = Field(default=0.5, ge=0.0)

    # TTS knobs. Audio is cached in memory and the firmware fetches it
    # within seconds; 60 s TTL gives generous headroom for a slow
    # firmware roundtrip without holding bytes forever. Cache capacity
    # bounds the worst-case memory footprint (~256 KB * 32 ~= 8 MB).
    tts_audio_ttl_seconds: float = Field(default=60.0, gt=0.0)
    tts_audio_cache_capacity: int = Field(default=32, ge=1)
    # eSpeak-NG voice + speech rate; both ignored for other providers.
    espeak_voice: str = "en"
    espeak_wpm: int = Field(default=175, ge=80, le=450)

    bearer_token: str = Field(default="", alias="SIDECAR_BEARER_TOKEN")
    anthropic_api_key: str = Field(default="", alias="ANTHROPIC_API_KEY")
    openai_api_key: str = Field(default="", alias="OPENAI_API_KEY")
    deepgram_api_key: str = Field(default="", alias="DEEPGRAM_API_KEY")

    firmware_url: str = Field(default="http://stackchan.local", alias="STACKCHAN_FIRMWARE_URL")
    firmware_token: str = Field(default="", alias="STACKCHAN_FIRMWARE_TOKEN")
    companion_enabled: bool = Field(default=True, alias="SIDECAR_COMPANION_ENABLED")


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
