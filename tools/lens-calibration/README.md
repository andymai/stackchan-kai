# stackchan-lens-calibration

Host tooling that parses the look-toward-motion tracker bench output and
checks the camera `flip_x` / `flip_y` mounting flags for
`/sd/STACKCHAN.RON`. Pure stdlib — no ML deps.

## What it does

`just tracker-bench` flashes a firmware that runs the `tracker` crate
over every camera frame and logs one row per decision over defmt:

```text
tracker-bench: motion=Tracking fired=12 centroid=(420/1000, -310/1000) target_pose=(240/100°, 160/100°)
```

The centroid is already flip-adjusted, so the within-frame
centroid→pose sign can't reveal a wrong flip. What the capture exposes
is the closed-loop behaviour across consecutive Tracking decisions:

- A **correctly-mounted** axis has negative feedback — the head aims at
  the motion, so the centroid component decays toward zero.
- A **wrongly-flipped** axis has positive feedback — the head chases
  away, so the centroid component grows and the pose runs to its clamp.

This tool measures that feedback sign per axis and flags the `flip_x` /
`flip_y` flag to toggle when an axis is chasing away. It also reports
centroid coverage and pose span as context for the current FOV.

## Workflow

```bash
# 1. Flash + capture the tracker bench (see CLAUDE.md for the tmux recipe).
#    Wave at the camera so the tracker has sustained motion to chase.
just tracker-bench | tee /tmp/scfmr.log

# 2. Parse, passing the CURRENT tracker flags from STACKCHAN.RON.
lens-calibration --input /tmp/scfmr.log --fov-h 62 --fov-v 49
```

Or pipe a live capture straight in: `... | lens-calibration`. With no
`--input`, the tool reads stdin.

Exit codes (mirroring `bench-trim`): `0` = mounting looks correct,
`1` = a flip flag should change (suggestion still printed), `2` = no
parseable frames / bad input.

## Options

- `--input PATH` — capture file to parse; reads stdin when omitted.
- `--flip-x` / `--flip-y` — set when the *current* `STACKCHAN.RON`
  already has that flip on, so the tool reports the right toggle.
- `--fov-h` / `--fov-v` — current `tracker.fov_h_deg` / `fov_v_deg`
  (defaults `62.0` / `49.0`), shown alongside coverage.
- `-v/--verbose` — log at INFO.

## Scope

The bench logs no ground-truth target position, so an *optimal* FOV
cannot be solved from the capture alone — this tool does flip detection
and coverage diagnostics only. Treat the flip verdict as a heuristic: it
needs sustained motion that actually moves the target across several
frames, and sparse / low-motion captures are reported as
insufficient-data rather than guessed.

## Note on firmware constants

The default FOV and flip values are mirrored from
`crates/stackchan-net/src/config.rs` (`TrackerSettings::DEFAULT`) and
`crates/tracker/src/lib.rs` (`TrackerConfig::DEFAULT`). A test pins the
expected values so a firmware change forces a conscious update here.

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
