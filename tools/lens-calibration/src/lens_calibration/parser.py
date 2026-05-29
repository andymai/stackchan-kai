"""Parser for the look-toward-motion tracker bench's defmt output.

The ``just tracker-bench`` firmware
(``crates/stackchan-firmware/examples/tracker_bench.rs``) runs the
[`tracker::Tracker`] over every camera frame and logs one row per
decision via defmt:

```text
tracker-bench: motion=Tracking fired=12 centroid=(420/1000, -310/1000) \
               target_pose=(240/100°, 160/100°)
```

Centroid and pose are emitted as scaled integers (``* 1000`` / ``* 100``)
because the firmware's defmt build does not enable the ``float`` feature.
defmt prefixes each line with a timestamp (``1234 ms ``) and/or a level
token; the parser locates the ``tracker-bench:`` marker anywhere in the
line and pulls the fields by name, so it tolerates the prefix.

The bench only logs on motion / decision transitions, so the captured
frames are **not** contiguous in time — consumers must treat each parsed
frame as an independent decision and never assume a fixed cadence.
Rows that carry the marker but match neither the decision shape (the
startup banner, the version line, a truncated capture) are counted as
``unparsed`` rather than raising.
"""

from __future__ import annotations

import re
from collections.abc import Iterable
from dataclasses import dataclass, field

# A tracker decision row, located anywhere in the (possibly prefixed) line.
# Centroid is scaled by 1000, pose by 100, matching `log_outcome`.
_FRAME_RE = re.compile(
    r"tracker-bench:\s+"
    r"motion=(?P<motion>\w+)\s+"
    r"fired=(?P<fired>\d+)\s+"
    r"centroid=\((?P<nx>[-+]?\d+)/1000,\s*(?P<ny>[-+]?\d+)/1000\)\s+"
    r"target_pose=\((?P<pan>[-+]?\d+)/100°?,\s*(?P<tilt>[-+]?\d+)/100°?\)"
)

# Any tracker-bench-tagged row, used to count rows that matched neither a
# decision frame nor are otherwise expected (banner / version line).
_TAG_RE = re.compile(r"tracker-bench:")


@dataclass(frozen=True)
class BenchFrame:
    """One parsed tracker decision.

    ``nx`` / ``ny`` are the post-flip normalised centroid in ``[-1, 1]``;
    ``pan_deg`` / ``tilt_deg`` are the resulting commanded target pose.
    """

    motion: str
    fired: int
    nx: float
    ny: float
    pan_deg: float
    tilt_deg: float


@dataclass
class ParseResult:
    """Outcome of parsing a tracker-bench capture.

    ``unparsed`` counts tracker-bench-tagged rows that matched neither the
    decision shape nor the expected banner/version lines (e.g. a truncated
    or noisy capture).
    """

    frames: list[BenchFrame] = field(default_factory=list)
    unparsed: int = 0


def parse_tracker_lines(lines: Iterable[str]) -> ParseResult:
    """Parse tracker-bench output rows into a :class:`ParseResult`.

    Tolerant by design: a malformed but tracker-bench-tagged row is
    counted as ``unparsed`` rather than raising, so a truncated or noisy
    capture still yields a usable analysis from the rows that did parse.
    """
    result = ParseResult()
    for line in lines:
        m = _FRAME_RE.search(line)
        if m is not None:
            result.frames.append(
                BenchFrame(
                    motion=m.group("motion"),
                    fired=int(m.group("fired")),
                    nx=int(m.group("nx")) / 1000.0,
                    ny=int(m.group("ny")) / 1000.0,
                    pan_deg=int(m.group("pan")) / 100.0,
                    tilt_deg=int(m.group("tilt")) / 100.0,
                )
            )
            continue
        if _TAG_RE.search(line) and not _is_known_noise(line):
            result.unparsed += 1
    return result


def _is_known_noise(line: str) -> bool:
    """Rows the bench emits that are not decisions and should not count
    as unparsed: the startup banner and the camera-mode notice."""
    return ("CoreS3 boot" in line) or ("camera mode forced ON" in line)
