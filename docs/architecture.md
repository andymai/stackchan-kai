---
title: Architecture overview
---

# Architecture overview

`stackchan-kai` models the desk toy as data. An [`Entity`] holds the
state (face, motor, perception, voice, mind, events, input, tick); a
[`Director`] sorts [`Modifier`]s by phase + priority and ticks them
against the entity each frame. The engine is `no_std`, allocation-free,
and shared verbatim between firmware and the host simulator.

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

Time to "boot complete — idle heartbeat" is ~1.4 s on the CoreS3.

## Task graph

Every task is either a producer (sensor → Signal) or a sink
(Signal → hardware). Each frame the render task drains sensor signals
into `entity.perception` / `entity.input`, calls
`director.run(&mut entity, now)`, then dispatches the post-frame sinks
(LCD blit, head pose, LED frame, chirp request).

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

Producers publish via `Signal::signal(value)`; the render task drains
via `try_take()` once per frame. Latest-wins: producers signal at any
rate, the render task picks up the most recent value, the next signal
overwrites anything unread.

## The engine

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

[`Modifier`] mutates the entity once per frame via `update(&mut Entity)`
and exposes static `ModifierMeta`. [`Director`] owns a fixed-capacity
heapless registry of modifier and skill references, sorts them once
on first `run()`, and ticks them each frame.

[`Skill`] is a longer-running NPC capability with a `name` and a
`description` consumable by a dispatcher. Trait surface only — no
shipped implementations yet.

`reads` / `writes` are documentation; debug-mode enforcement after each
`update` is planned.

## Phase ordering

Modifiers run in phase order, then by priority within a phase, then by
registration order:

| Phase        | Numeric | Role                                                  |
| ------------ | ------- | ----------------------------------------------------- |
| `Perception` | 10      | empty (render task drains Signals before `run()`)     |
| `Cognition`  | 20      | empty                                                 |
| `Affect`     | 30      | emotion deciders (Touch/Remote/Pickup/Voice/...)      |
| `Speech`     | 40      | empty                                                 |
| `Expression` | 50      | visual modifiers (`StyleFromEmotion`, Blink, Breath, …)   |
| `Motion`     | 60      | head modifiers (`IdleSway`, `HeadFromEmotion`, …)         |
| `Audio`      | 70      | audio-driven visual (`MouthFromAudio`)                |
| `Output`     | 80      | empty (render task draws after `run()`)               |

Numeric gaps of 10 leave room to insert a phase between existing
variants without renumbering. The current population list lives in
`crates/stackchan-core/src/modifiers/mod.rs` and the `Phase` enum
docstring — those track the source of truth.

`Skill`s run after the modifier pass each frame; the `Director`
polls each registered skill's `should_fire` predicate and invokes
matching ones in priority order. Skills write `mind.intent` /
`mind.attention` / `voice` / `events`; modifiers in later phases
translate that into face / motor.

## Entity components

```rust
pub struct Entity {
    pub face: Face,         // visual surface
    pub motor: Motor,       // head pose
    pub perception: Perception,
    pub voice: Voice,
    pub mind: Mind,
    pub events: Events,     // one-frame flags, cleared by Director
    pub input: Input,       // firmware → modifier pending inputs
    pub tick: Tick,         // { now, dt_ms, frame }, stamped each run()
}
```

Sub-component ownership: `perception` and `input` are firmware-write
/ modifier-read; `face`, `motor`, and `voice.chirp_request` are
modifier-write / firmware-read.

## Skill conventions

Skills don't write `entity.face` or `entity.motor` directly. They
express intent through `mind`, `voice`, and `events`; modifiers in
`Phase::Expression` and `Phase::Motion` translate that intent into
rendered face and physical motion. The rule is documented; a
`SkillView<'a>` borrow type that enforces it via the type system is
sketched but not implemented.

## Host simulator

`crates/stackchan-sim` constructs an `Entity` + `FakeClock` and runs
the same `Director` against hand-crafted time sequences. Pixel-golden
tests assert on `Eye::weight`, `Mouth::mouth_open`, etc. The `viz`
binary opens an `egui` + `winit` window and runs the modifier stack at
30 FPS so behavior changes iterate in sub-second cycles instead of the
~30 s build → flash → boot loop
(`cargo run -p stackchan-sim --bin viz --features viz`).

[`Entity`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/entity.rs
[`Director`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/director.rs
[`Modifier`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/modifier.rs
[`Skill`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/skill.rs
