"""Unit tests for the Piper TTS provider.

End-to-end synthesis (calling a real `piper` binary with a real
voice model) is covered by on-host smoke tests, not the unit suite —
the model file is ~50 MB and the binary is not in the CI image.
These tests mock the subprocess layer so the wrapping contract
(setup errors, error stages, voice naming) is pinned everywhere.
"""

from __future__ import annotations

import subprocess
from pathlib import Path
from typing import Any

import pytest

from stackchan_sidecar.tts import PiperProvider, TTSError


def _patch_piper_available(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        "stackchan_sidecar.tts.piper.shutil.which", lambda _: "/usr/bin/piper"
    )


@pytest.mark.asyncio
async def test_setup_error_when_binary_missing(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setattr("stackchan_sidecar.tts.piper.shutil.which", lambda _: None)
    fake_model = tmp_path / "voice.onnx"
    fake_model.write_bytes(b"x")
    with pytest.raises(TTSError) as exc_info:
        await PiperProvider(model_path=fake_model).synthesize("hello")
    assert exc_info.value.stage == "setup"


@pytest.mark.asyncio
async def test_setup_error_when_model_missing(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    _patch_piper_available(monkeypatch)
    missing = tmp_path / "absent.onnx"
    with pytest.raises(TTSError) as exc_info:
        await PiperProvider(model_path=missing).synthesize("hello")
    assert exc_info.value.stage == "setup"
    assert str(missing) in exc_info.value.detail


@pytest.mark.asyncio
async def test_rejects_empty_text(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    _patch_piper_available(monkeypatch)
    fake_model = tmp_path / "voice.onnx"
    fake_model.write_bytes(b"x")
    provider = PiperProvider(model_path=fake_model)
    with pytest.raises(TTSError, match="empty text"):
        await provider.synthesize("")
    with pytest.raises(TTSError, match="empty text"):
        await provider.synthesize("   \n\t")


@pytest.mark.asyncio
async def test_subprocess_failure_surfaces_as_synthesize_error(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    _patch_piper_available(monkeypatch)
    fake_model = tmp_path / "voice.onnx"
    fake_model.write_bytes(b"x")

    def fake_run(*_a: Any, **_kw: Any) -> subprocess.CompletedProcess[str]:
        raise subprocess.CalledProcessError(
            returncode=1, cmd=["piper"], stderr="model load failed"
        )

    monkeypatch.setattr("stackchan_sidecar.tts.piper.subprocess.run", fake_run)
    with pytest.raises(TTSError) as exc_info:
        await PiperProvider(model_path=fake_model).synthesize("hello")
    assert exc_info.value.stage == "synthesize"
    assert "model load failed" in exc_info.value.detail


@pytest.mark.asyncio
async def test_subprocess_timeout_surfaces_as_synthesize_error(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    _patch_piper_available(monkeypatch)
    fake_model = tmp_path / "voice.onnx"
    fake_model.write_bytes(b"x")

    def fake_run(*_a: Any, **_kw: Any) -> subprocess.CompletedProcess[str]:
        raise subprocess.TimeoutExpired(cmd="piper", timeout=30.0)

    monkeypatch.setattr("stackchan_sidecar.tts.piper.subprocess.run", fake_run)
    with pytest.raises(TTSError) as exc_info:
        await PiperProvider(model_path=fake_model).synthesize("hello")
    assert exc_info.value.stage == "synthesize"
    assert "timed out" in exc_info.value.detail


@pytest.mark.asyncio
async def test_speaker_id_passes_through_to_subprocess(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    _patch_piper_available(monkeypatch)
    fake_model = tmp_path / "voice.onnx"
    fake_model.write_bytes(b"x")

    captured_args: list[list[str]] = []

    def fake_run(args: list[str], **_kw: Any) -> subprocess.CompletedProcess[str]:
        captured_args.append(args)
        # Pretend it ran but produced no WAV so we surface as a
        # transcode-stage error rather than success (we have no WAV
        # to read back).
        raise subprocess.CalledProcessError(returncode=0, cmd=args, stderr="")

    monkeypatch.setattr("stackchan_sidecar.tts.piper.subprocess.run", fake_run)
    with pytest.raises(TTSError):
        await PiperProvider(model_path=fake_model, speaker_id=3).synthesize("hi")
    assert captured_args, "subprocess.run should have been called"
    args = captured_args[0]
    assert "--speaker" in args
    assert "3" in args
    # And the model path is forwarded.
    assert "--model" in args
    assert str(fake_model) in args
