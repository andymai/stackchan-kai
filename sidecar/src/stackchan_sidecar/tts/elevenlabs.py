"""ElevenLabs TTS provider.

Setup needs an ``xi-api-key``, a ``voice_id``, and a Starter-tier or
higher account — the free tier only emits MP3, and this provider
requests raw ``pcm_16000`` so the firmware audio pipeline doesn't
need an MP3 decoder. The endpoint returns the wire-shape PCM
unchanged.
"""

from __future__ import annotations

import logging

import httpx

from .errors import TTSError
from .protocol import PCM_SAMPLE_RATE_HZ, TTSResult

_LOG = logging.getLogger("stackchan_sidecar.tts.elevenlabs")

_ENDPOINT_TEMPLATE = "https://api.elevenlabs.io/v1/text-to-speech/{voice_id}"
# Latency-optimised model. Swap for `eleven_multilingual_v2` for
# non-English voices.
_DEFAULT_MODEL = "eleven_turbo_v2_5"
_DEFAULT_TIMEOUT_S = 30.0


class ElevenLabsProvider:
    """POSTs to ElevenLabs and asks for raw 16 kHz mono s16 LE PCM."""

    name: str = "elevenlabs"

    def __init__(
        self,
        *,
        api_key: str,
        voice_id: str,
        model_id: str = _DEFAULT_MODEL,
        timeout_seconds: float = _DEFAULT_TIMEOUT_S,
    ) -> None:
        self._api_key = api_key
        self._voice_id = voice_id
        self._model_id = model_id
        self._timeout_seconds = timeout_seconds

    async def synthesize(self, text: str) -> TTSResult:
        if not self._api_key:
            raise TTSError("setup", "elevenlabs_api_key not configured")
        if not text.strip():
            raise TTSError("synthesize", "empty text")
        endpoint = _ENDPOINT_TEMPLATE.format(voice_id=self._voice_id)
        headers = {
            "xi-api-key": self._api_key,
            "Accept": "audio/pcm",
            "Content-Type": "application/json",
        }
        params = {"output_format": f"pcm_{PCM_SAMPLE_RATE_HZ}"}
        body = {"text": text, "model_id": self._model_id}
        try:
            async with httpx.AsyncClient(timeout=self._timeout_seconds) as client:
                response = await client.post(endpoint, headers=headers, params=params, json=body)
        except httpx.HTTPError as e:
            raise TTSError("synthesize", f"http error: {e}") from e
        if response.status_code != 200:
            raise TTSError(
                "synthesize",
                f"elevenlabs returned {response.status_code}: {response.text.strip()[:120]}",
            )
        pcm = response.content
        if not pcm:
            raise TTSError("synthesize", "elevenlabs produced empty audio")
        if len(pcm) % 2 != 0:
            # The response is supposed to be s16, so byte count must
            # be even. An odd count means the API returned a wrapper
            # we don't recognise (mp3, ogg) — treat as transcode-stage.
            raise TTSError("transcode", f"elevenlabs PCM odd-length: {len(pcm)} bytes")
        _LOG.debug(
            "elevenlabs synthesized %d bytes (voice=%s, model=%s)",
            len(pcm),
            self._voice_id,
            self._model_id,
        )
        return TTSResult(pcm=pcm, provider=self.name, voice=self._voice_id)
