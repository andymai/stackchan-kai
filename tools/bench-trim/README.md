# stackchan-bench-trim

Host tooling that parses the servo calibration bench output and suggests
pan/tilt trim values for `/sd/STACKCHAN.RON`. Pure stdlib — no ML deps.

## What it does

`just bench` flashes a firmware that sweeps each servo axis through a
fixed pattern and logs one row per step over defmt:

```text
bench pan: cmd=-30.00 raw_pos=547 actual_deg=+14.37 delta=+44.37
```

`delta = actual - cmd`. A systematic non-zero mean delta is a trim
offset; this tool computes the corrected `head.pan_trim_deg` /
`head.tilt_trim_deg` to paste into `STACKCHAN.RON`, and validates that
the post-trim residuals cluster near zero. If an axis's delta tracks
*opposite* to the command (slope ≈ -2), no trim can fix it — the tool
flags a `*_DIRECTION` flip in `crates/stackchan-firmware/src/head.rs`
instead.

## Workflow

```bash
# 1. Flash + capture the bench sweep (see CLAUDE.md for the tmux recipe).
just bench | tee /tmp/scfmr.log

# 2. Parse, passing the CURRENT trims from STACKCHAN.RON so the tool can
#    compute the new values.
bench-trim --input /tmp/scfmr.log --pan-trim 0.0 --tilt-trim 49.0
```

Or pipe a live capture straight in: `... | bench-trim`. With no
`--input`, the tool reads stdin.

Exit codes (mirroring `kws-eval`): `0` = all axes within tolerance,
`1` = an axis is out of tolerance (suggestion still printed), `2` = no
parseable steps / bad input.

## Options

- `--input PATH` — capture file to parse; reads stdin when omitted.
- `--pan-trim` / `--tilt-trim` — current `STACKCHAN.RON` values
  (defaults: `0.0` / `49.0`, matching the firmware compile-time trims).
- `--tolerance` — max acceptable post-trim residual in degrees
  (default `2.0`).
- `-v/--verbose` — log at INFO.

## Note on firmware constants

`POSITION_CENTER`, `POSITION_PER_DEGREE`, and the default trims are
mirrored from `crates/scservo/src/lib.rs` and
`crates/stackchan-firmware/src/head.rs`. A test pins the expected values
so a firmware change forces a conscious update here.

Live-serial capture is intentionally out of scope: parse-from-file/stdin
keeps the tool host-pure and composes with the existing `/tmp/scfmr.log`
capture workflow.

## Development

```bash
uv sync
uv run ruff check .
uv run ruff format --check .
uv run mypy
uv run pytest
```
