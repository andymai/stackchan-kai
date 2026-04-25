---
title: Modifier authoring guide
---

# Modifier authoring guide

A `Modifier` is a state machine that mutates the `Avatar` once per
frame. Modifiers compose by ordering — the canonical stack runs in a
fixed sequence in `render_task`, with each later modifier observing
the cumulative effect of all earlier ones.

## The trait

```rust
pub trait Modifier {
    fn update(&mut self, avatar: &mut Avatar, now: Instant);
}
```

That's the entire surface. Modifiers own their state (timers, RNG,
pending transitions) — `update` is the only mutation surface. Time
flows in via `Instant` (a wrapper around `embassy_time::Instant` on
firmware, `FakeClock`-driven on host).

## Steps to add a new modifier

1. **Choose the file.** New modifiers go under
   `crates/stackchan-core/src/modifiers/<your_name>.rs`.

2. **Implement the state machine.** Pattern:

   ```rust
   pub struct YourModifier {
       // state fields — timers, RNG, last-value
   }

   impl YourModifier {
       #[must_use]
       pub const fn new() -> Self { /* ... */ }
   }

   impl Modifier for YourModifier {
       fn update(&mut self, avatar: &mut Avatar, now: Instant) {
           // Read avatar fields the modifier is aware of.
           // Mutate avatar fields the modifier owns.
       }
   }
   ```

3. **Add unit tests.** Each modifier file has a `mod tests` at the
   bottom that exercises edge cases against `Instant::from_millis(...)`
   sequences. Pattern: assert specific avatar fields after a known
   tick sequence.

4. **Wire into `render_task`.** Open `crates/stackchan-firmware/src/main.rs`,
   construct your modifier alongside the others, and call
   `your_modifier.update(&mut avatar, now)` in the canonical order.
   Update the boot info-line listing modifiers in the modifier stack.

5. **Add a sim integration test.** In
   `crates/stackchan-sim/src/lib.rs`'s `integration_tests` mod, add a
   test that drives the new modifier through a realistic time sequence.
   Use `FakeClock` for deterministic time.

6. **Update the boot-log golden.** If your modifier produces a new
   info-line at startup (e.g. "your-modifier: enabled"), add the
   substring to `tests/golden/boot.txt`.

## Reading vs writing avatar fields

Avatar fields fall into two buckets:

**Pixel-affecting** — fields that change the rendered face: `left_eye`,
`right_eye`, `mouth`, `emotion`, `eye_curve`, `mouth_curve`,
`cheek_blush`, etc. These are listed in `Avatar::frame_eq`. New
pixel-affecting fields **must** be added to `frame_eq` so the render
task's dirty-check works correctly.

**Non-pixel** — sensor inputs and motor outputs: `head_pose`,
`accel_g`, `gyro_dps`, `ambient_lux`, `battery_percent`, `audio_rms`,
etc. These are *excluded* from `frame_eq` because they don't directly
affect drawn pixels. Modifiers translate them into pixel-affecting
state (e.g. `LowBatteryEmotion` reads `battery_percent` and writes
`emotion`).

## When ordering matters

The canonical stack ordering is non-trivial. Examples:

- `EmotionTouch` → `EmotionCycle` — touch must run first so a manual
  tap-set emotion overrides cycle's auto-rotation gate.
- `EmotionStyle` → `Blink` / `Breath` — style sets baseline weights;
  blink/breath add subtle modulation on top.
- `IdleDrift` (eye position) → `IdleSway` (head pose) — eye drift is
  pixel-affecting and runs in the visual stack; sway is motor-only and
  runs after.
- `MouthOpenAudio` runs last so audio-driven mouth-open isn't
  overwritten by a stale `Blink` write.

When in doubt, mirror the order in `render_task` exactly and add a
`stackchan-sim` integration test that asserts the visible behavior
(eyes don't walk off-screen, mouth doesn't oscillate at 60 Hz, etc.).
