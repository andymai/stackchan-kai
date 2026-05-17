import uvicorn

from .app import create_app
from .config import load_settings
from .llm.anthropic import AnthropicLLM
from .logging import setup_logging
from .stt.faster_whisper import FasterWhisperSTT


def main() -> None:
    settings = load_settings()
    setup_logging(settings.log_level)

    stt = FasterWhisperSTT(model_name=settings.stt_model)
    llm = AnthropicLLM(api_key=settings.anthropic_api_key, model=settings.llm_model)
    app = create_app(settings, stt, llm)

    uvicorn.run(app, host=settings.host, port=settings.port, log_config=None)


if __name__ == "__main__":
    main()
