"""``kws-eval`` CLI — run a trained ``.tflite`` against a WAV and
report detection scores.

Useful for tuning ``behavior.wake_word_threshold`` without
reflashing: capture a representative WAV via ``kws-record``, run
the model offline, and pick a threshold where positives trigger
cleanly and negatives stay below.

Requires the ``eval`` extra: ``uv sync --extra eval`` (pulls in
``ai-edge-litert``).
"""

from __future__ import annotations

import argparse
import logging
import sys
import wave
from collections.abc import Sequence
from pathlib import Path

import numpy as np

from kws_trainer.features import SAMPLE_RATE_HZ
from kws_trainer.inference import evaluate_pcm, load_interpreter

_LOG = logging.getLogger("kws_trainer.cli.eval")


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="kws-eval",
        description=(
            "Run a trained .tflite wake-word model against a WAV "
            "and report per-frame scores + peak."
        ),
    )
    p.add_argument(
        "--model",
        required=True,
        type=Path,
        help="Path to the streaming-quantized .tflite model.",
    )
    p.add_argument(
        "--input",
        required=True,
        type=Path,
        help="WAV file to evaluate (16 kHz mono s16 LE — the same shape kws-record produces).",
    )
    p.add_argument(
        "--threshold",
        type=float,
        default=0.5,
        help="Score threshold for the 'triggered?' summary line (default: 0.5).",
    )
    p.add_argument(
        "--print-scores",
        action="store_true",
        help="Print the per-frame score timeline as `frame=score` lines. "
        "Off by default — long clips produce hundreds of rows.",
    )
    p.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="Log at INFO level (default: WARNING).",
    )
    return p


def _configure_logging(verbose: bool) -> None:
    logging.basicConfig(
        level=logging.INFO if verbose else logging.WARNING,
        format="%(message)s",
    )


def load_wav_as_int16(path: Path) -> np.ndarray:
    """Read a 16 kHz mono s16 LE WAV into a 1-D ``int16`` numpy
    array. Raises ``ValueError`` on any shape mismatch — the trained
    model expects this exact wire shape and a stereo/22kHz input
    would silently produce garbage scores."""
    with wave.open(str(path), "rb") as w:
        if w.getnchannels() != 1:
            raise ValueError(f"{path}: expected mono, got {w.getnchannels()} channels")
        if w.getsampwidth() != 2:
            raise ValueError(f"{path}: expected 16-bit samples, got {w.getsampwidth() * 8}-bit")
        if w.getframerate() != SAMPLE_RATE_HZ:
            raise ValueError(f"{path}: expected {SAMPLE_RATE_HZ} Hz, got {w.getframerate()} Hz")
        n_frames = w.getnframes()
        raw = w.readframes(n_frames)
    return np.frombuffer(raw, dtype=np.int16)


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    _configure_logging(args.verbose)
    if not args.model.is_file():
        print(f"error: --model {args.model} does not exist", file=sys.stderr)
        return 2
    if not args.input.is_file():
        print(f"error: --input {args.input} does not exist", file=sys.stderr)
        return 2

    try:
        interpreter = load_interpreter(args.model)
    except RuntimeError as e:
        print(f"error: {e}", file=sys.stderr)
        return 3

    try:
        pcm = load_wav_as_int16(args.input)
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    if pcm.size == 0:
        print(f"error: {args.input} contains zero samples", file=sys.stderr)
        return 2

    result = evaluate_pcm(interpreter, pcm, threshold=args.threshold)

    duration_s = pcm.size / SAMPLE_RATE_HZ
    print(f"clip: {args.input.name}  duration: {duration_s:.2f}s  frames: {result.n_frames}")
    print(
        f"peak score: {result.peak_score:.4f} at frame {result.peak_frame_index} "
        f"(threshold {result.threshold:.2f})"
    )
    print(f"triggered: {'YES' if result.triggered else 'no'}")
    if args.print_scores:
        for i, s in enumerate(result.per_frame_scores):
            print(f"  frame={i:04d} score={s:.4f}")
    return 0 if result.triggered else 1


if __name__ == "__main__":
    sys.exit(main())
