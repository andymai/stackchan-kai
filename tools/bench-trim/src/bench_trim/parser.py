"""Parser for the servo calibration bench's defmt output.

The ``just bench`` firmware (``crates/stackchan-firmware/examples/bench.rs``)
emits one row per sweep step:

```text
bench pan: cmd=-30.00 raw_pos=547 actual_deg=+14.37 delta=+44.37
```

defmt prefixes each line with a timestamp (``1234 ms ``) and/or a level
token. The parser locates the ``bench <axis>:`` marker anywhere in the
line and pulls the four ``key=value`` fields by name, so it tolerates the
prefix and any field-format quirks (``-30`` vs ``-30.00``). Warn/error
rows the bench emits on a failed or timed-out readback are counted as
dropped steps rather than parsed; the ``bench complete`` banner and the
version line are ignored.
"""

from __future__ import annotations

import re
from collections.abc import Iterable
from dataclasses import dataclass, field

# A bench step line, located anywhere in the (possibly prefixed) row.
_STEP_RE = re.compile(
    r"bench\s+(?P<axis>pan|tilt):\s+"
    r"cmd=(?P<cmd>[-+]?\d+(?:\.\d+)?)\s+"
    r"raw_pos=(?P<raw>\d+)\s+"
    r"actual_deg=(?P<actual>[-+]?\d+(?:\.\d+)?)\s+"
    r"delta=(?P<delta>[-+]?\d+(?:\.\d+)?)"
)

# A bench row tagged with an axis but carrying a failure marker instead
# of readings — set_pose failure, read error, or readback timeout.
_DROP_RE = re.compile(r"bench\s+(?:pan|tilt):.*(?:failed|err|timed out)")

# Any axis-tagged bench row, used to count rows that matched neither a
# step nor a known failure marker.
_AXIS_TAG_RE = re.compile(r"bench\s+(?:pan|tilt):")


@dataclass(frozen=True)
class BenchStep:
    """One parsed sweep step. ``delta_deg = actual_deg - cmd_deg``."""

    axis: str
    cmd_deg: float
    raw_pos: int
    actual_deg: float
    delta_deg: float


@dataclass
class ParseResult:
    """Outcome of parsing a bench capture.

    ``dropped`` counts axis-tagged failure rows (failed move, read error,
    timeout); ``unparsed`` counts bench-tagged rows that matched neither
    the step shape nor a known failure marker (e.g. a truncated capture).
    """

    steps: list[BenchStep] = field(default_factory=list)
    dropped: int = 0
    unparsed: int = 0


def parse_bench_lines(lines: Iterable[str]) -> ParseResult:
    """Parse bench output rows into a :class:`ParseResult`.

    Tolerant by design: a malformed but bench-tagged row is counted as
    ``unparsed`` rather than raising, so a truncated or noisy capture
    still yields a usable suggestion from the rows that did parse.
    """
    result = ParseResult()
    for line in lines:
        m = _STEP_RE.search(line)
        if m is not None:
            result.steps.append(
                BenchStep(
                    axis=m.group("axis"),
                    cmd_deg=float(m.group("cmd")),
                    raw_pos=int(m.group("raw")),
                    actual_deg=float(m.group("actual")),
                    delta_deg=float(m.group("delta")),
                )
            )
            continue
        if _DROP_RE.search(line):
            result.dropped += 1
            continue
        if _AXIS_TAG_RE.search(line):
            result.unparsed += 1
    return result
