---
title: Architecture overview
---

# Architecture overview

`stackchan-kai` models the desk toy as an NPC engine. An [`Entity`] is
a component-model bag of state (face, motor, perception, voice, mind,
events, input, tick); a [`Director`] owns the per-frame schedule, sorts
[`Modifier`]s by phase + priority, and ticks them against the entity.
The engine is `no_std`, allocation-free, and shared verbatim between
the firmware and the host simulator.

## Boot sequence

```
esp-hal init
    │
    ▼
internal SRAM + PSRAM heaps (esp-alloc)
    │
    ▼
esp-rtos embassy executor
    │
    ▼
AXP2101 LDOs (LCD rails, power-key timing, BATFET, ADC)
    │
    ▼
AW9523 I/O expander (LCD reset pulse, backlight-boost gate)
    │
    ▼
SPI2 + mipidsi ILI9342C (320×240 RGB565)
    │
    ▼
SCServo on UART1 (head pan/tilt)
    │
    ▼
PY32 co-processor (servo power, WS2812 ring)
    │
    ▼
Spawn embassy tasks → main heartbeat loop
```

Total time to "boot complete — idle heartbeat" is ~1.4 s on the CoreS3.

## Task graph

The render task is the orchestrator: every other task is either a
producer (sensor → Signal) or a sink (Signal → hardware). Each frame
the render task drains sensor signals into `entity.perception` /
`entity.input`, calls `director.run(&mut entity, now)`, then dispatches
post-frame sinks (LCD blit, head pose, LED frame, chirp request).

```
                  ┌─────────────┐
   touch ────────▶│             │
   IR    ────────▶│             │
   IMU   ────────▶│             │
   ambient ──────▶│  render     │──▶ LCD (mipidsi blit)
   power ────────▶│  (30 FPS)   │──▶ pose Signal ──▶ head_task ──▶ SCServo
   audio RMS ────▶│             │──▶ LED frame Signal ──▶ led_task ──▶ PY32
   camera ───────▶│             │──▶ chirp queue ──▶ audio TX
                  └─────────────┘
                         │
                         └──▶ heartbeat → watchdog (5 s poll)
```

Each producer publishes via `Signal::signal(value)`. The render task
drains via `try_take()` once per frame. **Latest-wins semantics**:
producers signal at any rate, the render task picks up the most recent
value per frame. Misses are normal and expected — the next signal
overwrites unread values.

## The engine

Three traits + one orchestrator:

- **[`Modifier`]** — per-frame state mutator. `update(&mut Entity)`. The
  14 stock animation behaviors all live here.
- **[`Skill`]** — discoverable capability with a `name` + `description`
  pair (modeled on Claude Code Skills) that doubles as trigger guidance
  for human readers and v2.x LLM-driven dispatch. Trait surface only
  today; zero implementations shipped.
- **[`Director`]** — owns a fixed-capacity heapless registry of
  modifier and skill references, sorts them once, ticks them each
  frame.

Modifiers declare static [`ModifierMeta`]:

```rust
pub struct ModifierMeta {
    pub name: &'static str,
    pub description: &'static str,
    pub phase: Phase,
    pub priority: i8,
    pub reads: &'static [Field],
    pub writes: &'static [Field],
}
```

`reads` / `writes` are documentation today; v2.x will enforce them via
debug-mode assertions after each `update`.

## Phase ordering

Modifiers run in phase order, then by priority within a phase, then by
registration order. The phase enum encodes the canonical NPC tick:

| Phase        | Numeric | Today                                                 |
| ------------ | ------- | ----------------------------------------------------- |
| `Perception` | 10      | empty — render task drains Signals before `run()`     |
| `Cognition`  | 20      | empty — v2.x: LAN-host LLM bridge adapter             |
| `Affect`     | 30      | 7 emotion deciders (Touch/Remote/Pickup/Voice/...)    |
| `Speech`     | 40      | empty — v2.x: TTS feeder + speech-queue producer      |
| `Expression` | 50      | 4 visual modifiers (`EmotionStyle`, Blink, Breath, …) |
| `Motion`     | 60      | 2 head modifiers (`IdleSway`, `EmotionHead`)          |
| `Audio`      | 70      | 1 audio-driven (`MouthOpenAudio`)                     |
| `Output`     | 80      | empty — render task draws + dispatches after `run()`  |

Numeric gaps of 10 leave room for v2.x phases (e.g.
`PostPerception = 15`, `IntentRefinement = 25`) without renumbering.

## Entity components

```rust
pub struct Entity {
    pub face: Face,         // visual surface
    pub motor: Motor,       // head pose
    pub perception: Perception, // raw sensors → world model
    pub voice: Voice,       // chirp_request, future speech queue
    pub mind: Mind,         // affect, autonomy, intent (v2.x), …
    pub events: Events,     // one-frame fire flags (cleared by Director)
    pub input: Input,       // pending firmware → modifier inputs
    pub tick: Tick,         // { now, dt_ms, frame } — stamped each run()
}
```

Sub-components carry domain boundaries: `entity.perception` is
firmware-write / modifier-read; `entity.input` is firmware-write /
modifier-consume; `entity.face` and `entity.motor` are
modifier-write / firmware-read; `entity.voice.chirp_request` is
modifier-write / firmware-drain.

## Skill conventions

By documented contract, skills MUST NOT write to `entity.face` or
`entity.motor` directly. Skills express intent through `mind`,
`voice`, and `events`; modifiers in `Phase::Expression` and
`Phase::Motion` translate that intent into rendered face and physical
motion. The rule is doc-enforced today; v2.x will introduce a
`SkillView<'a>` borrow type that mechanically excludes face/motor from
the writable surface.

## Host simulator

`crates/stackchan-sim` constructs an `Entity` + `FakeClock` and runs
the same `Director` against hand-crafted time sequences. Pixel-golden
tests assert on `Eye::weight`, `Mouth::mouth_open`, etc. The `viz`
binary opens an `egui` + `winit` window and runs the canonical
modifier stack at wall-clock 30 FPS — drops behavior-iteration cycles
from ~30 s (build → flash → boot) to <1 s
(`cargo run -p stackchan-sim --bin viz --features viz`).

[`Entity`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/entity.rs
[`Director`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/director.rs
[`Modifier`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/modifier.rs
[`Skill`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/skill.rs
[`ModifierMeta`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/director.rs
