from __future__ import annotations

from bench_trim.parser import parse_bench_lines

# Verbatim format from crates/stackchan-firmware/examples/bench.rs, with
# a defmt `<ms> ms` timestamp prefix on some rows and not others.
CAPTURE = """\
1200 ms bench pan: cmd=-30.00 raw_pos=410 actual_deg=-29.91 delta=+0.09
bench pan: cmd=0.00 raw_pos=512 actual_deg=+0.00 delta=+0.00
1500 ms INFO bench pan: cmd=30.00 raw_pos=614 actual_deg=+29.91 delta=-0.09
bench tilt: cmd=-20.00 raw_pos=547 actual_deg=+24.37 delta=+44.37
bench tilt: read_position err at cmd=0.00: Timeout
bench tilt: cmd=20.00 raw_pos=683 actual_deg=+64.10 delta=+44.10
bench tilt: read_position timed out at cmd=10.00
1600 ms bench complete — re-flash main firmware with `just flash` to resume normal boot
"""


def test_parses_step_rows_with_and_without_prefix() -> None:
    result = parse_bench_lines(CAPTURE.splitlines())
    assert len(result.steps) == 5
    pan = [s for s in result.steps if s.axis == "pan"]
    tilt = [s for s in result.steps if s.axis == "tilt"]
    assert len(pan) == 3
    assert len(tilt) == 2


def test_field_values_extracted() -> None:
    result = parse_bench_lines(CAPTURE.splitlines())
    first = result.steps[0]
    assert first.axis == "pan"
    assert first.cmd_deg == -30.00
    assert first.raw_pos == 410
    assert first.actual_deg == -29.91
    assert first.delta_deg == 0.09


def test_failure_rows_counted_as_dropped() -> None:
    result = parse_bench_lines(CAPTURE.splitlines())
    assert result.dropped == 2


def test_complete_banner_ignored() -> None:
    result = parse_bench_lines(["bench complete — done"])
    assert result.steps == []
    assert result.dropped == 0
    assert result.unparsed == 0


def test_malformed_bench_row_counted_unparsed_not_raised() -> None:
    result = parse_bench_lines(["bench pan: cmd=-30.00 raw_pos="])
    assert result.steps == []
    assert result.unparsed == 1


def test_integer_cmd_format_tolerated() -> None:
    result = parse_bench_lines(["bench pan: cmd=-30 raw_pos=410 actual_deg=-30 delta=0"])
    assert len(result.steps) == 1
    assert result.steps[0].cmd_deg == -30.0
    assert result.steps[0].delta_deg == 0.0
