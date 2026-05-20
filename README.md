<div align="center">

# stackchan-kai

[![CI](https://github.com/andymai/stackchan-kai/actions/workflows/ci.yml/badge.svg)](https://github.com/andymai/stackchan-kai/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/andymai/stackchan-kai)](https://github.com/andymai/stackchan-kai/releases)
[![Last release](https://img.shields.io/github/release-date/andymai/stackchan-kai?label=last%20release)](https://github.com/andymai/stackchan-kai/releases)
[![Commit activity](https://img.shields.io/github/commit-activity/m/andymai/stackchan-kai?label=commits%2Fmonth)](https://github.com/andymai/stackchan-kai/commits/main)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org/)

`no_std` Rust firmware for the M5Stack CoreS3 Stack-chan — embassy-based, LAN-only, host-testable.

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
See the [justfile](./justfile) for the full recipe set (host tests, MSRV
build, sensor bench examples).

## Why

M5Stack ships Stack-chan with the `xiaozhi` firmware stack: a cloud-dependent
LLM-agent pipeline written in C++. `stackchan-kai` rebuilds the desk-toy
surface — animated face, head motion, local sensors, optional sidecar-routed
voice agent — in `no_std` Rust on top of
[`esp-hal`](https://github.com/esp-rs/esp-hal) and
[embassy](https://embassy.dev/). The engine is modeled as data, the render
path is shared with a host-side simulator, and the only network egress is
whatever the operator points the voice path at.

## The engine

`stackchan-core` models the avatar as data: an [`Entity`] (face, motor,
perception, voice, mind, events, input, tick) plus a [`Director`] that
sorts `Modifier`s by phase and ticks them each frame.

```rust
use stackchan_core::{Director, Entity, Instant};
use stackchan_core::modifiers::{Blink, EmotionCycle, IdleHeadDrift};

let mut entity = Entity::default();
let mut emotion = EmotionCycle::new();   // Phase::Affect
let mut blink = Blink::new();            // Phase::Expression
let mut drift = IdleHeadDrift::new();    // Phase::Motion

let mut director = Director::new();
director.add_modifier(&mut emotion).expect("registry has room");
director.add_modifier(&mut blink).expect("registry has room");
director.add_modifier(&mut drift).expect("registry has room");

for ms in (0..10_000).step_by(33) {
    director.run(&mut entity, Instant::from_millis(ms));
}
```

Each `Modifier` declares a phase (`Perception`, `Cognition`, `Affect`,
`Speech`, `Expression`, `Decoration`, `Motion`, `Audio`) and a priority;
the Director sorts once and ticks per frame. A parallel `Skill` surface
carries longer-running, predicate-fired capabilities — skills write
intent into `mind` and `voice`, modifiers translate that to face and
motion. Catalogues live in
[`crates/stackchan-core/src/modifiers/`](./crates/stackchan-core/src/modifiers)
and [`crates/stackchan-core/src/skills/`](./crates/stackchan-core/src/skills).

Because time flows in through a `Clock` trait, the same Director runs
against a `FakeClock` on the host. `stackchan-sim` drives the modifier
stack through scripted time sequences with pixel-golden assertions
and an `egui` visualiser (`cargo run -p stackchan-sim --bin viz
--features viz`) — behaviour iteration takes under a second instead of
a ~30 s flash cycle. See the
[architecture overview](https://andymai.github.io/stackchan-kai/architecture)
and [modifier authoring guide](https://andymai.github.io/stackchan-kai/modifiers)
for the details.

## Voice agent

Opt-in. Wake word fires from on-device
[microWakeWord](https://github.com/kahrendt/microWakeWord) inference
(TFLite Micro + ESP-NN, model on SD card) or from an operator-initiated
`POST /listen`. The firmware uploads captured PCM (`audio/L16` at
16 kHz mono) to a sidecar URL of your choice and renders the JSON
reply (`text`, `emotion`) on the avatar's toast band. STT and LLM live
in the sidecar — kai never embeds them.

A reference Python sidecar (faster-whisper + Anthropic Claude, Docker
/ systemd deployable) ships in [`sidecar/`](./sidecar). Setup in
[docs/voice.md](./docs/voice.md); wire contract in
[sidecar/README.md](./sidecar/README.md). Without a sidecar URL
configured the listen path is inactive; everything else runs the same.

## Networking

`STACKCHAN.RON` on an SD card brings up Wi-Fi station, mDNS, and
SNTP-on-link-up; the firmware then exposes a LAN-only HTTP control
plane. Writes carry a bearer token (constant-time compare). Without an
SD card the firmware boots offline and the desk-toy surface works the
same.

- `GET /` — embedded operator dashboard
- `GET /state` / `GET /state/stream` — snapshot or live SSE
- `GET` / `PUT /settings` — persistent config with atomic SD writeback
- `POST /emotion`, `/look-at`, `/look-at-point`, `/face-target`,
  `/reset`, `/speak`, `/volume`, `/mute`, `/mood`, `/palette`,
  `/head/offsets`, `/face-geometry` — runtime override + control
- `POST /sleep` / `/wake` — collapse the avatar (eyes shut, head limp,
  LED dark, audio paused); wake via route, MCP tool, any touch, or the
  side power button
- `POST /listen` — operator-initiated voice capture (mirrors the
  wake-word path)
- `POST /camera/mode` + `/camera/capture`, `GET /camera/snapshot` —
  toggle tracker / capture pipeline, trigger a frame, fetch the last
  320×240 RGB565 raster from SD
- `POST /dance` — JSON keyframe stream for the `DancePlayer` modifier
  ([docs/dance.md](./docs/dance.md))
- `POST /mcp` — JSON-RPC 2.0 MCP endpoint for AI-agent integrations
  (`set_emotion`, `look_at`, `speak`, `create_reminder`, …)
- `POST /firmware/update` — ed25519-signed SCFW image; flashes the
  inactive OTA slot and soft-resets. Compiled out unless
  `STACKCHAN_OTA_PUBLIC_KEY` is set at build time.

Full reference: [docs/http.md](./docs/http.md).

**Discovery + inter-device**

- **mDNS** + DNS-SD (`_stackchan._tcp.local.`) with a `kai=1` variant
  marker; TXT publishes live `yaw=` / `pitch=` so a follower can mirror
  pose without an HTTP round-trip
- **ESP-NOW** — peer-allowlisted RX driving the same `RemoteCommand`
  plumbing as HTTP, plus pose-mirror + heartbeat TX for multi-unit
  choreography
- **BLE peripheral** — Device Information, Battery, emotion, audio,
  avatar control, and view services; Wi-Fi credentials can be set via
  a custom provisioning service or via BluFi (Espressif standard);
  shares the radio with Wi-Fi via `esp-radio` coex
- **Claude Desktop companion** — Nordic UART Service exposes
  desktop-side render / permission / control / time tasks for a
  laptop-attached operator surface

## Features

**Avatar**

- Eased transitions across the m5stack-avatar emotion palette, blink /
  breath / idle-drift at double-buffered 30 FPS
- Symbolic overlays — speech-bubble text plus decorator badges (heart,
  sweat, dizzy, ear, pairing, angry, shy) layered on the base face
- Battery indicator — opt-in corner overlay, segment-bucketed to keep
  per-percent jitter out of the renderer's dirty-check
- Color palette swap — runtime theme presets (default / dark / cute /
  dog) that don't bleed into the symbolic-overlay layer
- Face geometry presets — selectable via `POST /face-geometry` and MCP;
  active selection persists to `/sd/RUNTIME.RON` alongside palette + mood
  and is restored on boot
- Idle autonomy — opt-in soliloquy bubbles at random intervals; opt-in
  top-of-hour chime when an RTC year is known

**Motion**

- Feetech SCServo pan/tilt with a calibration bench (`just bench`) and
  a runtime zero-point correction surface for day-of mounting drift
- 3D `lookAtPoint` IK via `POST /look-at-point`, plus attention-driven
  head tilt and microsaccades from the camera tracker
- Dance keyframe playback through the `DancePlayer` modifier
- Sleep mode collapses head pose, eyes, LED, and audio together

**Sensors + inputs**

- BMI270 accel + gyro (live tilt streaming, shake detection)
- BMM150 magnetometer — compensated µT, live bench via `just mag-bench`
  (bench-only on this unit; see *Known limitations*)
- FT6336U capacitive touch, Si12T body-touch strip (back-of-head pads)
- LTR-553 ambient light + proximity, NEC IR decoder
- GC0308 camera capture into a block-grid motion tracker driving
  engagement gaze with microsaccades and lost-target search

**Peripherals**

- BM8563 RTC, PY32 co-processor, WS2812 neck LED ring (`just leds-bench`)
- AXP2101 PMU with side power-key timing and battery gauge

**Robustness**

- No `unwrap` / `expect` in library code, typed errors throughout
  ([docs/errors.md](./docs/errors.md))
- `unsafe` denied workspace-wide; firmware crate allows it only behind
  per-module annotations for linker symbols and register-map pointers
- Signed OTA path (ed25519, compiled out by default)

## Scope

This project deliberately does not:

- **Embed STT, LLM, or TTS** — speech intelligence lives in an
  operator-supplied sidecar. The firmware embeds wake-word inference
  (microWakeWord, TFLite Micro) but ships captured PCM upstream for
  transcription and generation, keeping the binary `no_std` and
  inside the embedded flash budget.
- **Support hardware beyond the CoreS3 Stack-chan kit** — the driver
  set is written against specific datasheets (BMI270, BMM150, FT6336U,
  Feetech SCServo, …) and tested on one physical unit. Porting to
  other M5Stack boards or ESP32 variants is out of scope.
- **Provide a stable public API yet** — all crates are Experimental
  per [STABILITY.md](./STABILITY.md); minor releases break things. The
  `stackchan-core` library is usable but its contract is still settling.
- **Replace general-purpose ESP-IDF or M5Unified firmware** — only the
  desk-toy surface area (face, motion, sensors, LAN control) is
  covered. Features outside that surface (arbitrary GPIO scripting,
  third-party display drivers) belong in a different project.
- **Accept unsolicited contributions** — single-maintainer, best-effort
  response. Bug reports and discussion are welcome; the PR policy is
  in `AGENTS.md`.

## Known limitations

- Tested on a single CoreS3 unit. The BMM150 magnetometer on this kit
  is bench-only — chassis-side interference makes the in-enclosure
  reading unusable; other sensors are exercised regularly.
- LAN-only HTTP plane, no TLS. The bearer-token gate is not a hardened
  auth surface for an untrusted network.
- All public APIs are Experimental per
  [STABILITY.md](./STABILITY.md). Minor releases will break things.
- Single-maintainer project. Issue and PR response is best-effort;
  nothing is on a cadence.

## License

Licensed under either of

- [Apache License, Version 2.0](./LICENSE-APACHE)
- [MIT License](./LICENSE-MIT)

at your option.

[`Entity`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/entity.rs
[`Director`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/director.rs
