---
crate: stackchan-core
role: Avatar domain library (no_std, no hardware deps)
bus: none
transport: "pure data + Modifier trait"
no_std: true
unsafe: forbidden
status: experimental (v0.x)
---

# stackchan-core

`no_std` domain library for the Stack-chan avatar. Models the face as
**data** and drives animation through a `Modifier` trait that mutates
an `Avatar` in response to the passage of time (supplied by a `Clock`).
No hardware, OS, or `alloc` dependency — this crate is the
platform-independent heart of the firmware and the simulator.

## Key Files

- `src/lib.rs` — crate root, public re-exports
- `src/avatar.rs` — `Avatar`, `Eye`, `EyePhase`, `Mouth`, `Point`, `SCALE_DEFAULT`
- `src/clock.rs` — `Clock` trait, `Instant` (millisecond-resolution monotonic)
- `src/draw.rs` — `Avatar::draw` renders into any `embedded_graphics::DrawTarget<Color = Rgb565>`
- `src/emotion.rs` — `Emotion` (Neutral, Happy, Sleepy, Surprised, Sad) and per-emotion style presets
- `src/head.rs` — `Pose`, `HeadDriver` trait, pan / tilt range constants
- `src/leds.rs` — `LedFrame`, `render_leds` — maps avatar state to the 12-pixel ring
- `src/modifiers/mod.rs` — `Modifier` trait + re-exports
- `src/modifiers/*.rs` — one file per modifier (blink, breath, emotion-*, idle-*, ambient-sleepy, intent-reflex, intent-style, body-gesture, wake-on-voice, low-battery, listen-head, mouth-open-audio, remote-command)
- `src/skills/*.rs` — one file per skill (look-at-sound, petting, handling) — long-running NPC capabilities that write `mind.intent` / `mind.attention`

## Architecture

```mermaid
flowchart TB
    subgraph Inputs
        Clock[Clock trait<br/><i>Instant</i>]
        Hardware[Hardware signals<br/><i>touch, IMU, IR, ambient</i>]
    end
    subgraph Core
        Avatar[(Avatar)]
        Modifiers[Modifier pipeline]
    end
    subgraph Outputs
        Draw[Avatar::draw<br/><i>embedded-graphics DrawTarget</i>]
        Head[HeadDriver trait<br/><i>Pose → pan/tilt</i>]
        Leds[render_leds<br/><i>LedFrame</i>]
    end

    Clock --> Modifiers
    Hardware --> Modifiers
    Modifiers -->|update(&mut Avatar, Instant)| Avatar
    Avatar --> Draw
    Avatar --> Head
    Avatar --> Leds
```

## Modifier Pipeline

Modifiers implement `fn update(&mut self, avatar: &mut Avatar, now: Instant)`.
Each one composes with the others — the firmware runs the full stack per
render tick. Listed roughly in application order:

| Modifier          | Effect                                                         |
|-------------------|----------------------------------------------------------------|
| `RemoteCommand`   | IR remote → emotion / pose override                            |
| `EmotionTouch`    | Touch-panel tap → emotion bump                                 |
| `AmbientSleepy`   | Dark room (low lux) → sleepy emotion                           |
| `IntentReflex`    | `mind.intent` transitions → emotion (PickedUp→Surprised, Shaken→Angry) |
| `WakeOnVoice`     | Sustained `audio_rms` above threshold → `Happy` + `Wake` chirp |
| `IntentFromLoud`   | Rising-edge `audio_rms` across loud threshold → `Surprised` + `Startled` intent + `Startle` chirp |
| `EmotionCycle`    | Default sequence: Neutral → Happy → Sleepy → Surprised → Sad   |
| `EmotionStyle`    | 300 ms ease on style fields (curves, scale, blush) per emotion |
| `EmotionHead`     | Per-emotion pose bias (neutral center, surprised up, etc.)     |
| `ListenHead`      | Upward tilt bias while `mind.attention == Listening`           |
| `HeadFromIntent`     | Brief asymmetric pan/tilt recoil on entry to `Startled`     |
| `IdleDrift`       | Slow randomized gaze drift                                     |
| `IdleSway`        | Subtle head-pan sway when idle                                 |
| `Blink`           | Lid close / open pulses                                        |
| `Breath`          | Per-cycle eye + mouth scale oscillation                        |

The full set lives in `src/modifiers/mod.rs` — the table above is
representative, not exhaustive.

`EmotionCycle → EmotionStyle → Blink → Breath → IdleDrift` is the
minimum stack; the firmware adds the others. The `Clock` argument makes
time a trait so the simulator can advance deterministically while
firmware advances from `embassy-time`.

## Key Types

- **`Avatar`** — `{ left_eye: Eye, right_eye: Eye, mouth: Mouth, emotion: Emotion }`. Plain data — no hidden state, no runtime invariants beyond "values stay in their documented ranges"
- **`Eye`** — `{ phase: EyePhase, weight: f32, offset: Point, ... }`. `weight` is the lid openness (0 = closed, 1 = open)
- **`Mouth`** — `{ rotation: f32, scale: f32, cheek_blush: f32, ... }`
- **`Clock`** — single method `fn now(&self) -> Instant`. Stable against re-entry, takes `&self`
- **`Instant`** — `u64` ms since some epoch. Operators for `+ delta_ms: u64` and sequences of differences
- **`Pose`** — `{ pan_deg: f32, tilt_deg: f32 }`. Bounded by `MAX_PAN_DEG` / `MAX_TILT_DEG`
- **`HeadDriver`** — `fn drive(&mut self, pose: Pose, now: Instant)`. Implemented by firmware (SCServo) and sim (recorder)

## Gotchas

1. **No `alloc`.** Modifiers own their state in fixed-size fields. Callers that want N of a modifier build a wrapper; the crate won't `Box` anything
2. **Time must be monotonic.** `Clock::now()` is trusted — a backward jump breaks modifiers that cache `last_update`. Wall-clock sources need a wrapper
3. **Draw is pure.** `Avatar::draw` produces pixels into the provided `DrawTarget`; it doesn't mutate the avatar. Run modifiers first, then draw
4. **Emotion transitions are 300 ms eased by `EmotionStyle`.** Don't snap emotion changes directly on the `Avatar` — go through the modifier so the ease kicks in
5. **No panic in library code.** Workspace lints deny `unwrap` / `expect` / `panic`. All APIs return values in documented ranges; pathological inputs saturate rather than panic

## Integration

- **Used by `stackchan-firmware`** for the render loop + head / LED output
- **Used by `stackchan-sim`** for host-side tests — the same `Avatar::draw` path runs against a `Vec<Rgb565>` framebuffer for pixel-golden snapshots
- **Unit-tested** with doctests; golden-test behaviour lives in `stackchan-sim`
- **Stability:** everything is `Experimental` in v0.x. The module structure and modifier set are stable; names and fields may still evolve before anything graduates to `Stable`
