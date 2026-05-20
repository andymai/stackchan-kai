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
from typing import TYPE_CHECKING, Any

import numpy as np

from .features import MEL_BIN_COUNT, MelFrontend, QuantParams

if TYPE_CHECKING:
    pass

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


def input_quant_params(interpreter: Any) -> QuantParams:
    """Read the model's input-tensor quantization parameters.

    The frontend must quantize log-mel energies using the model's
    actual ``(scale, zero_point)`` — not the firmware default — so
    feature values land at the same integer points the model saw
    during training.
    """
    details = interpreter.get_input_details()[0]
    quant = details.get("quantization_parameters", {})
    scales = quant.get("scales", []) or [0.5]
    zero_points = quant.get("zero_points", []) or [-25]
    scale = float(scales[0]) if len(scales) else 0.5
    zero_point = int(zero_points[0]) if len(zero_points) else -25
    return QuantParams(scale=scale, zero_point=zero_point)


def run_frame(interpreter: Any, frame: np.ndarray) -> float:
    """Feed one ``(MEL_BIN_COUNT,)`` ``int8`` frame to the model and
    return the scalar score (float in ``[0, 1]``).

    The input tensor's shape is taken from the model — typically
    ``(1, 40, 1)`` for streaming MixConv models. Dequantization
    of the output uses the output tensor's own scale/zero_point.
    """
    input_details = interpreter.get_input_details()[0]
    output_details = interpreter.get_output_details()[0]
    target_shape = tuple(int(d) for d in input_details["shape"])
    # Most microWakeWord streaming models expect shape (1, 40, 1).
    # Reshape via broadcasting so the function works for any
    # consistent leading singleton.
    reshaped = frame.reshape(target_shape).astype(input_details["dtype"])
    interpreter.set_tensor(input_details["index"], reshaped)
    interpreter.invoke()
    raw = interpreter.get_tensor(output_details["index"])
    # Dequantize int8 outputs to float using the output tensor's
    # quantization parameters; float outputs pass through unchanged.
    if output_details["dtype"] == np.int8:
        out_quant = output_details.get("quantization_parameters", {})
        scales = out_quant.get("scales", [1.0])
        zps = out_quant.get("zero_points", [0])
        scale = float(scales[0])
        zp = int(zps[0])
        score = float(scale * (int(raw.flatten()[0]) - zp))
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
    frontend = MelFrontend(quant=quant)
    feature_matrix = frontend.process_pcm(pcm)
    scores: list[float] = []
    peak_score = 0.0
    peak_index = 0
    for i, frame in enumerate(feature_matrix):
        if frame.shape != (MEL_BIN_COUNT,):
            raise ValueError(f"frame {i}: expected shape ({MEL_BIN_COUNT},), got {frame.shape}")
        score = run_frame(interpreter, frame)
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
    "evaluate_pcm",
    "input_quant_params",
    "load_interpreter",
    "run_frame",
]
