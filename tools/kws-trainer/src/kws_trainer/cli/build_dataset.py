"""``kws-build-dataset`` CLI — turn a directory of labelled WAVs into
a train/val/test split manifest.

Operator workflow:

1. Capture ~50 ``positive-*.wav`` and ~200 ``negative-*.wav``
   recordings via ``kws-record`` (or copy in equivalents from
   another source).
2. Drop them all into one directory (nesting is fine —
   subdirectories are walked).
3. Run::

       uv run kws-build-dataset --input samples/ --output manifest.json

4. Inspect the manifest summary; re-record any classes that are
   under-represented and re-run.
"""

from __future__ import annotations

import argparse
import logging
import sys
from collections.abc import Sequence
from pathlib import Path

from kws_trainer.dataset import (
    scan_directory,
    split_recordings,
    summarize_splits,
    write_manifest,
)

_LOG = logging.getLogger("kws_trainer.cli.build_dataset")


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="kws-build-dataset",
        description=(
            "Walk a directory of labelled WAVs and emit a deterministic "
            "train/val/test split manifest."
        ),
    )
    p.add_argument(
        "--input",
        required=True,
        type=Path,
        help="Directory containing labelled WAVs. Files must be named "
        "`<label>-<NNN>.wav` where `<label>` is one of "
        "{positive, negative, noise, silence}. Subdirectories are walked.",
    )
    p.add_argument(
        "--output",
        required=True,
        type=Path,
        help="Manifest JSON to write. Sample paths are recorded relative "
        "to --input so the manifest can move alongside the directory.",
    )
    p.add_argument(
        "--val-fraction",
        type=float,
        default=0.15,
        help="Fraction of each label's samples to hold out for validation (default: 0.15).",
    )
    p.add_argument(
        "--test-fraction",
        type=float,
        default=0.15,
        help="Fraction of each label's samples to hold out for test (default: 0.15).",
    )
    p.add_argument(
        "--seed",
        type=int,
        default=42,
        help="Random seed for the deterministic split shuffle (default: 42). "
        "Same seed + same input directory → bit-stable manifest.",
    )
    p.add_argument(
        "--strict",
        action="store_true",
        help="Exit with code 2 if any input WAV is rejected (wrong shape, "
        "bad name, too short). Default: log + continue.",
    )
    p.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="Log per-file scan decisions at INFO level.",
    )
    return p


def _configure_logging(verbose: bool) -> None:
    logging.basicConfig(
        level=logging.INFO if verbose else logging.WARNING,
        format="%(message)s",
    )


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    _configure_logging(args.verbose)
    input_dir: Path = args.input
    if not input_dir.is_dir():
        print(f"error: --input {input_dir} is not a directory", file=sys.stderr)
        return 2

    scan = scan_directory(input_dir)
    if not scan.recordings:
        print(
            f"error: no usable WAVs found under {input_dir}",
            file=sys.stderr,
        )
        if scan.invalid:
            print(
                f"  ({len(scan.invalid)} files rejected — re-run with --verbose for details)",
                file=sys.stderr,
            )
        return 2

    for invalid in scan.invalid:
        _LOG.info("skip %s: %s", invalid.path, invalid.reason)
    if scan.invalid and args.strict:
        print(
            f"error: {len(scan.invalid)} files rejected in strict mode",
            file=sys.stderr,
        )
        return 2

    splits = split_recordings(
        scan.recordings,
        val_fraction=args.val_fraction,
        test_fraction=args.test_fraction,
        seed=args.seed,
    )
    write_manifest(splits, args.output, base=input_dir, split_seed=args.seed)

    summary = summarize_splits(splits)
    print(f"manifest: {args.output}")
    for split_name in ("train", "val", "test"):
        counts = summary[split_name]
        items = ", ".join(f"{lbl}={n}" for lbl, n in sorted(counts.items())) or "(empty)"
        print(f"  {split_name}: {items}")
    if scan.invalid:
        print(f"rejected: {len(scan.invalid)} files (use --verbose to see why)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
