# stackchan-rs

A clean-slate Rust firmware for the M5Stack CoreS3 StackChan character. No AI.
No upstream cloud. No C blobs. Just a desk toy that animates a face.

> **Status:** pre-v0.1.0 — the repo is being scaffolded. No released crate
> artefacts. See [`STABILITY.md`](STABILITY.md) for the stability policy once
> we tag v0.1.0.

## Why

The upstream M5Stack / xiaozhi firmware integrates a Chinese LLM-agent stack
with cloud dependencies, questionable security posture, and a C++ codebase
that's hard to reason about. This repo rebuilds just the local desk-toy piece
— animated face, motion, local interaction — in `no_std` Rust on top of
[`esp-hal`](https://github.com/esp-rs/esp-hal) and [embassy](https://embassy.dev/).

## Workspace layout

| Crate | What | Test target |
| --- | --- | --- |
| `crates/stackchan-core` | `no_std` domain library: `Avatar`, `Eye`, `Mouth`, `Modifier` trait, `Clock` trait, `Emotion`. Pure Rust, no hardware deps. | Host unit tests |
| `crates/stackchan-sim` | Headless integration tests that drive `stackchan-core` with a fake clock. Golden-test modifier sequences without hardware. | Host integration tests |
| `crates/axp2101` | Minimal AXP2101 PMU driver (I²C). Just enough to bring up the CoreS3 LCD rail + 3V3. embedded-hal-async. | Host mock-I²C tests |
| `crates/stackchan-firmware` | Binary crate. `no_std` + `alloc`. embassy executor. Wires PMU init + mipidsi LCD driver + `stackchan-core` render loop on the CoreS3. | HIL via probe-rs + defmt-test |

## Build

```bash
# Host side (core + sim + axp2101)
cargo test
cargo clippy --workspace

# Firmware side (requires espup-installed esp toolchain)
cargo build -p stackchan-firmware --release
espflash flash --monitor target/xtensa-esp32s3-none-elf/release/stackchan-firmware
```

## Conventions

- Rust 2024 edition, strict clippy (pedantic + nursery + no-panic), MIT/Apache
  dual license.
- Conventional commits via `.githooks/commit-msg`. After cloning:
  `git config core.hooksPath .githooks`
- Every change lands via PR once v0.1.0 ships; greptile bot review required.
- Parallel work: `git worktree add .worktrees/<branch> <branch>` — the
  `.worktrees/` directory is gitignored.

## License

Licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)

at your option.

## AI disclosure

See [AI-DISCLOSURE.md](AI-DISCLOSURE.md).
