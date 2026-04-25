---
title: Modifier authoring guide
---

# Modifier authoring guide

A `Modifier` is a state machine that mutates an `Entity` once per
frame. Modifiers compose by registration order with a [`Director`]
that sorts them by `(phase, priority, registration_order)` before the
first tick. Each later modifier observes the cumulative effect of all
earlier ones.

## The trait

```rust
pub trait Modifier {
    fn meta(&self) -> &'static ModifierMeta;
    fn update(&mut self, entity: &mut Entity);
}
```

Two methods. `meta` returns a `&'static ModifierMeta` constant
declaring `name`, `description`, `phase`, `priority`, and the `reads`
/ `writes` field sets. `update` is the only mutation surface — read
the entity, decide, write back. Time flows in via `entity.tick.now`
(stamped by `Director::run`); modifiers don't take a `now` parameter.

## Steps to add a new modifier

1. **Choose the file.** New modifiers go under
   `crates/stackchan-core/src/modifiers/<your_name>.rs`.

2. **Implement the state machine.** Pattern:

   ```rust
   use crate::director::{Field, ModifierMeta, Phase};
   use crate::entity::Entity;
   use crate::modifier::Modifier;

   pub struct YourModifier {
       // state fields — timers, RNG, last-value
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
               phase: Phase::Expression, // or Affect / Motion / Audio / ...
               priority: 0,
               reads: &[/* Field::... */],
               writes: &[/* Field::... */],
           };
           &META
       }

       fn update(&mut self, entity: &mut Entity) {
           let now = entity.tick.now;
           // Read entity fields the modifier is aware of.
           // Mutate entity fields the modifier owns.
       }
   }
   ```

3. **Pick a phase.** See the architecture doc's phase table. Most
   custom behaviors are `Affect` (decide an emotion), `Expression`
   (modulate face style), `Motion` (head pose), or `Audio`
   (audio-driven visual).

4. **Add unit tests.** Each modifier file has a `mod tests` at the
   bottom that exercises edge cases against `Instant::from_millis(...)`
   sequences. Pattern: assert specific entity fields after a known tick
   sequence. Set the time on `entity.tick.now`, call `update`,
   assert.

5. **Wire into `render_task`.** Open
   `crates/stackchan-firmware/src/main.rs`, construct your modifier
   alongside the others, and call
   `director.add_modifier(&mut your_modifier).expect("registry full")`
   in the canonical order. Update the boot info-line listing.

6. **Add a sim integration test if behavior is non-local.** In
   `crates/stackchan-sim/src/lib.rs`'s `integration_tests` mod, add a
   test that drives the new modifier through a realistic time sequence
   alongside related modifiers. Use `FakeClock` for deterministic
   time.

## Reads vs writes

Entity fields fall into a few buckets:

- **Pixel-affecting** (`face.*`): listed in `Face::frame_eq`. New
  pixel-affecting fields **must** be added to `frame_eq` so the render
  task's dirty-check works correctly.
- **Sensor inputs** (`perception.*`): firmware writes from sensor
  drains; modifiers read.
- **Pending inputs** (`input.tap_pending`, `input.remote_pending`):
  firmware writes; the consuming modifier reads + clears in the same
  tick.
- **Cognitive state** (`mind.*`): emotion modifiers write
  `mind.affect.emotion` + `mind.autonomy.manual_until`; downstream
  modifiers read.
- **Output requests** (`voice.chirp_request`): modifiers write a
  request; firmware reads + clears after `Director::run`.

The `reads` / `writes` slices on `ModifierMeta` are documentation
today. v2.x will enforce them via debug-mode assertions after each
`update` — a write to an undeclared field becomes a panic in debug
builds.

## Field granularity

`Field` is fine-grained per-leaf-field (e.g. `LeftEyePhase` vs
`LeftEyeWeight`) so different sub-fields of the same component don't
false-flag as conflicts. `Field::group()` buckets fine variants into
coarse `FieldGroup`s for human-readable conflict reports.

## When ordering matters

The canonical Affect phase ordering is non-trivial. Examples:

- `EmotionTouch` runs first (priority `-100`) — a tap queued from
  the touch task must take effect before any environmental override.
- `EmotionCycle` runs last in Affect — the autonomous advancer only
  fires when no input modifier set `mind.autonomy.manual_until`.
- `EmotionStyle` runs in `Expression` and reads
  `mind.affect.emotion` set by Affect modifiers; Blink / Breath /
  IdleDrift then add per-frame deltas on top.
- `IdleSway` (Motion) writes the base `motor.head_pose`;
  `EmotionHead` (also Motion, registered later) adds an emotion-keyed
  bias on top.
- `MouthOpenAudio` runs in `Audio` so audio-driven mouth-open isn't
  overwritten by a stale earlier write.

When in doubt, mirror the registration order in `render_task` and add
a `stackchan-sim` integration test that asserts the visible behavior
(eyes don't walk off-screen, mouth doesn't oscillate at 60 Hz, etc.).

## Skills (not modifiers)

A modifier is the right shape when the behavior runs every frame and
its job is to translate state. A `Skill` is the right shape when the
behavior is **discoverable** — the kind of thing a future LLM
dispatcher might pick from a menu by reading its description. Skills
have `should_fire(&Entity)` + `invoke(&mut Entity)` + a richer
`SkillMeta` and are bound by a doc-enforced rule: **no direct writes
to `face` or `motor`** (skills go through `mind` / `voice` /
`events`; modifiers translate). Trait surface ships today; zero skill
implementations have landed.

[`Director`]: https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-core/src/director.rs
