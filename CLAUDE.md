# stackchan-kai

## Project Structure

Cargo workspace with six crates:
- `crates/stackchan-core` — `no_std` domain library: `Avatar`, `Eye`, `Mouth`, `Modifier` trait, `Clock` trait, `Emotion`, `Pose`. Pure Rust, no hardware deps.
- `crates/stackchan-sim` — Headless integration tests that drive `stackchan-core` with a fake clock.
- `crates/axp2101` — Minimal AXP2101 PMU driver (I²C, embedded-hal-async).
- `crates/aw9523` — Minimal AW9523 I/O-expander init (I²C, embedded-hal-async). Pulls the LCD reset pin and gates the backlight-boost rail on the CoreS3.
- `crates/scservo` — Feetech SCServo half-duplex serial driver (UART1, embedded-io-async). Drives the pan/tilt head servos.
- `crates/stackchan-firmware` — Binary crate. `no_std` + `alloc`. embassy executor on CoreS3.

## Build

```bash
cargo test                                  # host-side: core, sim, axp2101, aw9523, scservo
cargo clippy --workspace --all-targets      # host crates only (firmware excluded from default-members)

# Firmware: requires `source ~/export-esp.sh` first so the `esp` toolchain is on PATH.
# `just fmr` wraps `cargo +esp build --release` + espflash flash-and-monitor through
# `sg dialout`; see the justfile for the full recipe set.
source ~/export-esp.sh
just fmr
```

## Flashing from an agent / non-TTY shell

`just fmr` (and any `espflash …--monitor` command) invokes an interactive
input reader that fails immediately with `× Failed to initialize input
reader` when stdin is not a TTY — which is the default for any
agent-spawned bash. Run flash + monitor inside **tmux** so the espflash
process gets a real PTY:

```bash
tmux new-session -d -s scfmr 'bash -l'
tmux send-keys -t scfmr 'source ~/export-esp.sh && just fmr 2>&1 | tee /tmp/scfmr.log' Enter
# Read /tmp/scfmr.log to follow boot output without attaching to the session.
```

Reuse the same session for re-flashes: `Ctrl-C` first
(`tmux send-keys -t scfmr C-c`) to break out of the running monitor,
then re-issue `just fmr`. To pick up a running device without
resetting it, use `just reattach` instead of `just fmr`.

## Pre-commit Hook

`.githooks/pre-commit` runs fmt, clippy (strict), host tests, doctests, workspace check, and a Cargo.lock drift guard. After cloning:

```bash
git config core.hooksPath .githooks
```

Conventional-commit check at `.githooks/commit-msg`.

## Architecture

- `stackchan-core` models the avatar as data: `Avatar { left_eye, right_eye, mouth, emotion }` plus a `Modifier` trait with `update(&mut Avatar, now: Instant)`. Time comes from the `Clock` trait so core is deterministic and host-testable.
- `stackchan-sim` constructs a `stackchan_core::Avatar` + a `FakeClock` and runs a list of `Modifier`s through hand-crafted time sequences. Golden assertions on `Eye::weight`, `Mouth::rotation`, etc.
- `stackchan-firmware` initializes AXP2101 → AW9523 (releases LCD reset + enables the backlight-boost gate) → SPI LCD via `mipidsi` → SCServo head driver on UART1 → spawns an embassy render task that composes `Avatar::draw(&mut framebuffer)` into `embedded-graphics` primitives, pushes frames at ~30 FPS. `HalClock` wraps embassy-time.
- `crates/axp2101` is the minimum set of registers (ALDO1/2, BLDO1/2, DLDO1, power-on sequencing) needed for CoreS3 LCD + 3V3 rails.
- `crates/aw9523` handles the rest of the CoreS3 boot dance: pulses `P1_1` (LCD_RST) and leaves `P1_7` (backlight-boost enable) high so the LCD backlight rail actually comes up.
- `crates/scservo` is a protocol-correct Feetech driver (checksum + status frame parsing) with golden-packet tests; position readback supports the calibration bench at `crates/stackchan-firmware/examples/bench.rs`.

## Conventions

Commits: conventional commits (`feat:`, `fix:`, `refactor:`, `chore:`, etc.).

PR workflow: every change lands via PR; greptile bot review required. v0.1.0 shipped 2026-04-23 — direct-to-main commits are no longer part of the workflow.

Parallel work: `git worktree add .worktrees/<branch> <branch>` — `.worktrees/` is gitignored.

Type naming — domain-first, no redundant suffixes:
- `Avatar`, `Eye`, `Mouth`, `Emotion` (not `AvatarData`, `EyeState`)
- `EyePhase::{Open, Closed}` (not `EyeState`); fields use `.phase`
- `Modifier`, `BlinkModifier`, `BreathModifier` (not `IModifier`, `ModifierImpl`)

Unsafe code: denied at the workspace level. The firmware crate explicitly allows unsafe for linker-defined symbols and register-map pointers, gated behind per-module `#![allow(unsafe_code)]` with a comment explaining why.

No `unwrap()` / `expect()` in library code. Use `?` with typed errors (`thiserror` for host; `defmt::Format` derives on firmware errors).

## Hardware notes

- CoreS3: ESP32-S3 dual-core Xtensa, 8 MB PSRAM, 16 MB flash.
- AXP2101 at I²C address `0x34`. Must be initialized before LCD SPI pins have power.
- ILI9342C LCD, 320×240, over SPI2. Backlight on AXP2101 DLDO1.
- PSRAM heap via `esp-alloc`; internal SRAM reserved for ISR/real-time.
- `/dev/ttyACM1` is the CoreS3 USB-JTAG on Andy's machine (may shuffle with `/dev/ttyACM0` which is his Pikatea macropad — check with `udevadm info` first).
- Serial access requires `sg dialout -c '...'` wrapper on the host until `dialout` group is in the active shell's supplementary groups.
- Logs travel over RTT via the USB-JTAG probe; `probe-rs run --chip esp32s3` is the canonical flash + log path (wired up as the `cargo run` runner in the firmware crate).

## Config + assets

None yet. Avatar geometry is hardcoded in `stackchan-core::avatar`. Later releases may add RON-configurable appearance.
