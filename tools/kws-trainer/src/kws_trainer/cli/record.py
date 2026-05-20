"""``kws-record`` CLI — capture a fixed-duration WAV from a running
firmware's ``audio_debug_udp_target`` stream.

Operator workflow:

1. Set ``behavior.audio_debug_udp_target = "<this-host-ip>:5005"`` in
   ``STACKCHAN.RON`` and reboot the device.
2. Run ``kws-record --listen-port 5005 --duration 5 --output sample.wav``.
3. Speak the wake phrase during the recording window.
4. Repeat to build a dataset for the eventual training step.
"""

from __future__ import annotations

import argparse
import logging
import sys
from collections.abc import Sequence

from kws_trainer.recorder import (
    FRAME_SAMPLES,
    SAMPLE_RATE_HZ,
    record,
    write_wav,
)

_LOG = logging.getLogger("kws_trainer.cli.record")


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="kws-record",
        description=(
            "Capture a fixed-duration WAV from a stackchan-kai device's "
            "audio_debug_udp_target stream."
        ),
    )
    p.add_argument(
        "--listen-port",
        type=int,
        required=True,
        help="UDP port to bind. Must match the port half of the firmware's "
        "behavior.audio_debug_udp_target value.",
    )
    p.add_argument(
        "--duration",
        type=float,
        default=5.0,
        help="Recording window in seconds (default: 5.0).",
    )
    p.add_argument(
        "--output",
        required=True,
        help="WAV file to write. 16 kHz mono s16 LE — the format "
        "microWakeWord's training pipeline ingests directly.",
    )
    # bind 0.0.0.0 by default — operator-facing tool, binding broadly
    # is the expected behaviour. Pin to a specific IP via --bind-addr
    # on multi-homed hosts.
    p.add_argument(
        "--bind-addr",
        default="0.0.0.0",
        help="Interface to bind. Default 0.0.0.0 listens on every "
        "interface; pin to a specific IP if your host is multi-homed.",
    )
    p.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="Log every dropped non-audio payload (useful for diagnosing "
        "wrong-port / multi-tenant networks).",
    )
    return p


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    _LOG.info(
        "listening on %s:%d for %.1fs — speak the wake phrase",
        args.bind_addr,
        args.listen_port,
        args.duration,
    )
    pcm, stats = record(
        listen_port=args.listen_port,
        duration_seconds=args.duration,
        bind_addr=args.bind_addr,
    )

    if stats.frames_received == 0:
        _LOG.error(
            "no audio frames received; check that the firmware's "
            "behavior.audio_debug_udp_target is set to a reachable "
            "<host>:%d and that no firewall is dropping UDP",
            args.listen_port,
        )
        return 2

    # Compare what arrived vs what we expected from the firmware's
    # nominal 50 frames/sec cadence. Surfaced as a warn so the
    # operator sees obvious drops; not an error since one or two
    # missing frames on a busy LAN is normal.
    expected_frames = int(args.duration * SAMPLE_RATE_HZ / FRAME_SAMPLES)
    if stats.frames_received < expected_frames * 0.95:
        _LOG.warning(
            "received %d/%d expected frames (%.0f%%) — UDP loss?",
            stats.frames_received,
            expected_frames,
            100.0 * stats.frames_received / expected_frames,
        )

    if stats.unexpected_payloads > 0:
        _LOG.warning(
            "dropped %d non-audio UDP payload(s) (sizes seen: %s)",
            stats.unexpected_payloads,
            stats.unexpected_sizes,
        )

    write_wav(bytes(pcm), args.output)
    _LOG.info(
        "wrote %s — %d frames (%.2f s of audio captured in %.2f s window)",
        args.output,
        stats.frames_received,
        stats.samples_written / SAMPLE_RATE_HZ,
        stats.elapsed_seconds,
    )
    return 0


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
