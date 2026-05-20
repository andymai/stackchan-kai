"""UDP-frame → PCM-buffer + WAV-writer primitives.

The firmware's ``audio_debug_udp_target`` task forwards each 20 ms
ES7210 frame as a raw little-endian ``s16`` UDP datagram, 320 samples
(640 bytes) per frame. This module reassembles them.

There's no header, no sequence number, no timestamp — the firmware
trusts UDP frame ordering on a LAN and accepts that occasional drops
will surface as silent gaps in the recording. The recorder logs each
unexpected datagram size so a stray packet (mDNS noise, IP-broadcast
discovery, etc.) is visible rather than silently misinterpreted as
audio.
"""

from __future__ import annotations

import io
import logging
import socket
import struct
import time
import wave
from dataclasses import dataclass, field

_LOG = logging.getLogger("kws_trainer.recorder")

# Wire constants — these match the firmware's `audio_debug_udp_target`
# task in `crates/stackchan-firmware/src/audio_debug.rs`. Don't drift
# from there.
SAMPLE_RATE_HZ = 16_000
FRAME_SAMPLES = 320  # 20 ms @ 16 kHz
FRAME_BYTES = FRAME_SAMPLES * 2  # s16 LE
SAMPLE_WIDTH_BYTES = 2
CHANNELS = 1


@dataclass
class RecordingStats:
    """Per-session counters surfaced after a recording finishes.

    A non-zero ``unexpected_payloads`` means the receiver got something
    other than a 640-byte audio frame — typically a stray broadcast or
    a misconfigured peer pointing at this port. A non-zero
    ``elapsed_seconds`` close to the target duration means the firmware
    is keeping up; large discrepancies suggest UDP loss or socket
    backpressure.
    """

    frames_received: int = 0
    unexpected_payloads: int = 0
    samples_written: int = 0
    elapsed_seconds: float = 0.0
    unexpected_sizes: list[int] = field(default_factory=list)

    @property
    def expected_samples_for_elapsed(self) -> int:
        return int(self.elapsed_seconds * SAMPLE_RATE_HZ)


def accumulate_frame(buffer: bytearray, payload: bytes, stats: RecordingStats) -> None:
    """Append one UDP payload to ``buffer`` if it's a well-shaped audio
    frame, otherwise log + bump ``unexpected_payloads`` and drop it.

    Kept pure (no socket I/O) so the size-validation behaviour can be
    unit-tested without a live UDP server.
    """
    if len(payload) != FRAME_BYTES:
        stats.unexpected_payloads += 1
        # Keep a bounded record of distinct sizes so the operator can
        # diagnose what's actually arriving; unbounded would let a
        # broadcast storm leak memory.
        if len(payload) not in stats.unexpected_sizes and len(stats.unexpected_sizes) < 16:
            stats.unexpected_sizes.append(len(payload))
        _LOG.debug(
            "dropped non-audio UDP payload size=%d (expected %d)",
            len(payload),
            FRAME_BYTES,
        )
        return
    buffer.extend(payload)
    stats.frames_received += 1
    stats.samples_written += FRAME_SAMPLES


def write_wav(pcm: bytes, path: str) -> None:
    """Write ``pcm`` (raw s16 LE bytes) to ``path`` as a 16 kHz mono WAV.

    Refuses to write an empty buffer — that's a sign the recorder
    didn't receive anything (firmware off, wrong port, firewall) and
    silently emitting a zero-byte WAV would mask the failure. The
    empty guard runs *after* the odd-length truncation so a 1-byte
    input (which becomes empty after truncation) raises the same
    error a 0-byte input does.
    """
    pcm = _trim_to_whole_samples(pcm)
    if not pcm:
        raise ValueError("refusing to write empty WAV — no audio captured")
    with wave.open(path, "wb") as out:
        out.setnchannels(CHANNELS)
        out.setsampwidth(SAMPLE_WIDTH_BYTES)
        out.setframerate(SAMPLE_RATE_HZ)
        out.writeframes(pcm)


def write_wav_to_bytes(pcm: bytes) -> bytes:
    """In-memory counterpart of [`write_wav`] for tests that want to
    inspect the produced header without touching the filesystem."""
    pcm = _trim_to_whole_samples(pcm)
    if not pcm:
        raise ValueError("refusing to write empty WAV — no audio captured")
    buf = io.BytesIO()
    with wave.open(buf, "wb") as out:
        out.setnchannels(CHANNELS)
        out.setsampwidth(SAMPLE_WIDTH_BYTES)
        out.setframerate(SAMPLE_RATE_HZ)
        out.writeframes(pcm)
    return buf.getvalue()


def _trim_to_whole_samples(pcm: bytes) -> bytes:
    """Drop a trailing partial-sample byte if one slipped in. UDP
    datagrams are atomic so a torn frame is impossible in practice,
    but the explicit truncation here lets callers (or tests with
    hand-crafted inputs) get a clean error instead of a corrupt WAV.
    """
    extra = len(pcm) % SAMPLE_WIDTH_BYTES
    return pcm if extra == 0 else pcm[: len(pcm) - extra]


def silence_pcm(seconds: float) -> bytes:
    """Build ``seconds`` of silence as s16 LE bytes. Test helper —
    lets fixtures construct a deterministic payload without monkey-
    patching the UDP receive path."""
    samples = int(seconds * SAMPLE_RATE_HZ)
    return struct.pack(f"<{samples}h", *([0] * samples))


def record(
    *,
    listen_port: int,
    duration_seconds: float,
    bind_addr: str = "0.0.0.0",
) -> tuple[bytes, RecordingStats]:
    """Listen on ``bind_addr:listen_port`` for ``duration_seconds``,
    reassemble incoming UDP audio frames, return ``(pcm, stats)``.

    Blocking; intended to run from the CLI's main thread. The socket
    timeout is set to a short slice of the remaining window so the
    loop exits promptly when the deadline passes even if no datagrams
    are arriving.
    """
    if duration_seconds <= 0:
        raise ValueError(f"duration must be positive, got {duration_seconds}")

    pcm = bytearray()
    stats = RecordingStats()
    deadline = time.monotonic() + duration_seconds

    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind((bind_addr, listen_port))
        # 100 ms gives us 5 polls per frame's worth of jitter, plenty
        # to detect the wall-clock deadline without dropping bursts.
        sock.settimeout(0.1)
        start = time.monotonic()
        while time.monotonic() < deadline:
            try:
                # 65535 = max UDP payload. A smaller buffer would
                # silently truncate any stray oversized datagram to
                # exactly the buffer size, which would mislead
                # diagnostics (stats.unexpected_sizes would all read
                # the buffer size instead of the real payload).
                payload, _addr = sock.recvfrom(65535)
            except TimeoutError:
                continue
            accumulate_frame(pcm, payload, stats)
        stats.elapsed_seconds = time.monotonic() - start

    return bytes(pcm), stats
