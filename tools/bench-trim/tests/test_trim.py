from __future__ import annotations

from bench_trim.parser import BenchStep, ParseResult
from bench_trim.trim import (
    DEFAULT_TILT_TRIM_DEG,
    POSITION_CENTER,
    POSITION_PER_DEGREE,
    analyze,
    delta_slope,
    direction_flip_suspected,
    mean_delta,
    suggest_trim,
)


def _step(axis: str, cmd: float, delta: float) -> BenchStep:
    actual = cmd + delta
    return BenchStep(
        axis=axis,
        cmd_deg=cmd,
        raw_pos=POSITION_CENTER,
        actual_deg=actual,
        delta_deg=delta,
    )


def test_constants_pinned_to_firmware() -> None:
    # These mirror crates/scservo/src/lib.rs + head.rs; a firmware change
    # to either must force a conscious update here.
    assert POSITION_CENTER == 512
    assert POSITION_PER_DEGREE == 1023.0 / 300.0
    assert DEFAULT_TILT_TRIM_DEG == 49.0


def test_constant_offset_suggests_current_minus_offset() -> None:
    steps = [_step("pan", c, 5.0) for c in (-30.0, -10.0, 10.0, 30.0)]
    assert mean_delta(steps) == 5.0
    assert suggest_trim(steps, 0.0) == -5.0


def test_tilt_offset_round_trips_toward_default_trim() -> None:
    # A unit that already carries the 49 deg trim but logs a residual
    # ~+44 offset (bench runs with trim=0) should converge near 49 - 44.
    steps = [_step("tilt", c, 44.0) for c in (-20.0, -10.0, 0.0, 10.0, 20.0)]
    assert suggest_trim(steps, DEFAULT_TILT_TRIM_DEG) == DEFAULT_TILT_TRIM_DEG - 44.0


def test_flip_suspected_on_inverted_tracking() -> None:
    # actual ≈ -cmd → delta = actual - cmd = -2*cmd, slope ≈ -2.
    steps = [_step("pan", c, -2.0 * c) for c in (-30.0, -10.0, 10.0, 30.0)]
    assert delta_slope(steps) == -2.0
    assert direction_flip_suspected(steps) is True


def test_flat_offset_not_flip() -> None:
    steps = [_step("pan", c, 5.0) for c in (-30.0, -10.0, 10.0, 30.0)]
    assert delta_slope(steps) == 0.0
    assert direction_flip_suspected(steps) is False


def test_analyze_within_tolerance_passes() -> None:
    steps = [_step("pan", c, 5.0) for c in (-30.0, -10.0, 10.0, 30.0)]
    reports = analyze(ParseResult(steps=steps), pan_trim=0.0, tilt_trim=49.0, tolerance_deg=2.0)
    pan = reports["pan"]
    assert pan.suggested_trim_deg == -5.0
    assert pan.max_residual_deg == 0.0
    assert pan.within_tolerance is True


def test_analyze_out_of_tolerance_warns_on_noisy_residual() -> None:
    # Symmetric deltas around a +5 mean leave a residual spread > tolerance.
    steps = [
        _step("pan", -30.0, 0.0),
        _step("pan", -10.0, 5.0),
        _step("pan", 10.0, 5.0),
        _step("pan", 30.0, 10.0),
    ]
    reports = analyze(ParseResult(steps=steps), pan_trim=0.0, tilt_trim=49.0, tolerance_deg=2.0)
    pan = reports["pan"]
    assert pan.within_tolerance is False
    assert pan.max_residual_deg > 2.0


def test_insufficient_data_flagged() -> None:
    steps = [_step("tilt", 0.0, 1.0)]
    reports = analyze(ParseResult(steps=steps), pan_trim=0.0, tilt_trim=49.0, tolerance_deg=2.0)
    assert reports["tilt"].insufficient_data is True
    assert reports["tilt"].within_tolerance is False
