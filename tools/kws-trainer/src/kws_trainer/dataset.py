"""Dataset assembly: directory of WAVs → labelled train/val/test split.

The recorder (`kws_trainer.recorder`) writes WAVs named with a
label prefix — ``positive-001.wav``, ``negative-042.wav``,
``silence-007.wav``. This module turns a directory of those WAVs
into a deterministic train/val/test split with a JSON manifest
the trainer can consume.

Labels are derived from the filename prefix only — no audio
classification. That's deliberate: operators curate the recordings
during capture (one PTT = one labelled clip), and a downstream tool
"auto-detecting" the wake phrase would risk leaking the training
target into the labelling.

Wire shape (input WAVs):
- 16 kHz mono s16 LE — matches `kws_trainer.recorder.SAMPLE_RATE_HZ`.
- Sample width must be 2 bytes; channels 1; frame rate 16000.
- Files violating the shape are surfaced in
  [`scan_directory`]'s `invalid` list rather than silently dropped.

Manifest layout (output JSON):

.. code-block:: json

    {
      "schema_version": 1,
      "split_seed": 42,
      "splits": {
        "train": [
          {"path": "positive-001.wav", "label": "positive",
           "duration_seconds": 5.0}
        ],
        "val": [...],
        "test": [...]
      },
      "summary": {"train": {"positive": 35, "negative": 140},
                  "val": {...}, "test": {...}}
    }
"""

from __future__ import annotations

import json
import logging
import random
import wave
from collections import Counter, defaultdict
from collections.abc import Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from .recorder import CHANNELS, SAMPLE_RATE_HZ, SAMPLE_WIDTH_BYTES

_LOG = logging.getLogger("kws_trainer.dataset")

Label = Literal["positive", "negative", "silence"]

_LABEL_PREFIXES: dict[str, Label] = {
    "positive": "positive",
    "negative": "negative",
    "silence": "silence",
    # `noise-` is an alias for `negative-` — some operators prefer
    # the more specific term when recording ambient backgrounds.
    "noise": "negative",
}

# Minimum WAV duration to count as a usable training sample. Anything
# shorter is almost certainly an aborted recording (op cancelled
# before the trigger window finished); microWakeWord's training
# pipeline doesn't fail on these, but they bias the loss and aren't
# worth carrying.
_MIN_DURATION_SECONDS = 0.5


@dataclass
class Recording:
    """One labelled training sample on disk."""

    path: Path
    label: Label
    duration_seconds: float

    def to_manifest_entry(self, base: Path) -> dict[str, object]:
        """Convert to the dict shape persisted in the manifest JSON.
        Stores the path relative to ``base`` so the manifest stays
        portable across hosts."""
        try:
            rel = self.path.relative_to(base)
        except ValueError:
            # Path isn't under base — keep it absolute. Operators who
            # mix sources will see the full path in the manifest.
            rel = self.path
        return {
            "path": str(rel),
            "label": self.label,
            "duration_seconds": round(self.duration_seconds, 3),
        }


@dataclass
class InvalidRecording:
    """A WAV that scanned but was rejected (wrong shape, bad name,
    too short). Surfaced so operators can fix the input rather than
    quietly dropping samples."""

    path: Path
    reason: str


@dataclass
class ScanResult:
    """Aggregate output of [`scan_directory`]."""

    recordings: list[Recording]
    invalid: list[InvalidRecording]

    @property
    def label_counts(self) -> dict[Label, int]:
        counts: Counter[Label] = Counter()
        for r in self.recordings:
            counts[r.label] += 1
        return dict(counts)


@dataclass
class Splits:
    """Deterministic train/val/test split of recordings."""

    train: list[Recording]
    val: list[Recording]
    test: list[Recording]

    @property
    def all_recordings(self) -> Iterable[Recording]:
        yield from self.train
        yield from self.val
        yield from self.test


def label_from_filename(path: Path) -> Label | None:
    """Derive the [`Label`] from ``path``'s stem prefix
    (``positive-001`` → ``"positive"``). Returns ``None`` on any
    unknown prefix so callers can surface it in [`InvalidRecording`]
    rather than crashing.
    """
    stem = path.stem
    head, _, _ = stem.partition("-")
    if not head:
        return None
    return _LABEL_PREFIXES.get(head.lower())


def inspect_wav(path: Path) -> tuple[float, str | None]:
    """Open ``path`` and return ``(duration_seconds, reason_or_none)``.

    ``reason_or_none`` is ``None`` on a well-shaped WAV; otherwise a
    short string describing the violation. The duration is best-effort
    in the error path — `0.0` when the file couldn't be opened at all.
    """
    try:
        with wave.open(str(path), "rb") as w:
            channels = w.getnchannels()
            width = w.getsampwidth()
            rate = w.getframerate()
            n_frames = w.getnframes()
    except (wave.Error, FileNotFoundError, OSError) as e:
        return 0.0, f"unreadable WAV: {e}"
    duration = n_frames / rate if rate else 0.0
    if channels != CHANNELS:
        return duration, f"channels={channels} != expected {CHANNELS}"
    if width != SAMPLE_WIDTH_BYTES:
        return duration, f"sample_width={width} != expected {SAMPLE_WIDTH_BYTES}"
    if rate != SAMPLE_RATE_HZ:
        return duration, f"sample_rate={rate} != expected {SAMPLE_RATE_HZ}"
    if duration < _MIN_DURATION_SECONDS:
        return duration, f"too short: {duration:.2f}s < {_MIN_DURATION_SECONDS}s"
    return duration, None


def scan_directory(root: Path, *, pattern: str = "*.wav") -> ScanResult:
    """Walk ``root`` for files matching ``pattern``, classify by
    filename prefix, validate WAV shape, and return the partition.

    Iterates with `Path.rglob` so nested operator layouts
    (``samples/2026-05-20/positive-001.wav``) are picked up.
    """
    recordings: list[Recording] = []
    invalid: list[InvalidRecording] = []
    for path in sorted(root.rglob(pattern)):
        if not path.is_file():
            continue
        label = label_from_filename(path)
        if label is None:
            invalid.append(
                InvalidRecording(path=path, reason=f"unknown label prefix in {path.name}")
            )
            continue
        duration, reason = inspect_wav(path)
        if reason is not None:
            invalid.append(InvalidRecording(path=path, reason=reason))
            continue
        recordings.append(Recording(path=path, label=label, duration_seconds=duration))
    return ScanResult(recordings=recordings, invalid=invalid)


def split_recordings(
    recordings: Sequence[Recording],
    *,
    val_fraction: float = 0.15,
    test_fraction: float = 0.15,
    seed: int = 42,
) -> Splits:
    """Deterministic train/val/test split, stratified per label.

    The split is independent per label so a small ``silence`` bucket
    isn't accidentally crowded out of the validation set by the much
    larger ``negative`` bucket. ``seed`` controls the shuffle so
    re-runs against the same input directory are bit-stable.
    """
    if not 0.0 <= val_fraction < 1.0:
        raise ValueError(f"val_fraction must be in [0, 1): {val_fraction}")
    if not 0.0 <= test_fraction < 1.0:
        raise ValueError(f"test_fraction must be in [0, 1): {test_fraction}")
    if val_fraction + test_fraction >= 1.0:
        raise ValueError(
            f"val + test fractions ({val_fraction + test_fraction}) leave no train data"
        )

    by_label: defaultdict[Label, list[Recording]] = defaultdict(list)
    for r in recordings:
        by_label[r.label].append(r)

    rng = random.Random(seed)
    train: list[Recording] = []
    val: list[Recording] = []
    test: list[Recording] = []
    for label in sorted(by_label.keys()):
        bucket = list(by_label[label])
        rng.shuffle(bucket)
        n_total = len(bucket)
        n_test = round(n_total * test_fraction)
        n_val = round(n_total * val_fraction)
        # Take from the front so the same seed + dataset always
        # produces the same partition assignment per file.
        test.extend(bucket[:n_test])
        val.extend(bucket[n_test : n_test + n_val])
        train.extend(bucket[n_test + n_val :])
    return Splits(train=train, val=val, test=test)


def summarize_splits(splits: Splits) -> dict[str, dict[Label, int]]:
    """Build the manifest's ``summary`` section: per-split label
    counts. Empty buckets are omitted so operators can spot a missing
    label at a glance."""
    out: dict[str, dict[Label, int]] = {}
    for name, bucket in (
        ("train", splits.train),
        ("val", splits.val),
        ("test", splits.test),
    ):
        counts: Counter[Label] = Counter()
        for r in bucket:
            counts[r.label] += 1
        out[name] = dict(counts)
    return out


def write_manifest(
    splits: Splits,
    output_path: Path,
    *,
    base: Path,
    split_seed: int,
) -> None:
    """Persist ``splits`` to ``output_path`` as JSON.

    ``base`` is the WAV-input root; paths in the manifest are
    rendered relative to it so the manifest can move alongside the
    sample directory across hosts.
    """
    manifest: dict[str, object] = {
        "schema_version": 1,
        "split_seed": split_seed,
        "splits": {
            "train": [r.to_manifest_entry(base) for r in splits.train],
            "val": [r.to_manifest_entry(base) for r in splits.val],
            "test": [r.to_manifest_entry(base) for r in splits.test],
        },
        "summary": summarize_splits(splits),
    }
    output_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    _LOG.info(
        "wrote manifest %s (train=%d val=%d test=%d)",
        output_path,
        len(splits.train),
        len(splits.val),
        len(splits.test),
    )


__all__ = [
    "InvalidRecording",
    "Label",
    "Recording",
    "ScanResult",
    "Splits",
    "inspect_wav",
    "label_from_filename",
    "scan_directory",
    "split_recordings",
    "summarize_splits",
    "write_manifest",
]
