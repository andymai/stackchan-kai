---
title: Signal channels
---

# Signal channels

Cross-task communication runs through typed `Signal<RawMutex, T>`
channels — `embassy_sync::signal::Signal`. Each channel has exactly
one producer and one consumer, latest-wins semantics, and never
blocks.

## The pattern

```rust
// Producer side (per-peripheral task):
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

pub static SOMETHING_SIGNAL: Signal<CriticalSectionRawMutex, ValueType>
    = Signal::new();

// Inside the producer's loop:
SOMETHING_SIGNAL.signal(my_value);

// Consumer side (render task):
if let Some(value) = SOMETHING_SIGNAL.try_take() {
    avatar.something = Some(value);
}
```

## Why `try_take` and not `wait`

The render task drains every signal once per 30 FPS frame. Using
`signal.wait()` would block until a value arrives — the render task
can't afford to block. `try_take()` is non-blocking: returns `Some(v)`
if a value is waiting (and consumes it), `None` otherwise. Dropped
values are fine because the next `signal()` overwrites the channel
with the latest reading.

This shape works because every signal we care about is a
**latest-wins** observation: "what's the current ambient lux?" not
"how many lux readings have I missed?". The exception is event-driven
signals (`TAP_SIGNAL`, `REMOTE_SIGNAL`); for those, a missed signal
means a missed tap, but they fire so rarely (sub-Hz) that the render
task's 33 ms tick virtually never coincides with the signal write.

## Catalog

| Signal | Producer | Consumer | Cadence |
|---|---|---|---|
| `audio::AUDIO_RMS_SIGNAL` | audio task RX RMS loop | render → `MouthFromAudio` + `EmotionFromVoice` | ~33 ms |
| `touch::TAP_SIGNAL` | touch task / button task | render → `EmotionFromTouch` | event |
| `ir::REMOTE_SIGNAL` | IR RMT task | render → `EmotionFromRemote` | event |
| `imu::IMU_SIGNAL` | IMU polling | render → `avatar.accel_g`, `gyro_dps` | 10 ms |
| `ambient::AMBIENT_LUX_SIGNAL` | LTR-553 polling | render → `avatar.ambient_lux` | 500 ms |
| `power::POWER_STATUS_SIGNAL` | AXP2101 polling | render → `avatar.battery_percent` | 1000 ms |
| `head::POSE_SIGNAL` | render task | head task → SCServo | 33 ms |
| `head::HEAD_POSE_ACTUAL_SIGNAL` | head task readback | render → `avatar.head_pose_actual` | 1000 ms |
| `leds::LED_FRAME_SIGNAL` | render task | led task → PY32 | 33 ms |
| `camera::CAMERA_FRAME_SIGNAL` | camera DMA task | render task → blit | gated |
| `camera::CAMERA_MODE_SIGNAL` | button task | render + camera tasks | event |
| `camera::CAMERA_CAPTURE_REQUEST` | render task | camera task | event |

## Watchdog supervision

The watchdog task (see `src/watchdog.rs`) doesn't peek the signals —
that would race the render task's drain. Instead, each periodic
producer increments an `AtomicU32` heartbeat counter once per loop
iteration, and the watchdog polls those counters every 5 s. When a
counter doesn't advance enough relative to its expected cadence, the
watchdog logs `WARN watchdog: channel '<name>' silent`. See
[architecture](architecture) for the full task graph and watchdog
placement.
