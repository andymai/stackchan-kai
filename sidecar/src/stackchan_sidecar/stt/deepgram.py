import httpx

_ENDPOINT = "https://api.deepgram.com/v1/listen"


class DeepgramSTT:
    def __init__(
        self,
        api_key: str,
        model: str = "nova-2",
        language: str = "en",
        timeout_seconds: float = 30.0,
    ) -> None:
        self._api_key = api_key
        self._model = model
        self._language = language
        self._timeout_seconds = timeout_seconds

    async def transcribe(self, pcm: bytes, sample_rate: int = 16000) -> str:
        if not pcm:
            return ""
        headers = {
            "Authorization": f"Token {self._api_key}",
            "Content-Type": f"audio/L16;rate={sample_rate};channels=1",
        }
        params = {"model": self._model, "language": self._language}
        async with httpx.AsyncClient(timeout=self._timeout_seconds) as client:
            response = await client.post(
                _ENDPOINT,
                headers=headers,
                params=params,
                content=pcm,
            )
            response.raise_for_status()
            payload = response.json()

        channels = (payload.get("results") or {}).get("channels") or []
        if not channels:
            return ""
        alternatives = channels[0].get("alternatives") or []
        if not alternatives:
            return ""
        return str(alternatives[0].get("transcript", "")).strip()
