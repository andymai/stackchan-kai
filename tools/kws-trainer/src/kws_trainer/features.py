"""Pure-numpy mel-spectrogram frontend mirroring the firmware's
``stackchan-audio-features`` crate.

The firmware's on-device KWS path computes log-mel features and feeds
them to a TFLite-micro classifier. ``kws-eval`` reproduces the
**exact same DSP pipeline** on the host so an evaluator's score for a
WAV matches what the device would compute on the same audio.

Spec parity sources (see ``crates/stackchan-audio-features/``):

- ``frontend.rs``: SAMPLE_RATE_HZ=16_000, WINDOW_SAMPLES=480,
  HOP_SAMPLES=160, FFT_SAMPLES=512, MEL_BIN_COUNT=40,
  MEL_LOWER_HZ=125.0, MEL_UPPER_HZ=7500.0.
- ``window.rs``: periodic Hann (``2pi*i/N``, not ``2pi*i/(N-1)``).
- ``mel.rs``: 40 triangles between 125 and 7500 Hz on the mel scale
  (``mel = 2595·log10(1 + hz/700)``).
- ``quantize.rs``: ``int8`` asymmetric quantization with default
  ``scale=0.5, zero_point=-25`` and ``LOG_EPSILON=1e-6``.

Any drift between this module and the firmware constants is a bug —
the firmware crate is the single source of truth.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

import numpy as np

SAMPLE_RATE_HZ: int = 16_000
WINDOW_SAMPLES: int = 480
HOP_SAMPLES: int = 160
FFT_SAMPLES: int = 512
FFT_BIN_COUNT: int = FFT_SAMPLES // 2 + 1
MEL_BIN_COUNT: int = 40
MEL_LOWER_HZ: float = 125.0
MEL_UPPER_HZ: float = 7500.0
# Firmware: `LOG_EPSILON = 1.0e-6` in `quantize.rs`.
LOG_EPSILON: float = 1.0e-6


@dataclass(frozen=True)
class QuantParams:
    """``int8`` asymmetric quantization parameters.

    Default values match microWakeWord's reference quantization
    (``scale=0.5, zero_point=-25``). Real model deployments override
    these from the loaded TFLite's input tensor's
    ``quantization_parameters``.
    """

    scale: float = 0.5
    zero_point: int = -25


def hz_to_mel(hz: float) -> float:
    """microWakeWord's mel scale: ``2595·log10(1 + hz/700)``."""
    return 2595.0 * math.log10(1.0 + hz / 700.0)


def mel_to_hz(mel: float) -> float:
    """Inverse of [`hz_to_mel`]."""
    return float(700.0 * (10.0 ** (mel / 2595.0) - 1.0))


def make_hann_window() -> np.ndarray:
    """Periodic 480-sample Hann window. ``2π·i/N``, not ``2π·i/(N-1)``."""
    n = np.arange(WINDOW_SAMPLES, dtype=np.float64)
    return 0.5 * (1.0 - np.cos(2.0 * np.pi * n / WINDOW_SAMPLES))


def make_mel_filterbank() -> np.ndarray:
    """40 by 257 triangular mel filterbank between 125 and 7500 Hz.

    Returns float64 weights indexed ``[mel_bin][fft_bin]``. Each row's
    triangle has its peak at the mel-bin centre with edges at the
    adjacent mel-bin centres.
    """
    mel_lower = hz_to_mel(MEL_LOWER_HZ)
    mel_upper = hz_to_mel(MEL_UPPER_HZ)
    mel_step = (mel_upper - mel_lower) / (MEL_BIN_COUNT + 1)
    centres_hz = np.array(
        [mel_to_hz(mel_lower + i * mel_step) for i in range(MEL_BIN_COUNT + 2)],
        dtype=np.float64,
    )
    bin_hz = np.arange(FFT_BIN_COUNT, dtype=np.float64) * SAMPLE_RATE_HZ / FFT_SAMPLES
    weights = np.zeros((MEL_BIN_COUNT, FFT_BIN_COUNT), dtype=np.float64)
    for m in range(MEL_BIN_COUNT):
        left, centre, right = centres_hz[m], centres_hz[m + 1], centres_hz[m + 2]
        for b, f in enumerate(bin_hz):
            if f <= left or f >= right:
                continue
            if f <= centre:
                weights[m, b] = (f - left) / (centre - left)
            else:
                weights[m, b] = (right - f) / (right - centre)
    return weights


def quantize_scalar(x: float, params: QuantParams) -> int:
    """``f32 → i8`` round-half-away-from-zero with saturation.

    Mirrors firmware's ``quantize_scalar`` in ``quantize.rs``. The
    ``int(round(...))`` cast saturates via ``clamp`` because raw
    ``int8`` truncates wrap-around for out-of-range values.
    """
    raw = x / params.scale + params.zero_point
    rounded = math.floor(raw + 0.5) if raw >= 0 else -math.floor(-raw + 0.5)
    return max(-128, min(127, rounded))


def log_then_quantize(mel_energy: np.ndarray, params: QuantParams) -> np.ndarray:
    """Apply ``natural-log`` (with epsilon floor) and ``int8`` quantize.

    ``mel_energy`` is shape ``(MEL_BIN_COUNT,)``. Output is shape
    ``(MEL_BIN_COUNT,)`` of dtype ``int8``.
    """
    if mel_energy.shape != (MEL_BIN_COUNT,):
        raise ValueError(f"mel_energy must have shape ({MEL_BIN_COUNT},), got {mel_energy.shape}")
    floored = np.maximum(mel_energy, LOG_EPSILON)
    logged = np.log(floored)
    # Vectorised round-half-away-from-zero + saturation.
    scaled = logged / params.scale + params.zero_point
    rounded = np.where(scaled >= 0, np.floor(scaled + 0.5), -np.floor(-scaled + 0.5))
    clipped: np.ndarray = np.clip(rounded, -128, 127).astype(np.int8)
    return clipped


class MelFrontend:
    """Streaming mel-feature extractor.

    Mirrors the firmware's [`MelFrontend`] class shape: feed samples
    in arbitrary chunks via [`push_samples`], drain emitted frames
    via the returned list. One frontend instance handles continuous
    audio for a session; call [`reset`] between independent inputs.

    For batch (file-at-a-time) eval, [`process_pcm`] is the simpler
    entry point — it accepts the whole sample array and returns the
    full ``(n_frames, MEL_BIN_COUNT)`` feature matrix.
    """

    def __init__(self, *, quant: QuantParams | None = None) -> None:
        self._quant = quant or QuantParams()
        self._hann = make_hann_window()
        self._mel_weights = make_mel_filterbank()
        self._buf = np.zeros(WINDOW_SAMPLES, dtype=np.float64)
        self._filled = 0
        self._hop_remaining = 0

    def reset(self) -> None:
        self._buf[:] = 0.0
        self._filled = 0
        self._hop_remaining = 0

    def push_samples(self, samples: np.ndarray) -> list[np.ndarray]:
        """Buffer ``samples`` (``int16`` 1-D) and return any frames
        that fit during this push.

        ``samples`` may be any length; the frontend tracks its own
        window/hop state across calls. The frame layout matches
        firmware's ``[i8; 40]`` (one element per mel bin).
        """
        if samples.dtype != np.int16:
            raise ValueError(f"samples must be int16, got {samples.dtype}")
        # Convert to float64 once for the whole batch; integer-to-float
        # widening is the same cost as the firmware's per-sample
        # promotion but avoids per-sample Python overhead.
        floats = samples.astype(np.float64)
        emitted: list[np.ndarray] = []
        idx = 0
        while idx < floats.size:
            if self._filled < WINDOW_SAMPLES:
                want = min(WINDOW_SAMPLES - self._filled, floats.size - idx)
                self._buf[self._filled : self._filled + want] = floats[idx : idx + want]
                self._filled += want
                idx += want
                if self._filled == WINDOW_SAMPLES:
                    emitted.append(self._emit_frame())
                    self._hop_remaining = HOP_SAMPLES
            else:
                # Steady state: rotate hop samples into the ring,
                # then emit one frame each time hop_remaining hits 0.
                want = min(self._hop_remaining, floats.size - idx)
                # Linear-shift the buffer (simpler than ring for a
                # host implementation; firmware uses a ring for perf
                # but on host the memmove is cheap).
                self._buf[:-want] = self._buf[want:]
                self._buf[-want:] = floats[idx : idx + want]
                idx += want
                self._hop_remaining -= want
                if self._hop_remaining == 0:
                    emitted.append(self._emit_frame())
                    self._hop_remaining = HOP_SAMPLES
        return emitted

    def _emit_frame(self) -> np.ndarray:
        """Run one window through Hann → FFT → mel → log → quantize.
        Returns an ``int8`` ``(MEL_BIN_COUNT,)`` array."""
        windowed = self._buf * self._hann
        padded = np.zeros(FFT_SAMPLES, dtype=np.float64)
        padded[:WINDOW_SAMPLES] = windowed
        spectrum = np.fft.rfft(padded)
        magnitude = np.abs(spectrum)
        mel_energy = self._mel_weights @ magnitude
        return log_then_quantize(mel_energy.astype(np.float64), self._quant)

    def process_pcm(self, pcm: np.ndarray) -> np.ndarray:
        """Convenience: run an entire ``int16`` PCM array through the
        frontend, returning a 2-D ``(n_frames, MEL_BIN_COUNT)``
        ``int8`` matrix. Resets the frontend state first so repeat
        calls on the same instance don't carry leftover ring state.
        """
        self.reset()
        frames = self.push_samples(pcm)
        if not frames:
            return np.zeros((0, MEL_BIN_COUNT), dtype=np.int8)
        return np.stack(frames, axis=0)


__all__ = [
    "FFT_BIN_COUNT",
    "FFT_SAMPLES",
    "HOP_SAMPLES",
    "LOG_EPSILON",
    "MEL_BIN_COUNT",
    "MEL_LOWER_HZ",
    "MEL_UPPER_HZ",
    "SAMPLE_RATE_HZ",
    "WINDOW_SAMPLES",
    "MelFrontend",
    "QuantParams",
    "hz_to_mel",
    "log_then_quantize",
    "make_hann_window",
    "make_mel_filterbank",
    "mel_to_hz",
    "quantize_scalar",
]
