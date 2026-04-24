# stackchan-kai

A clean-slate Rust firmware for the M5Stack CoreS3 StackChan character. No AI.
No upstream cloud. No C blobs. Just a desk toy that animates a face.

> **Status:** v0.1.0 shipped 2026-04-23. All public items are currently
> [Experimental](STABILITY.md#experimental); the v0.x series will iterate the
> avatar domain model before anything graduates to Stable.

## Why

The upstream M5Stack / xiaozhi firmware integrates a Chinese LLM-agent stack
with cloud dependencies, questionable security posture, and a C++ codebase
that's hard to reason about. This repo rebuilds just the local desk-toy piece
— animated face, motion, local interaction — in `no_std` Rust on top of
[`esp-hal`](https://github.com/esp-rs/esp-hal) and [embassy](https://embassy.dev/).

## Workspace layout

| Crate | What | Test target |
| --- | --- | --- |
| `crates/stackchan-core` | `no_std` domain library: `Avatar`, `Eye`, `Mouth`, `Modifier` trait, `Clock` trait, `Emotion`, `Pose`. Pure Rust, no hardware deps. | Host unit tests |
| `crates/stackchan-sim` | Headless integration tests that drive `stackchan-core` with a fake clock. Golden-test modifier sequences without hardware. | Host integration tests |
| `crates/axp2101` | Minimal AXP2101 PMU driver (I²C). Just enough to bring up the CoreS3 LCD rail + 3V3. embedded-hal-async. | Host mock-I²C tests |
| `crates/aw9523` | Minimal AW9523 I/O-expander init routine for the CoreS3 (LCD reset pulse, backlight-boost gate). embedded-hal-async. | Host mock-I²C tests |
| `crates/scservo` | Feetech SCServo half-duplex serial driver (UART1). embedded-io-async. | Host unit tests with a mock UART |
| `crates/stackchan-firmware` | Binary crate. `no_std` + `alloc`. embassy executor. Wires PMU init + mipidsi LCD driver + SCServo head driver + `stackchan-core` render loop on the CoreS3. | HIL via probe-rs + defmt-test |

## Build

```bash
# Host side (core + sim + drivers).
cargo test
cargo clippy --workspace --exclude stackchan-firmware --all-targets -- -D warnings

# Firmware side (requires espup-installed esp toolchain).
source ~/export-esp.sh
just build-firmware         # or: cd crates/stackchan-firmware && cargo +esp build --release
just fmr                    # flash + monitor in one go
```

See the `justfile` for the full set of host + firmware recipes.

## Conventions

- Rust 2024 edition, strict clippy (pedantic + nursery + no-panic), MIT/Apache
  dual license.
- Conventional commits via `.githooks/commit-msg`. After cloning:
  `git config core.hooksPath .githooks`
- Every change lands via PR; greptile bot review required.
- Parallel work: `git worktree add .worktrees/<branch> <branch>` — the
  `.worktrees/` directory is gitignored.

## License

Licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)

at your option.

## AI disclosure

See [AI-DISCLOSURE.md](AI-DISCLOSURE.md).
