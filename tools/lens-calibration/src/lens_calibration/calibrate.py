"""Flip-flag detection and lens diagnostics over parsed tracker frames.

The firmware maps a post-flip normalised centroid to a pose delta (see
``crates/tracker/src/lib.rs`` ``update``):

```text
pan_delta  = +nx_eff * fov_h_deg * 0.5 * p_gain   (nx > 0 → pan right)
tilt_delta = -ny_eff * fov_v_deg * 0.5 * p_gain   (ny > 0 → tilt down)
```

``flip_x`` / ``flip_y`` negate the centroid *before* that mapping, so the
logged centroid is already flip-adjusted. That makes the within-frame
centroid→pose sign tautological — it cannot reveal a wrong flip on its
own. What the capture *does* expose is the closed-loop behaviour across
consecutive Tracking decisions:

* A correctly-mounted axis has **negative feedback** — the head aims at
  the motion, so the centroid component shrinks toward zero after a move.
* A wrongly-flipped axis has **positive feedback** — the head chases
  away from the motion, so the centroid component grows / saturates in
  one sign while the pose runs toward its clamp.

We model this as the correlation between a centroid component at frame
``i`` and its change to frame ``i+1`` over consecutive *logged* Tracking
frames: a consistently negative correlation is a healthy mount, a
positive one flags a flip. This is a heuristic, not a solve: it needs
the head to actually move the target across the frame and enough frames
to be statistically meaningful, so sparse or low-motion captures return
insufficient-data rather than a guess.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

from .parser import BenchFrame, ParseResult

# Mirrored from the firmware so the tool can reason about the lens
# without a device. KEEP IN SYNC with
# crates/stackchan-net/src/config.rs (TrackerSettings::DEFAULT) and
# crates/tracker/src/lib.rs (TrackerConfig::DEFAULT); a firmware change
# to any of these silently invalidates the diagnostics below.
DEFAULT_FOV_H_DEG = 62.0
DEFAULT_FOV_V_DEG = 49.0
DEFAULT_FLIP_X = False
DEFAULT_FLIP_Y = False

# Below this many consecutive Tracking pairs, the feedback-sign fit is
# too noisy to trust — report insufficient data instead of guessing.
MIN_FRAMES_FOR_FIT = 4

# A feedback correlation at or above this implies the head is chasing
# away from the target on that axis (positive feedback) — a flip suspect.
# A small dead band around zero avoids flagging near-stationary captures.
_FLIP_CORRELATION_THRESHOLD = 0.2


@dataclass(frozen=True)
class AxisReport:
    """Per-axis flip analysis and lens diagnostics."""

    axis: str
    n_pairs: int
    feedback_correlation: float
    current_flip: bool
    suggested_flip: bool
    mean_abs_centroid: float
    max_abs_centroid: float
    pose_span_deg: float
    fov_deg: float
    insufficient_data: bool

    @property
    def flip_change(self) -> bool:
        """True when the suggested flip differs from the current setting."""
        return (not self.insufficient_data) and (self.suggested_flip != self.current_flip)


def _tracking_frames(result: ParseResult) -> list[BenchFrame]:
    return [f for f in result.frames if f.motion == "Tracking"]


def _correlation(xs: list[float], ys: list[float]) -> float:
    """Pearson correlation. Returns 0.0 when either series has no
    variance (a flat capture carries no flip signal)."""
    n = len(xs)
    if n == 0:
        return 0.0
    mean_x = sum(xs) / n
    mean_y = sum(ys) / n
    cov = sum((x - mean_x) * (y - mean_y) for x, y in zip(xs, ys, strict=True))
    var_x = sum((x - mean_x) ** 2 for x in xs)
    var_y = sum((y - mean_y) ** 2 for y in ys)
    if var_x == 0.0 or var_y == 0.0:
        return 0.0
    return cov / math.sqrt(var_x * var_y)


def _component_series(frames: list[BenchFrame], axis: str) -> list[float]:
    if axis == "x":
        return [f.nx for f in frames]
    return [f.ny for f in frames]


def feedback_correlation(frames: list[BenchFrame], axis: str) -> float:
    """Correlation between a centroid component and its change to the
    next frame, over consecutive Tracking frames.

    Negative feedback (centroid shrinks after a move) → negative; a
    positive value means the centroid grows in place — the head is
    chasing away, i.e. the axis flip is wrong.
    """
    series = _component_series(frames, axis)
    if len(series) < 2:
        return 0.0
    current = series[:-1]
    delta = [series[i + 1] - series[i] for i in range(len(series) - 1)]
    return _correlation(current, delta)


def _analyze_axis(
    axis: str,
    frames: list[BenchFrame],
    current_flip: bool,
    fov_deg: float,
) -> AxisReport:
    series = _component_series(frames, axis)
    pose = [f.pan_deg if axis == "x" else f.tilt_deg for f in frames]
    n_pairs = max(len(series) - 1, 0)
    mean_abs = sum(abs(v) for v in series) / len(series) if series else 0.0
    max_abs = max((abs(v) for v in series), default=0.0)
    pose_span = (max(pose) - min(pose)) if pose else 0.0

    if n_pairs < MIN_FRAMES_FOR_FIT:
        return AxisReport(
            axis=axis,
            n_pairs=n_pairs,
            feedback_correlation=0.0,
            current_flip=current_flip,
            suggested_flip=current_flip,
            mean_abs_centroid=mean_abs,
            max_abs_centroid=max_abs,
            pose_span_deg=pose_span,
            fov_deg=fov_deg,
            insufficient_data=True,
        )

    corr = feedback_correlation(frames, axis)
    flip_wrong = corr >= _FLIP_CORRELATION_THRESHOLD
    return AxisReport(
        axis=axis,
        n_pairs=n_pairs,
        feedback_correlation=corr,
        current_flip=current_flip,
        # A wrong-feedback axis needs the flip toggled from its current value.
        suggested_flip=(not current_flip) if flip_wrong else current_flip,
        mean_abs_centroid=mean_abs,
        max_abs_centroid=max_abs,
        pose_span_deg=pose_span,
        fov_deg=fov_deg,
        insufficient_data=False,
    )


def analyze(
    result: ParseResult,
    *,
    flip_x: bool,
    flip_y: bool,
    fov_h_deg: float,
    fov_v_deg: float,
) -> dict[str, AxisReport]:
    """Produce a per-axis flip report from the Tracking frames.

    ``flip_x`` / ``flip_y`` and the FOV values are the *current*
    ``STACKCHAN.RON`` settings; the report says whether each flip flag
    should change and surfaces centroid / pose coverage as context.
    """
    tracking = _tracking_frames(result)
    return {
        "x": _analyze_axis("x", tracking, flip_x, fov_h_deg),
        "y": _analyze_axis("y", tracking, flip_y, fov_v_deg),
    }
