<div align="center">

# stackchan-kai

**Clean-slate Rust firmware for the M5Stack CoreS3 Stack-chan — `no_std`, embassy, no cloud.**

A small NPC engine in `no_std` Rust that animates a desk-toy face. No vendor cloud. No telemetry. No C blobs.

[![CI](https://github.com/andymai/stackchan-kai/actions/workflows/ci.yml/badge.svg)](https://github.com/andymai/stackchan-kai/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/andymai/stackchan-kai)](https://github.com/andymai/stackchan-kai/releases)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org/)
[![unsafe denied](https://img.shields.io/badge/unsafe-workspace--denied-success.svg)]()

[Stability](./STABILITY.md) · [Changelog](./CHANGELOG.md) · [Justfile](./justfile) · [Handbook](https://andymai.github.io/stackchan-kai/)

</div>

> **Status:** v0.1.0 shipped 2026-04-23. Public items are
> [Experimental](STABILITY.md#experimental); the v0.x series will iterate
> the engine domain model before anything graduates to Stable.

## Flash it

```bash
cargo install espup && espup install
source ~/export-esp.sh
just fmr    # flash + monitor over USB-Serial-JTAG
```

Needs a [CoreS3 Stack-chan kit](https://shop.m5stack.com/products/stackchan-kawaii-co-created-open-source-ai-desktop-robot),
a USB-C cable, Rust 1.88+, and `dialout` group membership for serial access.
See the [justfile](./justfile) for the full recipe set (host tests, CI gates,
sensor bench examples).

## Why

M5Stack ships Stack-chan with an xiaozhi firmware stack: a Chinese
LLM-agent pipeline with cloud dependencies, questionable security posture, and
a C++ codebase that's hard to audit. stackchan-kai rebuilds just the local
desk-toy surface — animated face, head motion, local sensors — in `no_std`
Rust on top of [`esp-hal`](https://github.com/esp-rs/esp-hal) and
[embassy](https://embassy.dev/). The engine is modeled as data and the render
path is shared with a host-side simulator, so most of the firmware is testable
without touching the hardware.

## The engine

`stackchan-core` is a small NPC engine: an [`Entity`] component bag
(face, motor, perception, voice, mind, events, input, tick) animated by
a [`Director`] that sorts behaviors into phases and ticks them each
frame.

```rust
use stackchan_core::{Director, Entity, Instant, modifiers::Blink};

let mut entity = Entity::default();
let mut blink = Blink::new();
let mut director = Director::new();
director.add_modifier(&mut blink).expect("registry has room");

for ms in (0..10_000).step_by(33) {
    director.run(&mut entity, Instant::from_millis(ms));
}
```

Two trait families:

- **`Modifier`** — per-frame state mutator. The 14 stock animation
  behaviors (Blink, Breath, IdleDrift, IdleSway, EmotionStyle,
  EmotionTouch, EmotionCycle, RemoteCommand, PickupReaction,
  WakeOnVoice, AmbientSleepy, LowBatteryEmotion, EmotionHead,
  MouthOpenAudio) all live here.
- **`Skill`** — discoverable capability with a `name` + `description`
  pair (modeled on Claude Code Skills) that doubles as trigger
  guidance. Trait surface only today; zero implementations shipped.

Modifiers register with a phase (`Affect` / `Expression` / `Motion` /
`Audio` today; `Perception` / `Cognition` / `Speech` / `Output`
reserved with numeric gaps for future insertion). The Director sorts
once by `(phase, priority, registration_order)` and ticks each
modifier per frame. See the [architecture overview](https://andymai.github.io/stackchan-kai/architecture)
and the [modifier authoring guide](https://andymai.github.io/stackchan-kai/modifiers)
for the full picture.

## Features

- **Animated face** — five emotions, 300 ms eased transitions, blink / breath / idle-drift modifiers at double-buffered 30 FPS
- **Head motion** — Feetech SCServo pan/tilt driver with a calibration bench (`just bench`)
- **9-axis sensing** — BMI270 accel + gyro, BMM150 magnetometer (compensated µT, live bench via `just mag-bench`)
- **Local inputs** — FT6336U touch, LTR-553 ambient + proximity, NEC IR decoder
- **Timekeeping + peripherals** — BM8563 RTC, PY32 co-processor, WS2812 neck LED ring (`just leds-bench`)
- **Reactive emotion engine** — touch / pickup / voice / ambient / battery all drive the face through the same Director pipeline
- **Host-side sim** — full Director runs on the host with pixel-golden tests + an `egui` visualiser (`cargo run -p stackchan-sim --bin viz --features viz`); behaviour iteration drops from ~30 s build cycles to <1 s
- **Safe by default** — no `unwrap` in library code, typed errors throughout, `unsafe` denied workspace-wide

## Non-goals

- **No voice agent or LLM.** This is not a xiaozhi replacement.
- **No cloud or telemetry.** Zero outbound network calls today.
- **No C/C++ in the firmware binary.** Drivers are written directly against datasheets.
- **No Wi-Fi / BLE yet.** The networking stack is out of scope for v0.x.
- **Not an M5Unified port.** Only the desk-toy surface area is covered.

## Roadmap

- **RON-configurable appearance** — eye / mouth geometry, palette, per-emotion style
- **Calibration tooling** — host-side bench writes servo + sensor calibration into versioned config
- **Crash recovery** — panic handler renders an error face and watchdog-reboots cleanly

## License

Licensed under either of

- [Apache License, Version 2.0](./LICENSE-APACHE)
- [MIT License](./LICENSE-MIT)

at your option.

[`Entity`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/entity.rs
[`Director`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/director.rs
