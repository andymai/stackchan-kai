"""Tests for the dataset assembly module.

Synthesise small WAVs in tmp_path rather than depending on captured
recordings — keeps the suite self-contained and fast.
"""

from __future__ import annotations

import json
import wave
from pathlib import Path

import pytest

from kws_trainer.dataset import (
    InvalidRecording,
    Recording,
    inspect_wav,
    label_from_filename,
    scan_directory,
    split_recordings,
    summarize_splits,
    write_manifest,
)
from kws_trainer.recorder import SAMPLE_RATE_HZ


def _write_wav(path: Path, duration_seconds: float, *, rate: int = SAMPLE_RATE_HZ) -> None:
    """Synthesise a silent mono s16 WAV of the requested duration."""
    path.parent.mkdir(parents=True, exist_ok=True)
    n_samples = round(duration_seconds * rate)
    pcm = b"\x00\x00" * n_samples
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(pcm)


def _write_stereo_wav(path: Path, duration_seconds: float) -> None:
    n_samples = round(duration_seconds * SAMPLE_RATE_HZ)
    # Interleaved L/R s16 silence.
    pcm = b"\x00\x00\x00\x00" * n_samples
    with wave.open(str(path), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SAMPLE_RATE_HZ)
        w.writeframes(pcm)


def test_label_from_filename_recognises_canonical_prefixes(tmp_path: Path) -> None:
    assert label_from_filename(tmp_path / "positive-001.wav") == "positive"
    assert label_from_filename(tmp_path / "negative-042.wav") == "negative"
    assert label_from_filename(tmp_path / "silence-007.wav") == "silence"


def test_label_from_filename_noise_alias_maps_to_negative(tmp_path: Path) -> None:
    # `noise-` and `negative-` should both classify as negative so
    # operators using either convention land in the same bucket.
    assert label_from_filename(tmp_path / "noise-005.wav") == "negative"


def test_label_from_filename_is_case_insensitive(tmp_path: Path) -> None:
    assert label_from_filename(tmp_path / "Positive-001.wav") == "positive"
    assert label_from_filename(tmp_path / "NEGATIVE-042.wav") == "negative"


def test_label_from_filename_rejects_unknown_prefix(tmp_path: Path) -> None:
    assert label_from_filename(tmp_path / "random-001.wav") is None
    assert label_from_filename(tmp_path / "no_dash.wav") is None


def test_inspect_wav_accepts_well_shaped_audio(tmp_path: Path) -> None:
    wav = tmp_path / "positive-001.wav"
    _write_wav(wav, duration_seconds=2.0)
    duration, reason = inspect_wav(wav)
    assert reason is None
    assert abs(duration - 2.0) < 0.01


def test_inspect_wav_rejects_wrong_sample_rate(tmp_path: Path) -> None:
    wav = tmp_path / "positive-001.wav"
    _write_wav(wav, duration_seconds=2.0, rate=22050)
    _duration, reason = inspect_wav(wav)
    assert reason is not None
    assert "sample_rate" in reason


def test_inspect_wav_rejects_stereo(tmp_path: Path) -> None:
    wav = tmp_path / "positive-001.wav"
    _write_stereo_wav(wav, duration_seconds=2.0)
    _duration, reason = inspect_wav(wav)
    assert reason is not None
    assert "channels" in reason


def test_inspect_wav_rejects_too_short(tmp_path: Path) -> None:
    wav = tmp_path / "positive-001.wav"
    _write_wav(wav, duration_seconds=0.1)
    duration, reason = inspect_wav(wav)
    assert reason is not None
    assert "too short" in reason
    # Duration is still reported in the error path so operators can
    # see how far off the recording was.
    assert duration > 0


def test_inspect_wav_handles_missing_file(tmp_path: Path) -> None:
    _duration, reason = inspect_wav(tmp_path / "missing.wav")
    assert reason is not None
    assert "unreadable" in reason


def test_scan_directory_classifies_and_collects(tmp_path: Path) -> None:
    _write_wav(tmp_path / "positive-001.wav", 2.0)
    _write_wav(tmp_path / "negative-001.wav", 2.0)
    _write_wav(tmp_path / "silence-001.wav", 2.0)
    # Subdirectory: rglob picks it up.
    _write_wav(tmp_path / "nested" / "positive-002.wav", 2.0)
    # Unknown prefix → goes to invalid.
    _write_wav(tmp_path / "random-001.wav", 2.0)
    # Wrong shape → goes to invalid.
    _write_stereo_wav(tmp_path / "negative-bad.wav", 2.0)

    result = scan_directory(tmp_path)
    assert len(result.recordings) == 4
    assert result.label_counts == {"positive": 2, "negative": 1, "silence": 1}
    assert len(result.invalid) == 2
    invalid_names = {inv.path.name for inv in result.invalid}
    assert invalid_names == {"random-001.wav", "negative-bad.wav"}


def test_scan_directory_returns_empty_on_no_matches(tmp_path: Path) -> None:
    result = scan_directory(tmp_path)
    assert result.recordings == []
    assert result.invalid == []


def _mk_recordings(counts: dict[str, int]) -> list[Recording]:
    out: list[Recording] = []
    for label, n in counts.items():
        for i in range(n):
            out.append(
                Recording(
                    path=Path(f"{label}-{i:03d}.wav"),
                    label=label,  # type: ignore[arg-type]
                    duration_seconds=2.0,
                )
            )
    return out


def test_split_recordings_is_deterministic_for_same_seed() -> None:
    recordings = _mk_recordings({"positive": 50, "negative": 200})
    a = split_recordings(recordings, seed=42)
    b = split_recordings(recordings, seed=42)
    assert [r.path for r in a.train] == [r.path for r in b.train]
    assert [r.path for r in a.val] == [r.path for r in b.val]
    assert [r.path for r in a.test] == [r.path for r in b.test]


def test_split_recordings_changes_with_seed() -> None:
    recordings = _mk_recordings({"positive": 50, "negative": 200})
    a = split_recordings(recordings, seed=1)
    b = split_recordings(recordings, seed=2)
    assert [r.path for r in a.train] != [r.path for r in b.train]


def test_split_recordings_stratifies_per_label() -> None:
    # Skewed bucket sizes — 50 positive, 200 negative, 10 silence.
    # The val/test fractions of 0.15 each should pull ~7-8 positives
    # AND ~30 negatives AND ~1-2 silences, not just "15% of the
    # whole pool" which would crowd silence out entirely.
    recordings = _mk_recordings({"positive": 50, "negative": 200, "silence": 10})
    splits = split_recordings(recordings, val_fraction=0.15, test_fraction=0.15)
    total = len(splits.train) + len(splits.val) + len(splits.test)
    assert total == 260
    # Each split contains at least one of every label (stratification).
    train_labels = {r.label for r in splits.train}
    val_labels = {r.label for r in splits.val}
    test_labels = {r.label for r in splits.test}
    assert train_labels == {"positive", "negative", "silence"}
    assert val_labels == {"positive", "negative", "silence"}
    assert test_labels == {"positive", "negative", "silence"}


def test_split_recordings_rejects_oversized_fractions() -> None:
    recordings = _mk_recordings({"positive": 10})
    with pytest.raises(ValueError, match="leave no train data"):
        split_recordings(recordings, val_fraction=0.6, test_fraction=0.5)


def test_summarize_splits_counts_per_label() -> None:
    recordings = _mk_recordings({"positive": 10, "negative": 40})
    splits = split_recordings(recordings, val_fraction=0.1, test_fraction=0.1)
    summary = summarize_splits(splits)
    # Sum across all splits == original counts.
    total_positive = sum(s.get("positive", 0) for s in summary.values())
    total_negative = sum(s.get("negative", 0) for s in summary.values())
    assert total_positive == 10
    assert total_negative == 40


def test_write_manifest_round_trips_to_json(tmp_path: Path) -> None:
    base = tmp_path / "samples"
    base.mkdir()
    recordings = _mk_recordings({"positive": 6, "negative": 6})
    # Point each path at a real file under base so to_manifest_entry's
    # relative_to call lands somewhere reasonable.
    placed: list[Recording] = []
    for r in recordings:
        target = base / r.path.name
        _write_wav(target, 2.0)
        placed.append(Recording(path=target, label=r.label, duration_seconds=r.duration_seconds))
    splits = split_recordings(placed, val_fraction=0.0, test_fraction=0.0)

    manifest_path = tmp_path / "manifest.json"
    write_manifest(splits, manifest_path, base=base, split_seed=42)

    loaded = json.loads(manifest_path.read_text())
    assert loaded["schema_version"] == 1
    assert loaded["split_seed"] == 42
    # All entries landed in train (val/test fractions were zero).
    assert len(loaded["splits"]["train"]) == 12
    # Paths are relative to base, not absolute.
    assert all("/" not in entry["path"] for entry in loaded["splits"]["train"])
    # Summary mirrors split counts.
    assert loaded["summary"]["train"] == {"positive": 6, "negative": 6}


def test_invalid_recording_dataclass_roundtrips() -> None:
    inv = InvalidRecording(path=Path("/x.wav"), reason="bad")
    assert inv.path == Path("/x.wav")
    assert inv.reason == "bad"
