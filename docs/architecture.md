---
title: Architecture overview
---

# Architecture overview

The firmware is a pure data-flow graph: sensors publish to `Signal`
channels, the render task drains them into `Avatar` fields, a fixed
modifier stack mutates the `Avatar` once per 30 FPS frame, and output
sinks (LCD, head servos, LEDs, audio TX) consume the result.

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
producer (sensor → Signal) or a sink (Signal → hardware).

```
                  ┌─────────────┐
   touch ────────▶│             │
   IR    ────────▶│             │
   IMU   ────────▶│             │
   ambient ──────▶│  render     │──▶ LCD (mipidsi blit)
   power ────────▶│  (30 FPS)   │──▶ pose Signal ──▶ head_task ──▶ SCServo
   audio RMS ────▶│             │──▶ LED frame Signal ──▶ led_task ──▶ PY32
   camera ───────▶│             │
                  └─────────────┘
                         │
                         └──▶ heartbeat → watchdog (5s poll)
```

Each producer publishes via `Signal::signal(value)`. The render task
drains via `try_take()` once per frame. **Latest-wins semantics**:
producers signal at any rate, the render task picks up the most recent
value per frame. Misses are normal and expected — the next signal
overwrites unread values.

## Modifier stack

The render task runs this canonical sequence per frame:

```
EmotionTouch → RemoteCommand → PickupReaction → WakeOnVoice
    → AmbientSleepy → LowBatteryEmotion
    → EmotionCycle → EmotionStyle → EmotionHead
    → Blink → Breath
    → IdleDrift → IdleSway
    → MouthOpenAudio
```

Ordering matters: `EmotionTouch` runs first so a tap queued from the
touch task becomes the active emotion before `EmotionCycle` checks
the `manual_until` gate. `IdleSway` writes the base `head_pose`;
`EmotionHead` adds an emotion-keyed bias on top (layered compose).

See the [Modifier authoring guide](modifiers) for how to extend.

## Host simulator

`crates/stackchan-sim` mirrors the firmware's modifier execution
against a `FakeClock` so the entire avatar behavior is host-testable
without flashing. The `viz` binary opens a window via `egui` + `winit`
and runs the canonical modifier stack at wall-clock 30 FPS — drops
behavior-iteration cycles from ~30 s (build → flash → boot) to <1 s
(`cargo run --features viz`).
