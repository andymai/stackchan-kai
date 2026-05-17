from fastapi.testclient import TestClient

_AUDIO_CT = "audio/L16;rate=16000;channels=1"
_MAX_BODY_BYTES = 30 * 16000 * 2 + 1024


def test_oversize_declared_length_rejected_before_read(
    client: TestClient,
    auth_headers: dict[str, str],
) -> None:
    declared = _MAX_BODY_BYTES + 1
    r = client.post(
        "/v1/listen",
        content=b"",
        headers={
            **auth_headers,
            "Content-Type": _AUDIO_CT,
            "Content-Length": str(declared),
        },
    )
    assert r.status_code == 413


def test_garbage_content_length_rejected(
    client: TestClient,
    auth_headers: dict[str, str],
) -> None:
    r = client.post(
        "/v1/listen",
        content=b"",
        headers={
            **auth_headers,
            "Content-Type": _AUDIO_CT,
            "Content-Length": "not-a-number",
        },
    )
    assert r.status_code == 400
