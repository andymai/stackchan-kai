"""Tests for the TFLite runner.

The real TFLite runtime lives in the optional ``eval`` extra; tests
mock the Interpreter shape so the unit suite runs without the ML
dep installed. End-to-end smoke against a real ``.tflite`` is
deferred to the on-device validation step.
"""

from __future__ import annotations

from typing import Any

import numpy as np
import pytest

from kws_trainer.features import MEL_BIN_COUNT
from kws_trainer.inference import (
    evaluate_pcm,
    input_quant_params,
    run_frame,
)


class _FakeInterpreter:
    """Minimal stub matching the subset of TFLite Interpreter's
    surface that the inference module touches: ``get_input_details``,
    ``get_output_details``, ``set_tensor``, ``invoke``, ``get_tensor``.
    """

    def __init__(
        self,
        *,
        input_shape: tuple[int, ...] = (1, MEL_BIN_COUNT, 1),
        input_dtype: Any = np.int8,
        input_scale: float = 0.5,
        input_zero_point: int = -25,
        output_dtype: Any = np.int8,
        output_scale: float = 1.0 / 255.0,
        output_zero_point: int = -128,
        score_for_frame: Any = None,
    ) -> None:
        self._input_shape = input_shape
        self._input_dtype = input_dtype
        self._input_scale = input_scale
        self._input_zero_point = input_zero_point
        self._output_dtype = output_dtype
        self._output_scale = output_scale
        self._output_zero_point = output_zero_point
        # `score_for_frame(frame_index, frame_array) -> raw_int8` lets
        # tests script per-frame outputs. Default returns mid-range.
        self._score_for_frame = score_for_frame or (lambda _i, _f: 0)
        self._frame_index = 0
        self._last_input: np.ndarray | None = None
        self._next_raw: int = 0

    def get_input_details(self) -> list[dict[str, Any]]:
        return [
            {
                "index": 0,
                "shape": np.array(self._input_shape, dtype=np.int32),
                "dtype": self._input_dtype,
                "quantization_parameters": {
                    "scales": np.array([self._input_scale]),
                    "zero_points": np.array([self._input_zero_point]),
                },
            }
        ]

    def get_output_details(self) -> list[dict[str, Any]]:
        return [
            {
                "index": 1,
                "shape": np.array([1], dtype=np.int32),
                "dtype": self._output_dtype,
                "quantization_parameters": {
                    "scales": np.array([self._output_scale]),
                    "zero_points": np.array([self._output_zero_point]),
                },
            }
        ]

    def set_tensor(self, _index: int, tensor: np.ndarray) -> None:
        self._last_input = tensor

    def invoke(self) -> None:
        if self._last_input is None:
            raise RuntimeError("invoke() before set_tensor()")
        self._next_raw = int(self._score_for_frame(self._frame_index, self._last_input.flatten()))
        self._frame_index += 1

    def get_tensor(self, _index: int) -> np.ndarray:
        return np.array([self._next_raw], dtype=self._output_dtype)


def test_input_quant_params_reads_from_interpreter() -> None:
    interp = _FakeInterpreter(input_scale=0.75, input_zero_point=10)
    q = input_quant_params(interp)
    assert q.scale == pytest.approx(0.75)
    assert q.zero_point == 10


def test_input_quant_params_falls_back_when_empty() -> None:
    class _NoQuantInterpreter(_FakeInterpreter):
        def get_input_details(self) -> list[dict[str, Any]]:
            return [
                {
                    "index": 0,
                    "shape": np.array((1, MEL_BIN_COUNT, 1), dtype=np.int32),
                    "dtype": np.int8,
                    "quantization_parameters": {},
                }
            ]

    q = input_quant_params(_NoQuantInterpreter())
    assert q.scale == 0.5
    assert q.zero_point == -25


def test_run_frame_dequantizes_int8_output() -> None:
    # Raw int8 output of 100 with scale=0.01 and zp=-50 →
    # (100 - (-50)) * 0.01 = 1.5
    interp = _FakeInterpreter(
        output_scale=0.01,
        output_zero_point=-50,
        score_for_frame=lambda _i, _f: 100,
    )
    frame = np.zeros(MEL_BIN_COUNT, dtype=np.int8)
    assert run_frame(interp, frame) == pytest.approx(1.5)


def test_run_frame_passes_float_outputs_through() -> None:
    # When output dtype is float, no dequantization should happen.
    class _FloatOutInterpreter(_FakeInterpreter):
        def __init__(self) -> None:
            super().__init__(output_dtype=np.float32)

        def get_tensor(self, _index: int) -> np.ndarray:
            # Return a deterministic float so the test can pin it.
            return np.array([0.42], dtype=np.float32)

    frame = np.zeros(MEL_BIN_COUNT, dtype=np.int8)
    assert run_frame(_FloatOutInterpreter(), frame) == pytest.approx(0.42)


def test_run_frame_reshapes_to_model_input_shape() -> None:
    # Captured input tensor must end up with the model's expected
    # shape (1, 40, 1) even though we pass in (40,).
    interp = _FakeInterpreter()
    frame = np.arange(MEL_BIN_COUNT, dtype=np.int8)
    run_frame(interp, frame)
    assert interp._last_input is not None
    assert interp._last_input.shape == (1, MEL_BIN_COUNT, 1)


def test_evaluate_pcm_aggregates_frame_scores() -> None:
    # Script the model to emit increasing raw ints; the dequantized
    # peak should land at the last frame.
    interp = _FakeInterpreter(
        output_scale=0.01,
        output_zero_point=0,
        # Each frame returns raw int8 == frame index, capped at int8 max.
        score_for_frame=lambda i, _f: min(i, 127),
    )
    # Long-enough PCM to produce ~6 frames.
    pcm = np.zeros(480 + 5 * 160, dtype=np.int16)
    result = evaluate_pcm(interp, pcm, threshold=0.04)
    assert result.n_frames == 6
    # Peak should be the last frame (5 * 0.01 = 0.05).
    assert result.peak_frame_index == 5
    assert result.peak_score == pytest.approx(0.05)
    assert result.triggered is True


def test_evaluate_pcm_below_threshold_not_triggered() -> None:
    interp = _FakeInterpreter(
        output_scale=0.001,
        output_zero_point=0,
        score_for_frame=lambda _i, _f: 1,
    )
    pcm = np.zeros(480 + 5 * 160, dtype=np.int16)
    result = evaluate_pcm(interp, pcm, threshold=0.5)
    assert result.peak_score < 0.5
    assert result.triggered is False


def test_evaluate_pcm_empty_pcm_returns_zero_frames() -> None:
    # Less than one window → no frames → peak score stays 0.
    interp = _FakeInterpreter()
    pcm = np.zeros(100, dtype=np.int16)
    result = evaluate_pcm(interp, pcm, threshold=0.5)
    assert result.n_frames == 0
    assert result.peak_score == 0.0
    assert result.triggered is False
