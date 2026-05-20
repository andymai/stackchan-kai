"""TFLite-based wake-word inference for ``kws-eval``.

The trained ``.tflite`` is a *streaming* model: each invocation
consumes one mel frame and produces one probability between 0 and
1. The model holds its own state across invocations (the ``resource
variable`` ops microWakeWord v2 relies on — see memory note
``project_stackchan_microwakeword_operators``), so the host runner
just feeds frames sequentially and reads the score.

The TFLite runtime is loaded lazily so importing this module never
fails on hosts without ``ai-edge-litert`` installed — the dep lives
in the optional ``eval`` group.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np

from .features import MEL_BIN_COUNT, MelFrontend, QuantParams

_LOG = logging.getLogger("kws_trainer.inference")


@dataclass(frozen=True)
class InferenceResult:
    """Outcome of running a TFLite model against a PCM clip."""

    per_frame_scores: list[float]
    peak_score: float
    peak_frame_index: int
    threshold: float
    triggered: bool

    @property
    def n_frames(self) -> int:
        return len(self.per_frame_scores)


def load_interpreter(model_path: Path) -> Any:
    """Open ``model_path`` with the TFLite runtime and return a
    ready-to-use Interpreter.

    Uses ``ai_edge_litert`` (the current canonical Python TFLite
    runtime as of 2024 — replaces the older ``tflite_runtime``
    package). Falls back to the full ``tensorflow`` package if
    ``ai_edge_litert`` isn't installed; the latter is bigger but
    operators with full TF may already have it.
    """
    try:
        from ai_edge_litert.interpreter import Interpreter  # type: ignore[import-not-found]
    except ImportError:
        try:
            from tensorflow.lite.python.interpreter import (  # type: ignore[import-untyped,unused-ignore]
                Interpreter,
            )
        except ImportError as e:
            raise RuntimeError(
                "no TFLite runtime found. Install the eval extra: "
                "`uv sync --extra eval` (pulls in ai-edge-litert)."
            ) from e
    interpreter = Interpreter(model_path=str(model_path))
    interpreter.allocate_tensors()
    return interpreter


@dataclass(frozen=True)
class ModelIO:
    """Snapshot of the interpreter's input/output tensor details.

    Reading these from the TFLite C API is cheap individually but
    adds up when called once per 10 ms frame on a multi-minute clip.
    Hoist out of the hot loop and pass through to [`run_frame`].
    """

    input_index: int
    input_shape: tuple[int, ...]
    input_dtype: Any
    output_index: int
    output_dtype: Any
    output_scale: float
    output_zero_point: int


def read_model_io(interpreter: Any) -> ModelIO:
    """Pull tensor metadata + output quantization from the
    interpreter once so the per-frame loop can pass it through
    without re-crossing the TFLite boundary."""
    inp = interpreter.get_input_details()[0]
    out = interpreter.get_output_details()[0]
    out_quant = out.get("quantization_parameters", {})
    out_scales = out_quant.get("scales", [])
    out_zps = out_quant.get("zero_points", [])
    return ModelIO(
        input_index=int(inp["index"]),
        input_shape=tuple(int(d) for d in inp["shape"]),
        input_dtype=inp["dtype"],
        output_index=int(out["index"]),
        output_dtype=out["dtype"],
        output_scale=float(out_scales[0]) if len(out_scales) > 0 else 1.0,
        output_zero_point=int(out_zps[0]) if len(out_zps) > 0 else 0,
    )


def input_quant_params(interpreter: Any) -> QuantParams:
    """Read the model's input-tensor quantization parameters.

    The frontend must quantize log-mel energies using the model's
    actual ``(scale, zero_point)`` — not the firmware default — so
    feature values land at the same integer points the model saw
    during training.
    """
    details = interpreter.get_input_details()[0]
    quant = details.get("quantization_parameters", {})
    scales = quant.get("scales", [])
    zero_points = quant.get("zero_points", [])
    # `if len(...) > 0` rather than `or [default]`: a numpy array
    # containing a single 0 (legitimate `zero_point=0` from a model
    # with symmetric quantization) is falsy under Python's `or`
    # truthiness, which silently shifts every feature by the
    # firmware-default 25 quantization steps and corrupts scores.
    scale = float(scales[0]) if len(scales) > 0 else 0.5
    zero_point = int(zero_points[0]) if len(zero_points) > 0 else -25
    return QuantParams(scale=scale, zero_point=zero_point)


def run_frame(interpreter: Any, frame: np.ndarray, io: ModelIO | None = None) -> float:
    """Feed one ``(MEL_BIN_COUNT,)`` ``int8`` frame to the model and
    return the scalar score (float in ``[0, 1]``).

    ``io`` is the cached tensor metadata from [`read_model_io`]. The
    optional default makes single-frame ad-hoc usage work without a
    pre-call setup step; batch loops should hoist [`read_model_io`]
    out and pass the result in to avoid per-frame TFLite-boundary
    crossings.
    """
    if io is None:
        io = read_model_io(interpreter)
    reshaped = frame.reshape(io.input_shape).astype(io.input_dtype)
    interpreter.set_tensor(io.input_index, reshaped)
    interpreter.invoke()
    raw = interpreter.get_tensor(io.output_index)
    if io.output_dtype == np.int8:
        score = float(io.output_scale * (int(raw.flatten()[0]) - io.output_zero_point))
    else:
        score = float(raw.flatten()[0])
    return score


def evaluate_pcm(
    interpreter: Any,
    pcm: np.ndarray,
    *,
    threshold: float = 0.5,
) -> InferenceResult:
    """Run streaming inference over an int16 PCM clip.

    Returns per-frame scores, peak score + index, and a triggered
    boolean (peak ≥ threshold). The frontend uses the model's actual
    input quantization, not the firmware default — so a model trained
    with a non-default ``scale`` doesn't see misaligned features.
    """
    quant = input_quant_params(interpreter)
    io = read_model_io(interpreter)
    frontend = MelFrontend(quant=quant)
    feature_matrix = frontend.process_pcm(pcm)
    scores: list[float] = []
    peak_score = 0.0
    peak_index = 0
    for i, frame in enumerate(feature_matrix):
        if frame.shape != (MEL_BIN_COUNT,):
            raise ValueError(f"frame {i}: expected shape ({MEL_BIN_COUNT},), got {frame.shape}")
        score = run_frame(interpreter, frame, io)
        scores.append(score)
        if score > peak_score:
            peak_score = score
            peak_index = i
    triggered = peak_score >= threshold
    _LOG.debug(
        "evaluate_pcm: %d frames, peak=%.3f at frame %d, triggered=%s",
        len(scores),
        peak_score,
        peak_index,
        triggered,
    )
    return InferenceResult(
        per_frame_scores=scores,
        peak_score=peak_score,
        peak_frame_index=peak_index,
        threshold=threshold,
        triggered=triggered,
    )


__all__ = [
    "InferenceResult",
    "ModelIO",
    "evaluate_pcm",
    "input_quant_params",
    "load_interpreter",
    "read_model_io",
    "run_frame",
]
