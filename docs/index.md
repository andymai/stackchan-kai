---
title: stackchan-kai engineering handbook
---

# stackchan-kai engineering handbook

Reference docs for [stackchan-kai](https://github.com/andymai/stackchan-kai)
— `no_std` Rust firmware for the M5Stack CoreS3 Stack-chan, plus the
host simulator and 14 driver crates.

## Pages

- [Architecture overview](architecture) — the engine, the firmware's task graph, and how the host sim mirrors it.
- [HTTP control plane](http) — LAN-scoped routes for live state, manual control, persistent config, and the embedded operator dashboard.
- [Modifier authoring guide](modifiers) — adding a new behavior to the engine.
- [Naming conventions](naming) — rules for `Intent` / `Modifier` / `Skill` / `ChirpKind` / `OverrideSource` names, with citations.
- [Signal channels](signals) — the typed `Signal<…, T>` pattern that wires sensors to the render task.
- [Typed-error catalog](errors) — every driver crate's `Error<E>` enum.

## Source layout

```
crates/
├── stackchan-core   # no_std engine (Entity, Director, Modifier, Skill, Phase, Clock)
├── stackchan-sim    # host simulator (FakeClock, Framebuffer, viz binary)
├── stackchan-firmware  # CoreS3 firmware binary (embassy + esp-rtos)
├── tracker          # block-grid motion tracker for the camera path
└── driver crates    # axp2101, aw9523, aw88298, bm8563, bmi270, bmm150,
                    # es7210, ft6336u, gc0308, ir-nec, ltr553, py32,
                    # scservo, si12t
```

See [STABILITY.md](https://github.com/andymai/stackchan-kai/blob/main/STABILITY.md)
for the graduation rules.
