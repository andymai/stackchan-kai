"""Tests for the numpy mel-spectrogram frontend.

These pin the DSP outputs against known references so any drift
from the firmware's ``stackchan-audio-features`` spec is caught at
host-test time, before a model trained on host features ever gets
flashed.
"""

from __future__ import annotations

import math

import numpy as np
import pytest

from kws_trainer.features import (
    FFT_BIN_COUNT,
    HOP_SAMPLES,
    LOG_EPSILON,
    MEL_BIN_COUNT,
    MEL_LOWER_HZ,
    MEL_UPPER_HZ,
    SAMPLE_RATE_HZ,
    WINDOW_SAMPLES,
    MelFrontend,
    QuantParams,
    hz_to_mel,
    log_then_quantize,
    make_hann_window,
    make_mel_filterbank,
    mel_to_hz,
    quantize_scalar,
)


def test_constants_match_firmware_spec() -> None:
    # Drift guard: if these change in `stackchan-audio-features::frontend`,
    # update them here too — the on-device feature pipeline IS the spec.
    assert SAMPLE_RATE_HZ == 16_000
    assert WINDOW_SAMPLES == 480
    assert HOP_SAMPLES == 160
    assert FFT_BIN_COUNT == 257
    assert MEL_BIN_COUNT == 40
    assert pytest.approx(125.0) == MEL_LOWER_HZ
    assert pytest.approx(7500.0) == MEL_UPPER_HZ


def test_hz_to_mel_round_trip() -> None:
    # mel(hz(m)) == m for arbitrary mel values.
    for m in [50.0, 500.0, 1000.0, 3000.0]:
        assert mel_to_hz(hz_to_mel(700.0 * (10 ** (m / 2595.0) - 1.0))) == pytest.approx(
            700.0 * (10 ** (m / 2595.0) - 1.0)
        )


def test_hz_to_mel_known_values() -> None:
    # 1000 Hz on the mel scale is well-tabulated as ~1000 mel (the
    # scale is designed so 1000 Hz ≈ 1000 mel for the slaney variant;
    # the htk variant we use puts it slightly higher).
    assert hz_to_mel(1000.0) == pytest.approx(999.99, abs=2.0)


def test_hann_window_is_periodic() -> None:
    # Periodic Hann (the firmware spec): coeff[0] == 0, coeff[N/2]
    # is the peak (1.0), and coeff is symmetric around N/2 (with the
    # subtle off-by-one that periodic introduces).
    window = make_hann_window()
    assert window.shape == (WINDOW_SAMPLES,)
    assert window[0] == pytest.approx(0.0)
    assert window[WINDOW_SAMPLES // 2] == pytest.approx(1.0)
    # The N-1 coefficient is not 0 for periodic — that's a periodic
    # vs symmetric Hann tell. Pin the value so drift to symmetric
    # is caught.
    expected_last = 0.5 * (1.0 - math.cos(2.0 * math.pi * (WINDOW_SAMPLES - 1) / WINDOW_SAMPLES))
    assert window[-1] == pytest.approx(expected_last)


def test_mel_filterbank_shape_and_partition() -> None:
    fb = make_mel_filterbank()
    assert fb.shape == (MEL_BIN_COUNT, FFT_BIN_COUNT)
    # Triangles sum to a partition-of-unity-ish profile across the
    # mel-overlap region: each inner FFT bin is covered by ~1.0
    # total weight across all mel filters. Bins outside the
    # MEL_LOWER..MEL_UPPER band drop to zero.
    bin_sum = fb.sum(axis=0)
    # Find a bin firmly inside the active mel band (e.g. ~1500 Hz).
    inner_bin = round(1500.0 * FFT_BIN_COUNT * 2 / SAMPLE_RATE_HZ)
    assert bin_sum[inner_bin] == pytest.approx(1.0, abs=0.05)


def test_quantize_scalar_default_quant_zero() -> None:
    # Default quant: scale=0.5, zero_point=-25 → 0.0 quantizes to
    # zero_point exactly (no rounding contribution from input).
    assert quantize_scalar(0.0, QuantParams()) == -25


def test_quantize_scalar_saturates_on_overflow() -> None:
    # Very large positive input clamps at +127; very negative at -128.
    assert quantize_scalar(10_000.0, QuantParams()) == 127
    assert quantize_scalar(-10_000.0, QuantParams()) == -128


def test_quantize_scalar_round_half_away_from_zero() -> None:
    # Mirrors firmware's `quantize_scalar` rounding behaviour. Use
    # scale=1.0, zero_point=0 so the output equals round(input).
    p = QuantParams(scale=1.0, zero_point=0)
    assert quantize_scalar(0.5, p) == 1  # half-up
    assert quantize_scalar(-0.5, p) == -1  # half-down (away-from-zero)
    assert quantize_scalar(1.5, p) == 2
    assert quantize_scalar(-1.5, p) == -2


def test_log_then_quantize_handles_zero_via_epsilon() -> None:
    # Inputs at 0 land at log(LOG_EPSILON) ≈ -13.8 which (under
    # default quant) is well within the int8 range and well below 0.
    out = log_then_quantize(np.zeros(MEL_BIN_COUNT, dtype=np.float64), QuantParams())
    expected = quantize_scalar(math.log(LOG_EPSILON), QuantParams())
    assert all(v == expected for v in out)


def test_log_then_quantize_unit_input_produces_log_of_one() -> None:
    # log(1.0) = 0 → quantizes to zero_point exactly under default
    # quant. Pin this so any drift in log/quant order surfaces.
    out = log_then_quantize(np.ones(MEL_BIN_COUNT, dtype=np.float64), QuantParams())
    assert all(v == -25 for v in out)


def test_log_then_quantize_rejects_wrong_shape() -> None:
    with pytest.raises(ValueError, match="shape"):
        log_then_quantize(np.zeros(MEL_BIN_COUNT - 1, dtype=np.float64), QuantParams())


def test_frontend_emits_one_frame_per_full_window_when_primed() -> None:
    # First WINDOW_SAMPLES → one frame; then every HOP_SAMPLES → one
    # more frame. A 480 + 160 + 160 + 160 sample push should emit
    # 4 frames (initial + 3 hops).
    fe = MelFrontend()
    n = WINDOW_SAMPLES + 3 * HOP_SAMPLES
    pcm = np.zeros(n, dtype=np.int16)
    frames = fe.push_samples(pcm)
    assert len(frames) == 4
    # All silence → all frames identical, all set to
    # quantize(log(LOG_EPSILON)).
    expected = quantize_scalar(math.log(LOG_EPSILON), QuantParams())
    for f in frames:
        assert f.shape == (MEL_BIN_COUNT,)
        assert all(v == expected for v in f)


def test_frontend_chunk_size_invariance() -> None:
    # Same PCM split into different chunk shapes must produce the
    # exact same frame sequence — the ring/hop state has to handle
    # arbitrary push sizes.
    rng = np.random.default_rng(seed=42)
    pcm = rng.integers(-5000, 5000, size=WINDOW_SAMPLES + 5 * HOP_SAMPLES, dtype=np.int16)
    fe_a = MelFrontend()
    a = fe_a.push_samples(pcm)
    # Chunk into 73-sample pieces (deliberately not aligned to hop or
    # window boundaries) and re-feed.
    fe_b = MelFrontend()
    chunked: list[np.ndarray] = []
    for start in range(0, pcm.size, 73):
        chunked += fe_b.push_samples(pcm[start : start + 73])
    assert len(a) == len(chunked)
    for fa, fb in zip(a, chunked, strict=True):
        assert np.array_equal(fa, fb)


def test_frontend_reset_clears_ring_state() -> None:
    fe = MelFrontend()
    pcm = np.ones(WINDOW_SAMPLES, dtype=np.int16) * 10_000
    fe.push_samples(pcm)
    fe.reset()
    # After reset, the first WINDOW_SAMPLES of zeros should produce
    # an all-silence frame identical to a fresh-frontend run.
    silence = np.zeros(WINDOW_SAMPLES, dtype=np.int16)
    frames_after = fe.push_samples(silence)
    fresh = MelFrontend()
    frames_fresh = fresh.push_samples(silence)
    assert len(frames_after) == len(frames_fresh) == 1
    assert np.array_equal(frames_after[0], frames_fresh[0])


def test_frontend_rejects_non_int16_input() -> None:
    fe = MelFrontend()
    with pytest.raises(ValueError, match="int16"):
        fe.push_samples(np.zeros(WINDOW_SAMPLES, dtype=np.float32))


def test_process_pcm_returns_2d_matrix() -> None:
    fe = MelFrontend()
    pcm = np.zeros(WINDOW_SAMPLES + 5 * HOP_SAMPLES, dtype=np.int16)
    matrix = fe.process_pcm(pcm)
    assert matrix.dtype == np.int8
    assert matrix.shape == (6, MEL_BIN_COUNT)


def test_process_pcm_returns_empty_for_undersized_input() -> None:
    # Less than one window → no frames.
    fe = MelFrontend()
    pcm = np.zeros(WINDOW_SAMPLES - 1, dtype=np.int16)
    matrix = fe.process_pcm(pcm)
    assert matrix.shape == (0, MEL_BIN_COUNT)


def test_frontend_sine_wave_concentrates_energy_in_expected_mel_bin() -> None:
    # 1 kHz pure tone should produce mel energy concentrated in the
    # bins covering 1 kHz, not at DC or near Nyquist. This is a sanity
    # check that the FFT + filterbank are wired in the right order.
    duration_s = 0.5
    n = int(duration_s * SAMPLE_RATE_HZ)
    t = np.arange(n) / SAMPLE_RATE_HZ
    amplitude = 10_000  # well within int16 range
    pcm = (amplitude * np.sin(2.0 * np.pi * 1000.0 * t)).astype(np.int16)
    fe = MelFrontend()
    matrix = fe.process_pcm(pcm)
    # Sum across time to find which mel bin holds the most energy
    # (we want consistent peak, not just any single frame's peak).
    # Convert back to float so int8 saturation doesn't dominate.
    aggregated = matrix.astype(np.int32).sum(axis=0)
    peak_bin = int(np.argmax(aggregated))
    # 1 kHz on the mel scale (htk) is ~999 mel. Our 40-bin filterbank
    # spans ~155 to ~2840 mel — 1 kHz lands somewhere mid-band.
    # The bin index can shift by ±2 depending on rounding; assert it
    # falls in a generous window around the expected centre.
    mel_lower = hz_to_mel(MEL_LOWER_HZ)
    mel_upper = hz_to_mel(MEL_UPPER_HZ)
    target_mel = hz_to_mel(1000.0)
    target_bin = round((target_mel - mel_lower) / (mel_upper - mel_lower) * (MEL_BIN_COUNT + 1) - 1)
    assert abs(peak_bin - target_bin) <= 3, f"peak at {peak_bin}, expected near {target_bin}"
