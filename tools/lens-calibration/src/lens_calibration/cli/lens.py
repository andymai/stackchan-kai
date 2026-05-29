"""``lens-calibration`` CLI — parse tracker-bench output and check the
camera ``flip_x`` / ``flip_y`` mounting flags.

Capture the tracker bench, then feed it in:

```bash
just tracker-bench | tee /tmp/scfmr.log
lens-calibration --input /tmp/scfmr.log
```

or pipe directly. ``--flip-x`` / ``--flip-y`` and ``--fov-h`` / ``--fov-v``
are the *current* values in ``/sd/STACKCHAN.RON`` (the ``tracker:`` block);
the tool reports whether either flip flag should change and surfaces lens
coverage diagnostics. Exit codes mirror ``bench-trim``: 0 = mounting looks
correct, 1 = a flip flag should change (suggestion printed), 2 = no
parseable frames / bad input.

Scope: the bench logs no ground-truth target position, so an *optimal*
FOV cannot be solved from the capture alone. This tool detects wrong flip
flags from the closed-loop feedback sign and reports coverage diagnostics
the operator interprets against the current FOV — it does not emit a
numeric FOV correction.
"""

from __future__ import annotations

import argparse
import logging
import sys
from collections.abc import Sequence
from pathlib import Path

from lens_calibration.calibrate import (
    DEFAULT_FLIP_X,
    DEFAULT_FLIP_Y,
    DEFAULT_FOV_H_DEG,
    DEFAULT_FOV_V_DEG,
    MIN_FRAMES_FOR_FIT,
    AxisReport,
    analyze,
)
from lens_calibration.parser import parse_tracker_lines

_LOG = logging.getLogger("lens_calibration.cli.lens")

_FLIP_KEY = {"x": "flip_x", "y": "flip_y"}


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="lens-calibration",
        description=(
            "Parse `just tracker-bench` defmt output and check the camera "
            "flip_x/flip_y mounting flags for STACKCHAN.RON."
        ),
    )
    p.add_argument(
        "--input",
        type=Path,
        default=None,
        help="Tracker-bench capture to parse (e.g. /tmp/scfmr.log). Reads stdin when omitted.",
    )
    p.add_argument(
        "--flip-x",
        action="store_true",
        default=DEFAULT_FLIP_X,
        help="Current tracker.flip_x from STACKCHAN.RON (default: off).",
    )
    p.add_argument(
        "--flip-y",
        action="store_true",
        default=DEFAULT_FLIP_Y,
        help="Current tracker.flip_y from STACKCHAN.RON (default: off).",
    )
    p.add_argument(
        "--fov-h",
        type=float,
        default=DEFAULT_FOV_H_DEG,
        help=(
            "Current tracker.fov_h_deg from STACKCHAN.RON, shown alongside "
            f"horizontal coverage (default: {DEFAULT_FOV_H_DEG})."
        ),
    )
    p.add_argument(
        "--fov-v",
        type=float,
        default=DEFAULT_FOV_V_DEG,
        help=(
            "Current tracker.fov_v_deg from STACKCHAN.RON, shown alongside "
            f"vertical coverage (default: {DEFAULT_FOV_V_DEG})."
        ),
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


def _read_lines(input_path: Path | None) -> list[str]:
    if input_path is None:
        return sys.stdin.read().splitlines()
    return input_path.read_text(encoding="utf-8", errors="replace").splitlines()


def _print_axis_report(report: AxisReport) -> None:
    label = "horizontal (pan)" if report.axis == "x" else "vertical (tilt)"
    key = f"tracker.{_FLIP_KEY[report.axis]}"
    print(f"[{label}] {report.n_pairs} consecutive Tracking pair(s)")
    if report.insufficient_data:
        print(
            f"  insufficient data (<{MIN_FRAMES_FOR_FIT} pairs) — "
            f"cannot judge {key}; capture more sustained motion"
        )
        return
    print(f"  current {key}: {str(report.current_flip).lower()}")
    print(f"  feedback correlation: {report.feedback_correlation:+.2f}")
    print(
        f"  centroid coverage: mean |c| {report.mean_abs_centroid:.2f}, "
        f"max |c| {report.max_abs_centroid:.2f}"
    )
    print(f"  pose span: {report.pose_span_deg:.1f} deg (fov {report.fov_deg:.0f} deg)")
    if report.flip_change:
        print(
            f"  WARNING: positive feedback — the head chases away on this axis. "
            f"Set {key}: {str(report.suggested_flip).lower()} and re-run the bench."
        )
    else:
        print("  verdict: PASS (mounting looks correct)")


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    _configure_logging(args.verbose)

    if args.input is not None and not args.input.is_file():
        print(f"error: --input {args.input} does not exist", file=sys.stderr)
        return 2

    lines = _read_lines(args.input)
    result = parse_tracker_lines(lines)

    if not result.frames:
        print(
            f"error: no parseable tracker-bench frames found (unparsed {result.unparsed})",
            file=sys.stderr,
        )
        return 2

    reports = analyze(
        result,
        flip_x=args.flip_x,
        flip_y=args.flip_y,
        fov_h_deg=args.fov_h,
        fov_v_deg=args.fov_v,
    )

    for axis in ("x", "y"):
        _print_axis_report(reports[axis])
    if result.unparsed:
        print(f"{result.unparsed} unparsed row(s)")

    any_flip_change = any(r.flip_change for r in reports.values())
    return 1 if any_flip_change else 0


if __name__ == "__main__":
    sys.exit(main())
