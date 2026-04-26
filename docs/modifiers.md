---
title: Modifier authoring guide
---

# Modifier authoring guide

A `Modifier` mutates an `Entity` once per frame. The [`Director`] sorts
registered modifiers by `(phase, priority, registration_order)` before
the first tick; each later modifier observes the cumulative effect of
all earlier ones.

## The trait

```rust
pub trait Modifier {
    fn meta(&self) -> &'static ModifierMeta;
    fn update(&mut self, entity: &mut Entity);
}
```

`meta` returns a `&'static ModifierMeta` constant declaring `name`,
`description`, `phase`, `priority`, and the `reads` / `writes` field
sets. `update` is the only mutation surface; time flows in via
`entity.tick.now` (stamped by `Director::run`).

## Adding a new modifier

1. Create `crates/stackchan-core/src/modifiers/<your_name>.rs`.

2. Implement the state machine:

   ```rust
   use crate::director::{Field, ModifierMeta, Phase};
   use crate::entity::Entity;
   use crate::modifier::Modifier;

   pub struct YourModifier {
       // state: timers, RNG, last-value
   }

   impl YourModifier {
       #[must_use]
       pub const fn new() -> Self { /* ... */ }
   }

   impl Modifier for YourModifier {
       fn meta(&self) -> &'static ModifierMeta {
           static META: ModifierMeta = ModifierMeta {
               name: "YourModifier",
               description: "One sentence: what triggers this and what \
                             entity fields it writes.",
               phase: Phase::Expression,
               priority: 0,
               reads: &[/* Field::... */],
               writes: &[/* Field::... */],
           };
           &META
       }

       fn update(&mut self, entity: &mut Entity) {
           let now = entity.tick.now;
           // Read + mutate entity fields.
       }
   }
   ```

3. Pick a phase. `Affect` decides emotions, `Expression` modulates face
   style, `Motion` writes head pose, `Audio` drives visual from audio.

4. Add unit tests in a `mod tests` at the bottom of the file. Set
   `entity.tick.now`, call `update`, assert.

5. Register in `crates/stackchan-firmware/src/main.rs::render_task`:
   `director.add_modifier(&mut your_modifier).expect("registry full")`.
   Update the boot info-line listing.

6. If the behavior is non-local, add a `stackchan-sim` integration
   test that drives the new modifier alongside related ones.

## Reads vs writes

Entity fields fall into a few buckets:

- Pixel-affecting (`face.*`): listed in `Face::frame_eq`. New
  pixel-affecting fields must be added to `frame_eq` so the render
  task's dirty-check works correctly.
- Sensor inputs (`perception.*`): firmware writes; modifiers read.
- Pending inputs (`input.tap_pending`, `input.remote_pending`):
  firmware writes; the consuming modifier reads + clears in the same
  tick.
- Cognitive state (`mind.*`): emotion modifiers write
  `mind.affect.emotion` + `mind.autonomy.manual_until`; downstream
  modifiers read.
- Output requests (`voice.chirp_request`): modifiers write; firmware
  reads + clears after `Director::run`.

The `reads` / `writes` slices on `ModifierMeta` are documentation;
debug-mode enforcement after each `update` is planned.

## Field granularity

`Field` is fine-grained per-leaf-field (e.g. `LeftEyePhase` vs
`LeftEyeWeight`) so different sub-fields of the same component don't
false-flag as conflicts. `Field::group()` buckets fine variants into
coarse `FieldGroup`s for human-readable conflict reports.

## Ordering

The Affect phase ordering matters:

- `EmotionFromTouch` runs first (priority `-100`) so a tap takes effect
  before any environmental override.
- `EmotionCycle` runs last in Affect; the autonomous advancer only
  fires when no input modifier set `mind.autonomy.manual_until`.
- `StyleFromEmotion` runs in `Expression` and reads
  `mind.affect.emotion`; Blink / Breath / IdleDrift then add per-frame
  deltas on top.
- `IdleSway` (Motion) writes the base `motor.head_pose`; `HeadFromEmotion`
  (registered later in Motion) adds an emotion-keyed bias on top.
- `MouthFromAudio` runs in `Audio` so audio-driven mouth-open isn't
  overwritten by a stale earlier write.

When in doubt, mirror the registration order in `render_task` and add
a sim integration test that asserts the visible behavior.

## Skills

A `Skill` has `should_fire(&Entity) -> bool` and
`invoke(&mut Entity) -> SkillStatus`, plus richer metadata than a
modifier. Use it when the behavior is discoverable — selected from a
menu rather than always-on. Skills don't write `face` or `motor`
directly; they go through `mind` / `voice` / `events` and modifiers
translate. The trait ships; no implementations have landed.

[`Director`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/director.rs
