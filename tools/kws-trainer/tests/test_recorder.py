"""Unit tests for the kws_trainer.recorder primitives.

Tests focus on the pure functions — frame accumulation, WAV header
shape, silence helper — so they run without a live UDP socket. The
``record`` function itself is exercised via an integration test that
sends frames over a loopback socket.
"""

from __future__ import annotations

import io
import socket
import struct
import threading
import time
import wave
from pathlib import Path

import pytest

from kws_trainer.recorder import (
    CHANNELS,
    FRAME_BYTES,
    FRAME_SAMPLES,
    SAMPLE_RATE_HZ,
    SAMPLE_WIDTH_BYTES,
    RecordingStats,
    accumulate_frame,
    record,
    silence_pcm,
    write_wav,
    write_wav_to_bytes,
)


def _frame(value: int = 0) -> bytes:
    return struct.pack(f"<{FRAME_SAMPLES}h", *([value] * FRAME_SAMPLES))


class TestAccumulateFrame:
    def test_accepts_correctly_sized_frame(self) -> None:
        buf = bytearray()
        stats = RecordingStats()
        accumulate_frame(buf, _frame(100), stats)
        assert len(buf) == FRAME_BYTES
        assert stats.frames_received == 1
        assert stats.samples_written == FRAME_SAMPLES
        assert stats.unexpected_payloads == 0

    def test_drops_short_payload(self) -> None:
        buf = bytearray()
        stats = RecordingStats()
        accumulate_frame(buf, b"\x00" * 100, stats)
        assert len(buf) == 0
        assert stats.frames_received == 0
        assert stats.unexpected_payloads == 1
        assert stats.unexpected_sizes == [100]

    def test_drops_oversize_payload(self) -> None:
        buf = bytearray()
        stats = RecordingStats()
        accumulate_frame(buf, b"\x00" * (FRAME_BYTES + 1), stats)
        assert len(buf) == 0
        assert stats.frames_received == 0
        assert stats.unexpected_payloads == 1

    def test_dedupes_unexpected_sizes(self) -> None:
        # Same wrong size showing up repeatedly shouldn't bloat the
        # diagnostic list — bounded record so a broadcast storm can't
        # leak memory.
        buf = bytearray()
        stats = RecordingStats()
        for _ in range(50):
            accumulate_frame(buf, b"\x00" * 200, stats)
        assert stats.unexpected_payloads == 50
        assert stats.unexpected_sizes == [200]

    def test_caps_distinct_size_diagnostic(self) -> None:
        buf = bytearray()
        stats = RecordingStats()
        # 20 distinct sizes; the helper should remember the first 16.
        for size in range(100, 120):
            accumulate_frame(buf, b"\x00" * size, stats)
        assert len(stats.unexpected_sizes) == 16

    def test_multiple_frames_concatenate(self) -> None:
        buf = bytearray()
        stats = RecordingStats()
        accumulate_frame(buf, _frame(1), stats)
        accumulate_frame(buf, _frame(2), stats)
        accumulate_frame(buf, _frame(3), stats)
        assert len(buf) == 3 * FRAME_BYTES
        assert stats.frames_received == 3
        assert stats.samples_written == 3 * FRAME_SAMPLES


class TestWavWriter:
    def test_round_trip_silence(self, tmp_path: Path) -> None:
        pcm = silence_pcm(1.0)
        path = tmp_path / "out.wav"
        write_wav(pcm, str(path))
        with wave.open(str(path), "rb") as r:
            assert r.getnchannels() == CHANNELS
            assert r.getsampwidth() == SAMPLE_WIDTH_BYTES
            assert r.getframerate() == SAMPLE_RATE_HZ
            assert r.getnframes() == SAMPLE_RATE_HZ
            # Whole file reads back as silence.
            assert r.readframes(r.getnframes()) == pcm

    def test_refuses_empty(self, tmp_path: Path) -> None:
        with pytest.raises(ValueError, match="empty"):
            write_wav(b"", str(tmp_path / "out.wav"))

    def test_in_memory_writer_emits_riff_header(self) -> None:
        pcm = silence_pcm(0.1)
        wav_bytes = write_wav_to_bytes(pcm)
        # RIFF header + WAVE form-type — sanity that we're producing
        # something a downstream wave reader can parse.
        assert wav_bytes[:4] == b"RIFF"
        assert wav_bytes[8:12] == b"WAVE"
        # Round-trip through wave.open to confirm the format chunk is
        # well-formed.
        with wave.open(io.BytesIO(wav_bytes), "rb") as r:
            assert r.getnchannels() == CHANNELS
            assert r.getframerate() == SAMPLE_RATE_HZ
            assert r.getsampwidth() == SAMPLE_WIDTH_BYTES

    def test_truncates_trailing_partial_sample(self, tmp_path: Path) -> None:
        # 3 bytes = 1 whole s16 sample + 1 dangling byte. The writer
        # should drop the dangling byte rather than corrupt the WAV.
        path = tmp_path / "out.wav"
        write_wav(b"\x00\x00\x7f", str(path))
        with wave.open(str(path), "rb") as r:
            assert r.getnframes() == 1


class TestSilenceHelper:
    def test_zero_duration_returns_empty(self) -> None:
        assert silence_pcm(0) == b""

    def test_one_second_is_16k_samples(self) -> None:
        pcm = silence_pcm(1.0)
        assert len(pcm) == SAMPLE_RATE_HZ * SAMPLE_WIDTH_BYTES
        assert pcm == b"\x00" * len(pcm)


class TestRecordIntegration:
    """End-to-end test that drives ``record`` with real loopback UDP.

    Picks an OS-assigned port to avoid collisions with anything else
    running on the test host (CI / dev machine).
    """

    def test_record_captures_frames_sent_to_loopback(self) -> None:
        # Bind a probe socket to grab a free port, hand it to record().
        # We use a Thread that fires three frames at 127.0.0.1:<port>
        # then quits.
        probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
        probe.close()

        def sender() -> None:
            # Tiny pause so the recorder's bind happens first; without
            # it, the first datagram might land before the receiver
            # socket exists and silently disappear.
            time.sleep(0.1)
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
                for value in (10, 20, 30):
                    s.sendto(_frame(value), ("127.0.0.1", port))
                    time.sleep(0.05)

        t = threading.Thread(target=sender, daemon=True)
        t.start()

        pcm, stats = record(
            listen_port=port,
            duration_seconds=0.5,
            bind_addr="127.0.0.1",
        )
        t.join(timeout=1.0)

        assert stats.frames_received == 3, stats
        assert stats.unexpected_payloads == 0
        assert len(pcm) == 3 * FRAME_BYTES

    def test_record_ignores_wrong_sized_datagrams(self) -> None:
        probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
        probe.close()

        def sender() -> None:
            time.sleep(0.1)
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
                s.sendto(_frame(0), ("127.0.0.1", port))  # good
                s.sendto(b"hello", ("127.0.0.1", port))  # nonsense
                s.sendto(_frame(0), ("127.0.0.1", port))  # good

        t = threading.Thread(target=sender, daemon=True)
        t.start()

        pcm, stats = record(
            listen_port=port,
            duration_seconds=0.5,
            bind_addr="127.0.0.1",
        )
        t.join(timeout=1.0)

        assert stats.frames_received == 2
        assert stats.unexpected_payloads == 1
        assert len(pcm) == 2 * FRAME_BYTES

    def test_record_with_zero_duration_rejects(self) -> None:
        with pytest.raises(ValueError, match="duration"):
            record(listen_port=1, duration_seconds=0)
