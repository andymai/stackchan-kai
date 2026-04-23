# stackchan-rs

## Project Structure

Cargo workspace with four crates:
- `crates/stackchan-core` — `no_std` domain library: `Avatar`, `Eye`, `Mouth`, `Modifier` trait, `Clock` trait, `Emotion`. Pure Rust, no hardware deps.
- `crates/stackchan-sim` — Headless integration tests that drive `stackchan-core` with a fake clock.
- `crates/axp2101` — Minimal AXP2101 PMU driver (I²C, embedded-hal-async).
- `crates/stackchan-firmware` — Binary crate. `no_std` + `alloc`. embassy executor on CoreS3.

## Build

```bash
cargo test                    # host-side: core, sim, axp2101
cargo clippy --workspace
cargo build -p stackchan-firmware --release   # requires espup-installed esp toolchain
espflash flash --monitor target/xtensa-esp32s3-none-elf/release/stackchan-firmware
```

## Pre-commit Hook

`.githooks/pre-commit` runs fmt, clippy (strict), host tests, doctests, workspace check, and a Cargo.lock drift guard. After cloning:

```bash
git config core.hooksPath .githooks
```

Conventional-commit check at `.githooks/commit-msg`.

## Architecture

- `stackchan-core` models the avatar as data: `Avatar { left_eye, right_eye, mouth, emotion }` plus a `Modifier` trait with `update(&mut Avatar, now: Instant)`. Time comes from the `Clock` trait so core is deterministic and host-testable.
- `stackchan-sim` constructs a `stackchan_core::Avatar` + a `FakeClock` and runs a list of `Modifier`s through hand-crafted time sequences. Golden assertions on `Eye::weight`, `Mouth::rotation`, etc.
- `stackchan-firmware` initializes AXP2101 → SPI LCD via `mipidsi` → spawns an embassy task that composes `Avatar::draw(&mut framebuffer)` into `embedded-graphics` primitives, pushes frames at ~30 FPS. `HalClock` wraps embassy-time.
- `crates/axp2101` is the minimum set of registers (ALDO1/2, BLDO1/2, power-on sequencing) needed for CoreS3 LCD + 3V3 rails.

## Conventions

Commits: conventional commits (`feat:`, `fix:`, `refactor:`, `chore:`, etc.).

PR workflow (post-v0.1.0): every change lands via PR; greptile bot review required. Pre-v0.1.0 commits may land directly on main for solo scaffold work.

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

## Config + assets

None yet. v0.1.0 avatar geometry is hardcoded in `stackchan-core::avatar`. Later releases may add RON-configurable appearance.
