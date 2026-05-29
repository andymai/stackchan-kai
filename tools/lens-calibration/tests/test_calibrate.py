from __future__ import annotations

from lens_calibration.calibrate import (
    DEFAULT_FLIP_X,
    DEFAULT_FLIP_Y,
    DEFAULT_FOV_H_DEG,
    DEFAULT_FOV_V_DEG,
    analyze,
    feedback_correlation,
)
from lens_calibration.parser import BenchFrame, ParseResult


def _frame(nx: float, ny: float, *, motion: str = "Tracking") -> BenchFrame:
    # Pose values track the centroid sign per the firmware mapping
    # (nx>0 → +pan, ny>0 → -tilt); the analysis keys off the centroid.
    return BenchFrame(
        motion=motion,
        fired=12,
        nx=nx,
        ny=ny,
        pan_deg=nx * 10.0,
        tilt_deg=-ny * 10.0,
    )


# Correct mount: the head aims at the motion, so the centroid magnitude
# decays toward zero on each successive Tracking frame (negative feedback).
CONVERGING = [0.6, 0.45, 0.32, 0.21, 0.12, 0.05]
# Flipped axis: the head chases away, so the centroid magnitude grows
# until the pose saturates (positive feedback).
DIVERGING = [0.1, 0.2, 0.33, 0.48, 0.62, 0.75]


def test_constants_pinned_to_firmware() -> None:
    # Mirror crates/stackchan-net/src/config.rs (TrackerSettings::DEFAULT)
    # and crates/tracker/src/lib.rs (TrackerConfig::DEFAULT); a firmware
    # change to any of these must force a conscious update here.
    assert DEFAULT_FOV_H_DEG == 62.0
    assert DEFAULT_FOV_V_DEG == 49.0
    assert DEFAULT_FLIP_X is False
    assert DEFAULT_FLIP_Y is False


def test_converging_axis_has_negative_feedback() -> None:
    frames = [_frame(nx, 0.0) for nx in CONVERGING]
    assert feedback_correlation(frames, "x") < 0.0


def test_diverging_axis_has_positive_feedback() -> None:
    frames = [_frame(nx, 0.0) for nx in DIVERGING]
    assert feedback_correlation(frames, "x") > 0.0


def test_correct_mount_passes_both_axes() -> None:
    frames = [_frame(nx, ny) for nx, ny in zip(CONVERGING, CONVERGING, strict=True)]
    reports = analyze(
        ParseResult(frames=frames),
        flip_x=False,
        flip_y=False,
        fov_h_deg=DEFAULT_FOV_H_DEG,
        fov_v_deg=DEFAULT_FOV_V_DEG,
    )
    assert reports["x"].flip_change is False
    assert reports["y"].flip_change is False
    assert reports["x"].suggested_flip is False
    assert reports["y"].suggested_flip is False


def test_flipped_x_detected_and_toggled() -> None:
    # X diverges (wrong flip), Y converges (correct).
    frames = [_frame(nx, ny) for nx, ny in zip(DIVERGING, CONVERGING, strict=True)]
    reports = analyze(
        ParseResult(frames=frames),
        flip_x=False,
        flip_y=False,
        fov_h_deg=DEFAULT_FOV_H_DEG,
        fov_v_deg=DEFAULT_FOV_V_DEG,
    )
    assert reports["x"].flip_change is True
    assert reports["x"].suggested_flip is True
    assert reports["y"].flip_change is False


def test_flipped_y_detected_and_toggled() -> None:
    frames = [_frame(nx, ny) for nx, ny in zip(CONVERGING, DIVERGING, strict=True)]
    reports = analyze(
        ParseResult(frames=frames),
        flip_x=False,
        flip_y=False,
        fov_h_deg=DEFAULT_FOV_H_DEG,
        fov_v_deg=DEFAULT_FOV_V_DEG,
    )
    assert reports["y"].flip_change is True
    assert reports["y"].suggested_flip is True
    assert reports["x"].flip_change is False


def test_already_flipped_axis_with_good_feedback_stays_on() -> None:
    # Operator already set flip_x=true and the capture converges: keep it.
    frames = [_frame(nx, 0.0) for nx in CONVERGING]
    reports = analyze(
        ParseResult(frames=frames),
        flip_x=True,
        flip_y=False,
        fov_h_deg=DEFAULT_FOV_H_DEG,
        fov_v_deg=DEFAULT_FOV_V_DEG,
    )
    assert reports["x"].suggested_flip is True
    assert reports["x"].flip_change is False


def test_diverging_with_flip_already_on_suggests_turning_off() -> None:
    frames = [_frame(nx, 0.0) for nx in DIVERGING]
    reports = analyze(
        ParseResult(frames=frames),
        flip_x=True,
        flip_y=False,
        fov_h_deg=DEFAULT_FOV_H_DEG,
        fov_v_deg=DEFAULT_FOV_V_DEG,
    )
    assert reports["x"].suggested_flip is False
    assert reports["x"].flip_change is True


def test_sparse_capture_flagged_insufficient() -> None:
    frames = [_frame(nx, 0.0) for nx in (0.5, 0.4)]
    reports = analyze(
        ParseResult(frames=frames),
        flip_x=False,
        flip_y=False,
        fov_h_deg=DEFAULT_FOV_H_DEG,
        fov_v_deg=DEFAULT_FOV_V_DEG,
    )
    assert reports["x"].insufficient_data is True
    assert reports["x"].flip_change is False


def test_non_tracking_frames_excluded_from_fit() -> None:
    # Only Tracking frames feed the feedback fit; Warmup / GlobalEvent are
    # decision noise the bench also logs.
    frames = [_frame(nx, 0.0) for nx in DIVERGING]
    frames += [_frame(0.0, 0.0, motion="GlobalEvent") for _ in range(10)]
    reports = analyze(
        ParseResult(frames=frames),
        flip_x=False,
        flip_y=False,
        fov_h_deg=DEFAULT_FOV_H_DEG,
        fov_v_deg=DEFAULT_FOV_V_DEG,
    )
    assert reports["x"].n_pairs == len(DIVERGING) - 1
    assert reports["x"].flip_change is True
