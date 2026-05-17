from fastapi.testclient import TestClient

_AUDIO_CT = "audio/L16;rate=16000;channels=1"


def test_missing_token_returns_401(client: TestClient, pcm_payload: bytes) -> None:
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={"Content-Type": _AUDIO_CT},
    )
    assert r.status_code == 401
    assert r.headers.get("www-authenticate") == "Bearer"


def test_empty_authorization_returns_401(client: TestClient, pcm_payload: bytes) -> None:
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={"Authorization": "", "Content-Type": _AUDIO_CT},
    )
    assert r.status_code == 401


def test_malformed_scheme_returns_401(client: TestClient, pcm_payload: bytes) -> None:
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={"Authorization": "Basic abc123", "Content-Type": _AUDIO_CT},
    )
    assert r.status_code == 401


def test_wrong_token_returns_401(client: TestClient, pcm_payload: bytes) -> None:
    r = client.post(
        "/v1/listen",
        content=pcm_payload,
        headers={"Authorization": "Bearer not-the-token", "Content-Type": _AUDIO_CT},
    )
    assert r.status_code == 401


def test_healthz_does_not_require_auth(client: TestClient) -> None:
    r = client.get("/healthz")
    assert r.status_code == 200
