import uvicorn

from .app import create_app
from .config import Settings, load_settings
from .llm import LLMProvider
from .logging import setup_logging
from .session_store import SessionStore
from .stt import STTProvider
from .tts import TTSProvider


def _build_stt(settings: Settings) -> STTProvider:
    if settings.stt_provider == "openai":
        from .stt.openai_whisper import OpenAIWhisperSTT

        return OpenAIWhisperSTT(api_key=settings.openai_api_key, model=settings.stt_model)
    if settings.stt_provider == "deepgram":
        from .stt.deepgram import DeepgramSTT

        return DeepgramSTT(api_key=settings.deepgram_api_key, model=settings.stt_model)
    from .stt.faster_whisper import FasterWhisperSTT

    return FasterWhisperSTT(model_name=settings.stt_model)


def _build_llm(settings: Settings) -> LLMProvider:
    if settings.llm_provider == "openai":
        from .llm.openai import OpenAILLM

        return OpenAILLM(api_key=settings.openai_api_key, model=settings.llm_model)
    if settings.llm_provider == "ollama":
        from .llm.ollama import OllamaLLM

        return OllamaLLM(host=settings.ollama_host, model=settings.llm_model)
    from .llm.anthropic import AnthropicLLM

    return AnthropicLLM(api_key=settings.anthropic_api_key, model=settings.llm_model)


def _build_tts(settings: Settings) -> TTSProvider:
    if settings.tts_provider == "piper":
        from .tts import PiperProvider

        # load_settings() already enforces piper_model_path != None
        # when tts_provider=piper, so the assertion is a type narrow,
        # not a runtime guard.
        assert settings.piper_model_path is not None
        return PiperProvider(
            model_path=settings.piper_model_path,
            speaker_id=settings.piper_speaker_id,
        )
    if settings.tts_provider == "elevenlabs":
        import logging

        logging.getLogger("stackchan_sidecar").warning(
            "tts_provider=elevenlabs requested but not implemented yet; "
            "falling back to espeak_ng.",
        )
    from .tts import EspeakProvider

    return EspeakProvider(voice=settings.espeak_voice, wpm=settings.espeak_wpm)


def main() -> None:
    settings = load_settings()
    setup_logging(settings.log_level)

    stt = _build_stt(settings)
    llm = _build_llm(settings)
    tts = _build_tts(settings)
    session_store = SessionStore()
    app = create_app(settings, stt, llm, session_store, tts=tts)

    uvicorn.run(app, host=settings.host, port=settings.port, log_config=None)


if __name__ == "__main__":
    main()
