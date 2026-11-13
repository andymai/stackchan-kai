<div align="center">

# stackchan-kai

[![CI](https://github.com/andymai/stackchan-kai/actions/workflows/ci.yml/badge.svg)](https://github.com/andymai/stackchan-kai/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/andymai/stackchan-kai)](https://github.com/andymai/stackchan-kai/releases)
[![Last release](https://img.shields.io/github/release-date/andymai/stackchan-kai?label=last%20release)](https://github.com/andymai/stackchan-kai/releases)
[![Commit activity](https://img.shields.io/github/commit-activity/m/andymai/stackchan-kai?label=commits%2Fmonth)](https://github.com/andymai/stackchan-kai/commits/main)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org/)

`no_std` Rust firmware on embassy for the Stack-chan desktop robot (M5Stack CoreS3). It is LAN-only, it needs no cloud service, and you can test it on the host.

[Stability](./STABILITY.md) · [Changelog](./CHANGELOG.md) · [Justfile](./justfile) · [Handbook](https://andymai.github.io/stackchan-kai/)

</div>

## Flash it

```bash
cargo install espup && espup install
source ~/export-esp.sh
just fmr    # flash + monitor over USB-Serial-JTAG
```

You need a [CoreS3 Stack-chan kit](https://shop.m5stack.com/products/stackchan-kawaii-co-created-open-source-ai-desktop-robot), a USB-C cable, and Rust 1.88+. For serial access, your user must be in the `dialout` group. See the [justfile](./justfile) for the full recipe set (host tests, MSRV build, sensor bench examples).

## Why

M5Stack ships Stack-chan with the `xiaozhi` firmware stack, an LLM agent pipeline in C++ that depends on a cloud service. `stackchan-kai` rebuilds the desk-toy features in `no_std` Rust on [`esp-hal`](https://github.com/esp-rs/esp-hal) and [embassy](https://embassy.dev/). These features are the animated face, head motion, local sensors, and an optional voice agent through a sidecar. The engine models the avatar as data, and a host-side simulator uses the same render path. Network egress goes only to the voice services that the operator sets and to the SNTP servers (`pool.ntp.org` by default).

## The engine

`stackchan-core` models the avatar as data. The data is an [`Entity`] (face, motor, perception, voice, mind, events, input, tick). A [`Director`] sorts `Modifier`s by phase and ticks them each frame.

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

Each `Modifier` declares a phase (`Perception`, `Cognition`, `Affect`, `Speech`, `Expression`, `Decoration`, `Motion`, `Audio`) and a priority. The Director sorts the modifiers one time and ticks them each frame. A parallel `Skill` trait carries capabilities that run longer and start when a predicate is true. Skills write intent into `mind` and `voice`, and modifiers turn that intent into face and motion. The catalogues are in [`crates/stackchan-core/src/modifiers/`](./crates/stackchan-core/src/modifiers) and [`crates/stackchan-core/src/skills/`](./crates/stackchan-core/src/skills).

Time comes in through a `Clock` trait, so the same Director runs against a `FakeClock` on the host. `stackchan-sim` drives the modifier stack through scripted time sequences with pixel-golden assertions. It also has an `egui` visualiser (`cargo run -p stackchan-sim --bin viz --features viz`). On the host, a behaviour change takes less than a second to try, and a flash cycle takes about 30 s. For the details, see the [architecture overview](https://andymai.github.io/stackchan-kai/architecture) and the [modifier authoring guide](https://andymai.github.io/stackchan-kai/modifiers).

## Voice agent

The voice agent is opt-in. A capture starts when on-device [microWakeWord](https://github.com/kahrendt/microWakeWord) inference hears the wake word (TFLite Micro + ESP-NN, with the model on the SD card). The operator can also start a capture with `POST /listen`. The firmware uploads the captured PCM (`audio/L16` at 16 kHz mono) to a sidecar URL that you choose. It then shows the JSON reply (`text`, `emotion`) on the toast band of the avatar. STT and the LLM run in the sidecar, and kai never embeds them.

[`sidecar/`](./sidecar) has a reference Python sidecar (faster-whisper + Anthropic Claude) that you can deploy with Docker or systemd. [docs/voice.md](./docs/voice.md) has the setup, and [sidecar/README.md](./sidecar/README.md) has the wire contract. If you do not set a sidecar URL, the listen path is off, and everything else runs the same.

## Networking

With `STACKCHAN.RON` on an SD card, the firmware starts Wi-Fi station mode and mDNS, and it starts SNTP when the link comes up. It then serves a LAN-only HTTP control plane. Each write must carry a bearer token, and the firmware compares the token in constant time. Without an SD card, the firmware boots offline, and the desk-toy features work the same.

- `GET /`: embedded operator dashboard
- `GET /state` / `GET /state/stream`: snapshot or live SSE
- `GET` / `PUT /settings`: persistent config with atomic SD writeback
- `POST /emotion`, `/look-at`, `/look-at-point`, `/face-target`, `/reset`, `/speak`, `/volume`, `/mute`, `/mood`, `/palette`, `/head/offsets`, `/face-geometry`: runtime override and control
- `POST /sleep` / `/wake`: collapse the avatar (eyes shut, head limp, LED dark, audio paused). To wake it, use the route, an MCP tool, any touch, or the side power button.
- `POST /listen`: voice capture that the operator starts (the same path as the wake word)
- `POST /camera/mode` + `/camera/capture`, `GET /camera/snapshot`: switch the LCD between the camera preview and the avatar, trigger a frame, and fetch the last 320×240 RGB565 raster from SD
- `POST /dance`: JSON keyframe stream for the `DancePlayer` modifier ([docs/dance.md](./docs/dance.md))
- `POST /mcp`: JSON-RPC 2.0 MCP endpoint for AI agent integrations (`set_emotion`, `look_at`, `speak`, `create_reminder`, …)
- `POST /firmware/update`: an ed25519-signed SCFW image. The firmware flashes it to the inactive OTA slot and does a soft reset. The build leaves this route out unless you set `STACKCHAN_OTA_PUBLIC_KEY` at build time. `just ota-image` builds and signs an image, and `just ota-push` uploads it.

The full reference is in [docs/http.md](./docs/http.md).

**Discovery + inter-device**

- **mDNS** + DNS-SD (`_stackchan._tcp.local.`) with a `kai=1` variant marker. The TXT record publishes live `yaw=` / `pitch=` values, so a follower can mirror the pose without an HTTP round trip.
- **ESP-NOW**: RX from allowlisted peers goes through the same `RemoteCommand` path as HTTP. TX sends pose-mirror and heartbeat messages for choreography with more than one unit.
- **BLE peripheral**: Device Information, Battery, emotion, audio, avatar control, and view services. You can set Wi-Fi credentials through a custom provisioning service or through BluFi (an Espressif standard). BLE shares the radio with Wi-Fi through `esp-radio` coex.
- **Claude Desktop companion**: the Nordic UART Service exposes desktop-side render, permission, control, and time tasks for an operator surface on an attached laptop.

## Features

**Avatar**

- Eased transitions across the m5stack-avatar emotion palette, and blink, breath, and idle drift at double-buffered 30 FPS
- Symbolic overlays: speech-bubble text and decorator badges (heart, sweat, dizzy, ear, pairing, angry, shy) on top of the base face
- Battery indicator: an opt-in corner overlay. It shows the level in segments, so jitter of one percent does not reach the renderer's dirty-check.
- Color palette swap: runtime theme presets (default / dark / cute / dog) that do not change the symbolic overlay layer
- Face geometry presets: select one with `POST /face-geometry` or MCP. The firmware saves the active selection to `/sd/RUNTIME.RON` with the palette and the mood, and loads it again at boot.
- Idle autonomy: opt-in soliloquy bubbles at random intervals, and an opt-in chime at the top of each hour when the RTC knows the year

**Motion**

- Feetech SCServo pan/tilt with a calibration bench (`just bench`), and a runtime zero-point correction for mounting drift on the day of use
- 3D `lookAtPoint` IK through `POST /look-at-point`, and attention-driven head tilt and microsaccades from the camera tracker
- Dance keyframe playback through the `DancePlayer` modifier
- Sleep mode collapses head pose, eyes, LED, and audio together

**Sensors + inputs**

- BMI270 accel + gyro (live tilt stream, shake detection)
- BMM150 magnetometer: compensated µT, live bench through `just mag-bench` (bench-only on this unit, see *Known limitations*)
- FT6336U capacitive touch, Si12T body-touch strip (back-of-head pads)
- LTR-553 ambient light + proximity, NEC IR decoder
- GC0308 camera capture into a block-grid motion tracker. The tracker drives engagement gaze with microsaccades and a search for a lost target.

**Peripherals**

- BM8563 RTC, PY32 co-processor, WS2812 neck LED ring (`just leds-bench`)
- AXP2101 PMU with side power-key timing and battery gauge

**Errors, `unsafe`, and OTA**

- No `unwrap` / `expect` in library code, and typed errors in the whole workspace ([docs/errors.md](./docs/errors.md))
- The workspace denies `unsafe`. The firmware crate allows it only behind per-module annotations, for linker symbols and register-map pointers.
- Signed OTA path (ed25519, compiled out by default)

## Scope

This project deliberately does not:

- **Embed STT, LLM, or TTS.** Speech intelligence runs in a sidecar that the operator supplies. The firmware embeds wake-word inference (microWakeWord, TFLite Micro), but it sends captured PCM upstream for transcription and generation. This keeps the binary `no_std` and inside the embedded flash budget.
- **Support hardware other than the CoreS3 Stack-chan kit.** The driver set follows specific datasheets (BMI270, BMM150, FT6336U, Feetech SCServo, …). The project tests it on one physical unit. A port to other M5Stack boards or ESP32 variants is out of scope.
- **Provide a stable public API.** All crates are Experimental per [STABILITY.md](./STABILITY.md), and minor releases break things. You can use the `stackchan-core` library, but its contract can change.
- **Replace general-purpose ESP-IDF or M5Unified firmware.** The project covers only the desk-toy features (face, motion, sensors, LAN control). Other features (arbitrary GPIO scripting, third-party display drivers) belong in a different project.
- **Accept unsolicited contributions.** The project has a single maintainer, and responses are best-effort. Bug reports and discussion are welcome, and the PR policy is in `AGENTS.md`.

## Known limitations

- The project tests on a single CoreS3 unit. The BMM150 magnetometer on this kit is bench-only, because interference from the chassis makes the reading in the enclosure unusable. The project uses the other sensors regularly.
- The HTTP plane is LAN-only and has no TLS. The bearer-token gate is not a hardened auth surface for an untrusted network.
- All public APIs are Experimental per [STABILITY.md](./STABILITY.md). Minor releases will break things.
- This is a single-maintainer project. Responses to issues and PRs are best-effort, with no fixed cadence.

## License

Licensed under either of

- [Apache License, Version 2.0](./LICENSE-APACHE)
- [MIT License](./LICENSE-MIT)

at your option.

[`Entity`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/entity.rs
[`Director`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/director.rs
