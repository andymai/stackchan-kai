"""``bench-trim`` CLI — parse servo calibration bench output and
suggest pan/tilt trim values.

Capture the bench sweep, then feed it in:

```bash
just bench | tee /tmp/scfmr.log
bench-trim --input /tmp/scfmr.log
```

or pipe directly. ``--pan-trim`` / ``--tilt-trim`` are the *current*
values in ``/sd/STACKCHAN.RON`` (``head.pan_trim_deg`` /
``head.tilt_trim_deg``); the tool reports the suggested new values to
paste back. Exit codes mirror ``kws-eval``: 0 = all axes within
tolerance, 1 = an axis is out of tolerance (suggestion still printed),
2 = no parseable steps / bad input.
"""

from __future__ import annotations

import argparse
import logging
import sys
from collections.abc import Sequence
from pathlib import Path

from bench_trim.parser import parse_bench_lines
from bench_trim.trim import (
    DEFAULT_PAN_TRIM_DEG,
    DEFAULT_TILT_TRIM_DEG,
    AxisReport,
    analyze,
)

_LOG = logging.getLogger("bench_trim.cli.trim")


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="bench-trim",
        description=(
            "Parse `just bench` defmt output and suggest head pan/tilt "
            "trim values for STACKCHAN.RON."
        ),
    )
    p.add_argument(
        "--input",
        type=Path,
        default=None,
        help="Bench capture to parse (e.g. /tmp/scfmr.log). Reads stdin when omitted.",
    )
    p.add_argument(
        "--pan-trim",
        type=float,
        default=DEFAULT_PAN_TRIM_DEG,
        help=(
            "Current head.pan_trim_deg from STACKCHAN.RON, used to compute "
            f"the suggested new value (default: {DEFAULT_PAN_TRIM_DEG})."
        ),
    )
    p.add_argument(
        "--tilt-trim",
        type=float,
        default=DEFAULT_TILT_TRIM_DEG,
        help=(
            "Current head.tilt_trim_deg from STACKCHAN.RON, used to compute "
            f"the suggested new value (default: {DEFAULT_TILT_TRIM_DEG})."
        ),
    )
    p.add_argument(
        "--tolerance",
        type=float,
        default=2.0,
        help="Max acceptable post-trim residual in degrees (default: 2.0).",
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
    key = f"head.{report.axis}_trim_deg"
    print(f"[{report.axis}] {report.n} step(s)")
    if report.insufficient_data:
        print(f"  insufficient data (<3 steps) — cannot suggest a {key} value")
        return
    print(f"  current {key}: {report.current_trim_deg:+.2f}")
    print(f"  mean delta:    {report.mean_delta_deg:+.2f} deg")
    if report.flip_suspected:
        print(
            f"  WARNING: delta-vs-cmd slope {report.slope:+.2f} suggests the servo "
            "tracks opposite to command."
        )
        print(
            f"  Flip {report.axis.upper()}_DIRECTION in head.rs to -1.0 and re-run "
            "the bench before trusting a trim."
        )
        return
    print(f"  suggested {key}: {report.suggested_trim_deg:+.2f}")
    print(
        f"  post-trim residual: max {report.max_residual_deg:.2f} deg, "
        f"stddev {report.stddev_residual_deg:.2f} deg"
    )
    print(f"  verdict: {'PASS' if report.within_tolerance else 'WARN (out of tolerance)'}")


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    _configure_logging(args.verbose)

    if args.input is not None and not args.input.is_file():
        print(f"error: --input {args.input} does not exist", file=sys.stderr)
        return 2

    lines = _read_lines(args.input)
    result = parse_bench_lines(lines)

    if not result.steps:
        print(
            "error: no parseable bench steps found "
            f"(dropped {result.dropped}, unparsed {result.unparsed})",
            file=sys.stderr,
        )
        return 2

    reports = analyze(
        result,
        pan_trim=args.pan_trim,
        tilt_trim=args.tilt_trim,
        tolerance_deg=args.tolerance,
    )

    for axis in ("pan", "tilt"):
        if axis in reports:
            _print_axis_report(reports[axis])
    if result.dropped or result.unparsed:
        print(f"dropped {result.dropped} failed step(s), {result.unparsed} unparsed row(s)")

    all_pass = all(r.within_tolerance for r in reports.values() if not r.insufficient_data) and any(
        not r.insufficient_data for r in reports.values()
    )
    return 0 if all_pass else 1


if __name__ == "__main__":
    sys.exit(main())
