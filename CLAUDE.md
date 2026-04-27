# stackchan-kai

> AI collaborators: see [AGENTS.md](AGENTS.md) for the dedicated agent playbook
> (session shapes, debugging recipes, memory pointer). This file is shared
> between humans and agents; AGENTS.md is agent-specific.

## Project Structure

Cargo workspace, organized by purpose:

**Domain + sim (host)**
- `crates/stackchan-core` — `no_std` domain library: `Entity`, `Director`, `Modifier` + `Skill` traits, `Clock`, `Emotion`, `Intent`, `Phase`. Pure Rust, no hardware deps.
- `crates/stackchan-sim` — Headless integration tests that drive `stackchan-core` with a fake clock.
- `crates/tracker` — Block-grid motion tracker for the camera path. Pure algorithm; host-testable.

**Driver crates** (`no_std`, embedded-hal-async)
- `crates/axp2101` — AXP2101 PMU driver (I²C). LDO rails, power-key timing, battery gauge.
- `crates/aw9523` — AW9523 I/O expander. CoreS3 boot dance: LCD reset pulse, backlight-boost gate.
- `crates/aw88298` — AW88298 mono Class-D amplifier (I²S TX path).
- `crates/bm8563` — BM8563 RTC with date-format helper.
- `crates/bmi270` — BMI270 6-axis IMU (accel + gyro).
- `crates/bmm150` — BMM150 magnetometer (bench-only on this unit; see hardware quirks).
- `crates/es7210` — ES7210 4-channel ADC (I²S RX path).
- `crates/ft6336u` — FT6336U capacitive touch controller.
- `crates/gc0308` — GC0308 camera SCCB init (LCD_CAM DMA path).
- `crates/ir-nec` — NEC-protocol IR decoder (RMT peripheral).
- `crates/ltr553` — LTR-553 ambient-light + proximity sensor.
- `crates/py32` — PY32 co-processor (servo-power gate, WS2812 LED ring).
- `crates/scservo` — Feetech SCServo half-duplex serial driver (UART1).
- `crates/si12t` — `Si12T` 3-zone capacitive touch (back-of-head body pads), I²C, polled at 50 ms.

**Networking + voice** (`no_std`)
- `crates/stackchan-net` — Shared wire formats: RON config schema for `STACKCHAN.RON`, HTTP request/body parsers, validators. Used by both the firmware HTTP plane and SD config bring-up.
- `crates/stackchan-tts` — Speech surface: `SpeechBackend` trait + a `BakedBackend` of compile-time PCM phrases. `no_std` + `alloc`.

**Firmware**
- `crates/stackchan-firmware` — Binary crate. `no_std` + `alloc`. Embassy executor on CoreS3.

## Build

Prefer just recipes over raw cargo — they encode the project's gates.

```bash
just check                                   # fmt + workspace clippy + host tests (host crates only)
just ci                                      # check + cargo-deny + workspace doc-lint
just msrv                                    # MSRV (rust 1.88) build of host crates

# Firmware: requires `source ~/export-esp.sh` first so the `esp` toolchain is on PATH.
source ~/export-esp.sh
just check-firmware                          # cargo +esp check
just clippy-firmware                         # strict clippy (matches CI)
just build-firmware                          # cargo +esp build --release
just fmr                                     # flash + monitor in one go
just reattach                                # attach to a running device without resetting
```

Per-bench recipes (each flashes a single example): `just bench`, `just mag-bench`, `just leds-bench`, `just aw88298-bench`, `just es7210-bench`, `just audio-bench`, `just tilt-extremes`, `just tilt-freewheel`. List all: `just`.

## Flashing from an agent / non-TTY shell

`just fmr` (and any `espflash …--monitor` command) invokes an interactive
input reader that fails immediately with `× Failed to initialize input
reader` when stdin is not a TTY — the default for any agent-spawned bash.
Run flash + monitor inside **tmux** so the espflash process gets a real PTY:

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

`.githooks/pre-commit` runs fmt, clippy (strict), host tests, doctests, workspace check, a Cargo.lock drift guard, and a README review reminder. After cloning:

```bash
git config core.hooksPath .githooks
```

Conventional-commit check at `.githooks/commit-msg`.

## Architecture

- `stackchan-core` models the desk toy as data: an `Entity { face, motor, perception, voice, mind, events, input, tick }` plus a `Director` that sorts `Modifier`s by phase + priority and ticks them each frame against the entity. Time flows in via `entity.tick.now`, stamped from the `Clock` trait so core is deterministic and host-testable.
- `stackchan-sim` constructs an `Entity` + `FakeClock` and runs the same `Director` through hand-crafted time sequences. Golden assertions on `Eye::weight`, `Mouth::mouth_open`, etc.
- `stackchan-firmware` initializes AXP2101 → AW9523 (releases LCD reset + enables backlight-boost) → SPI LCD via `mipidsi` → SCServo head driver on UART1 → spawns embassy tasks for: render (~30 FPS), head (~50 Hz), touch, IR, IMU, ambient, button, LEDs, power, audio (RX RMS + TX queue), camera, plus optional Wi-Fi + HTTP control plane when `STACKCHAN.RON` is on the SD card. Cross-task communication runs through typed `Signal<RawMutex, T>` channels — `try_take` for sensor input (latest-wins), `signal()` for output sinks. SSE fan-out uses `embassy_sync::pubsub::PubSubChannel` instead of `Signal` to support multiple subscribers.

## Conventions

**Commits:** conventional commits (`feat:`, `fix:`, `refactor:`, `chore:`, `docs:`, `test:`, `build:`, `ci:`, `perf:`, `style:`, `revert:`). Optional scope: `feat(core):`, `chore(firmware):`. Enforced via `.githooks/commit-msg`.

**Branch naming:** `<type>/<kebab-case-description>`, mirroring the commit type. Examples from history: `docs/claudemd-version-qualifier`, `refactor/rip-mag-data-only`, `feat/sim-host-visualizer`, `chore/prune-info-logs`, `fix/i2c-100khz-py32`.

**PR workflow:** every change lands via PR; greptile bot review is the soft convention. v0.1.0 shipped 2026-04-23 — direct-to-main commits are no longer part of the workflow. PR titles match the commit subject; bodies follow the existing Summary + Test plan template.

**Parallel work:** `git worktree add .worktrees/<dirname> <branch>` — `.worktrees/` is gitignored. Always run `gh pr merge` from the main repo path, not from inside a worktree (the `--delete-branch` flag pulls cwd out from under bash).

**Type naming** — domain-first, no redundant suffixes. See [docs/naming.md](docs/naming.md) for the full ruleset (Intent / Modifier / Skill / ChirpKind / OverrideSource) with citations and inventory.

**Unsafe code:** denied at the workspace level. The firmware crate explicitly allows unsafe for linker-defined symbols and register-map pointers, gated behind per-module `#![allow(unsafe_code)]` with a comment explaining why.

**No `unwrap()` / `expect()` in library code.** Use `?` with typed errors (`thiserror` for host; `defmt::Format` derives on firmware errors). See [docs/errors.md](docs/errors.md) for the typed-error catalog.

## Hardware notes

- CoreS3: ESP32-S3 dual-core Xtensa, 8 MB PSRAM, 16 MB flash.
- AXP2101 at I²C address `0x34`. Must be initialized before LCD SPI pins have power.
- ILI9342C LCD, 320×240, over SPI2. Backlight on AXP2101 BLDO1 (gated through AW9523 `P1_7`).
- PSRAM heap via `esp-alloc`; internal SRAM reserved for ISR/real-time.
- `/dev/ttyACM1` is the CoreS3 USB-Serial-JTAG on Andy's machine (may shuffle with `/dev/ttyACM0` — check `udevadm info` first).
- Serial access requires `sg dialout -c '...'` wrapper on the host until `dialout` group is in the active shell's supplementary groups.
- Logs travel as `defmt`-encoded bytes over the ESP32-S3 USB-Serial-JTAG peripheral; `espflash --monitor --log-format defmt` decodes on the host. No RTT probe required.

## Config + assets

Boot-time network + auth config lives at `/sd/STACKCHAN.RON` — schema in [`stackchan-net::config`](crates/stackchan-net/src/config.rs), atomic writeback via `PUT /settings`. The firmware boots offline if the SD card is absent.

Face geometry is hardcoded in `stackchan-core::face`. Later releases may add RON-configurable appearance.
