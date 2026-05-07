<div align="center">

# stackchan-kai

**Clean-slate Rust firmware for the M5Stack CoreS3 Stack-chan — `no_std`, embassy, no cloud.**

[![CI](https://github.com/andymai/stackchan-kai/actions/workflows/ci.yml/badge.svg)](https://github.com/andymai/stackchan-kai/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/andymai/stackchan-kai)](https://github.com/andymai/stackchan-kai/releases)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org/)
[![unsafe denied](https://img.shields.io/badge/unsafe-workspace--denied-success.svg)]()

[Stability](./STABILITY.md) · [Changelog](./CHANGELOG.md) · [Justfile](./justfile) · [Handbook](https://andymai.github.io/stackchan-kai/)

</div>

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

`stackchan-core` models the avatar as data: an [`Entity`] (face, motor,
perception, voice, mind, events, input, tick) plus a [`Director`] that
sorts `Modifier`s by phase and ticks them each frame.

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

Each `Modifier` declares a phase (`Affect`, `Expression`, `Motion`,
`Audio`) and a priority; the Director sorts once and ticks per frame.
Stock modifiers cover blinking, breathing, idle eye drift, occasional
head glances, emotion transitions, touch / IR / voice / ambient /
battery reactions, attention-driven head tilt, and audio-driven mouth
motion. A parallel
`Skill` surface carries predicate-fired capabilities. See the
[architecture overview](https://andymai.github.io/stackchan-kai/architecture)
and [modifier authoring guide](https://andymai.github.io/stackchan-kai/modifiers)
for the details.

## Features

- **Animated face** — eased transitions across the m5stack-avatar emotion palette, blink / breath / idle-drift at double-buffered 30 FPS
- **Symbolic overlays** — speech-bubble text and decorator badges (heart, sweat, sleep, anger, shy) layered on top of the base face
- **Color palette swap** — runtime theme switch through a small set of named presets (default / dark / cute / dog) without disturbing the symbolic-overlay layer's distinctness
- **Head motion** — Feetech SCServo pan/tilt with a calibration bench (`just bench`) and a runtime zero-point correction surface for day-of mounting drift
- **9-axis sensing** — BMI270 accel + gyro, BMM150 magnetometer (compensated µT, live bench via `just mag-bench`)
- **Local inputs** — FT6336U touch, Si12T body-touch strip, LTR-553 ambient + proximity, NEC IR decoder
- **Timekeeping + peripherals** — BM8563 RTC, PY32 co-processor, WS2812 neck LED ring (`just leds-bench`)
- **Camera tracking** — GC0308 capture into a block-grid motion tracker, engagement-driven gaze with microsaccades and lost-target search
- **Optional autonomy** — opt-in soliloquy bubbles at random intervals; opt-in top-of-hour chime
- **Host-side sim** — runs the full modifier stack on the host with pixel-golden tests + an `egui` visualiser (`cargo run -p stackchan-sim --bin viz --features viz`); cuts behaviour iteration from ~30 s build cycles to under a second
- **Safe by default** — no `unwrap` in library code, typed errors throughout, `unsafe` denied workspace-wide

## Networking

`STACKCHAN.RON` on an SD card brings up Wi-Fi station, mDNS, and SNTP-on-link-up,
and the firmware exposes a LAN-only HTTP control plane:

- `GET /` — operator dashboard, single-page HTML embedded in the binary
- `GET /state/stream` — live state via Server-Sent Events
- `POST /emotion`, `/look-at`, `/face-target`, `/reset`, `/speak` — manual override
- `POST /volume`, `/mute`, `/mood`, `/palette`, `/head/offsets` — runtime control surface
- `POST /sleep`, `/wake` — sleep mode (eyes shut / head limp / LED dark / audio paused). Wake via the route, MCP tool, any touch, or the side power button.
- `POST /mcp` — JSON-RPC 2.0 endpoint speaking minimal MCP for AI agent integrations (`set_emotion`, `look_at`, `speak`, `set_volume`, `create_reminder` / `list_reminders` / `cancel_reminder`, …)
- `POST /firmware/update` — ed25519-signed SCFW image upload; auto-flashes the inactive OTA slot and soft-resets. Compiled out unless `STACKCHAN_OTA_PUBLIC_KEY` is set at build time. Always requires a configured bearer token.
- `GET` / `PUT /settings` — persistent config with atomic SD writeback
- Bearer-token auth on writes; constant-time compare; LAN-scoped (no TLS)

Discovery and inter-device:

- **mDNS** — `<hostname>.local` plus DNS-SD service records (`_stackchan._tcp.local.`)
  carrying a `kai=1` variant marker so kai-aware clients gate on extension endpoints
  while a generic Bonjour browser still lists the device alongside upstream stackchan units.
  TXT also publishes live `yaw=` / `pitch=` (one decimal, ≤100 ms refresh with a 1 s heartbeat)
  so a follower mimicking head pose tracks the leader without an HTTP round-trip
- **ESP-NOW** — peer-allowlisted RX driving the same `RemoteCommand` plumbing as HTTP,
  plus a TX path that broadcasts pose-mirror + heartbeat frames so multiple units can
  choreograph against each other
- **BLE peripheral** — Device Information / Battery / emotion / Wi-Fi provisioning /
  audio / avatar control / camera-view services, advertised as `stackchan-XXXXXX`
  (last three MAC bytes); shares the radio with Wi-Fi via `esp-radio` coex

Without an SD card the firmware boots offline and the desk-toy surface works
the same. See [HTTP control plane](https://andymai.github.io/stackchan-kai/http)
for the full reference.

## Non-goals

- No voice agent or LLM. This is not a xiaozhi replacement.
- No cloud APIs or telemetry.
- No C/C++ in the firmware binary. Drivers are written directly against datasheets.
- Not an M5Unified port. Only the desk-toy surface area is covered.

## License

Licensed under either of

- [Apache License, Version 2.0](./LICENSE-APACHE)
- [MIT License](./LICENSE-MIT)

at your option.

[`Entity`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/entity.rs
[`Director`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/director.rs
