"""Trim-suggestion logic over parsed bench steps.

The firmware maps a commanded angle to a servo step count as
``commanded_pos = POSITION_CENTER + direction * (angle + trim) *
POSITION_PER_DEGREE`` (see ``crates/stackchan-firmware/src/head.rs``
``position_for``). The bench logs ``delta = actual - cmd`` with trim=0
and direction=+1, so a systematic non-zero mean delta is exactly the
trim correction: shifting trim by ``-mean_delta`` drives delta toward 0.

A delta-vs-cmd slope near ``-2`` means ``actual ≈ -cmd`` — the servo
tracks opposite to the command, which a numeric trim cannot fix. In that
case the operator must flip the axis ``*_DIRECTION`` in head.rs and
re-run the bench; we flag it rather than emitting a misleading trim.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

from .parser import BenchStep, ParseResult

# Mirrored from the firmware so the tool can reason about the servo math
# without a device. KEEP IN SYNC with crates/scservo/src/lib.rs
# (POSITION_CENTER, POSITION_PER_DEGREE) and the trim defaults in
# crates/stackchan-firmware/src/head.rs (PAN_TRIM_DEG, TILT_TRIM_DEG); a
# firmware change to any of these silently invalidates the suggestions.
POSITION_CENTER = 512
POSITION_PER_DEGREE = 1023.0 / 300.0
DEFAULT_PAN_TRIM_DEG = 0.0
DEFAULT_TILT_TRIM_DEG = 49.0

# Below this many steps on an axis, slope and trim fits are too noisy to
# trust — report insufficient data instead of guessing.
MIN_STEPS_FOR_FIT = 3

# A delta-vs-cmd slope at or below this implies actual tracks opposite to
# command (ideal flipped-axis slope is -2.0); treat it as a flip suspect.
_FLIP_SLOPE_THRESHOLD = -1.0


@dataclass(frozen=True)
class AxisReport:
    """Per-axis analysis of a bench sweep."""

    axis: str
    n: int
    mean_delta_deg: float
    current_trim_deg: float
    suggested_trim_deg: float
    slope: float
    flip_suspected: bool
    max_residual_deg: float
    stddev_residual_deg: float
    within_tolerance: bool
    insufficient_data: bool


def mean_delta(steps: list[BenchStep]) -> float:
    return sum(s.delta_deg for s in steps) / len(steps)


def suggest_trim(steps: list[BenchStep], current_trim: float) -> float:
    """New trim that drives mean delta toward zero.

    delta = actual - cmd, so subtracting the mean delta from the current
    trim cancels the systematic offset.
    """
    return current_trim - mean_delta(steps)


def delta_slope(steps: list[BenchStep]) -> float:
    """Least-squares slope of delta vs cmd. Zero variance in cmd (all
    steps at one command) yields a 0.0 slope — flat, no flip."""
    n = len(steps)
    mean_cmd = sum(s.cmd_deg for s in steps) / n
    mean_d = sum(s.delta_deg for s in steps) / n
    cov = sum((s.cmd_deg - mean_cmd) * (s.delta_deg - mean_d) for s in steps)
    var = sum((s.cmd_deg - mean_cmd) ** 2 for s in steps)
    if var == 0.0:
        return 0.0
    return cov / var


def direction_flip_suspected(steps: list[BenchStep]) -> bool:
    if len(steps) < MIN_STEPS_FOR_FIT:
        return False
    return delta_slope(steps) <= _FLIP_SLOPE_THRESHOLD


def residuals_after(steps: list[BenchStep], trim_shift: float) -> list[float]:
    """Deltas after applying a trim shift of ``trim_shift`` degrees.

    A positive trim shift adds to the effective angle, reducing delta by
    the same amount in the +direction reference frame.
    """
    return [s.delta_deg + trim_shift for s in steps]


def max_abs_residual(residuals: list[float]) -> float:
    return max((abs(r) for r in residuals), default=0.0)


def stddev(residuals: list[float]) -> float:
    n = len(residuals)
    if n == 0:
        return 0.0
    mean = sum(residuals) / n
    return math.sqrt(sum((r - mean) ** 2 for r in residuals) / n)


def _analyze_axis(
    axis: str, steps: list[BenchStep], current_trim: float, tolerance_deg: float
) -> AxisReport:
    n = len(steps)
    if n < MIN_STEPS_FOR_FIT:
        md = mean_delta(steps) if n else 0.0
        return AxisReport(
            axis=axis,
            n=n,
            mean_delta_deg=md,
            current_trim_deg=current_trim,
            suggested_trim_deg=current_trim - md if n else current_trim,
            slope=0.0,
            flip_suspected=False,
            max_residual_deg=0.0,
            stddev_residual_deg=0.0,
            within_tolerance=False,
            insufficient_data=True,
        )

    md = mean_delta(steps)
    suggested = current_trim - md
    flip = direction_flip_suspected(steps)
    # Suggested trim shifts the effective angle by (suggested - current) =
    # -mean_delta, so residuals are deltas plus that shift.
    residuals = residuals_after(steps, suggested - current_trim)
    max_res = max_abs_residual(residuals)
    return AxisReport(
        axis=axis,
        n=n,
        mean_delta_deg=md,
        current_trim_deg=current_trim,
        suggested_trim_deg=suggested,
        slope=delta_slope(steps),
        flip_suspected=flip,
        max_residual_deg=max_res,
        stddev_residual_deg=stddev(residuals),
        within_tolerance=(not flip) and max_res <= tolerance_deg,
        insufficient_data=False,
    )


def analyze(
    result: ParseResult,
    *,
    pan_trim: float,
    tilt_trim: float,
    tolerance_deg: float,
) -> dict[str, AxisReport]:
    """Group parsed steps by axis and produce a per-axis report."""
    by_axis: dict[str, list[BenchStep]] = {}
    for step in result.steps:
        by_axis.setdefault(step.axis, []).append(step)

    reports: dict[str, AxisReport] = {}
    for axis, current in (("pan", pan_trim), ("tilt", tilt_trim)):
        steps = by_axis.get(axis, [])
        if not steps:
            continue
        reports[axis] = _analyze_axis(axis, steps, current, tolerance_deg)
    return reports
