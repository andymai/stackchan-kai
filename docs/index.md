---
title: stackchan-kai engineering handbook
---

# stackchan-kai engineering handbook

Reference docs for the [stackchan-kai](https://github.com/andymai/stackchan-kai)
codebase — `no_std` Rust firmware for the M5Stack CoreS3 Stack-chan,
plus the host-side simulator + 14 driver crates that compose it.

This handbook complements the in-repo guides:

- **[CLAUDE.md](https://github.com/andymai/stackchan-kai/blob/main/CLAUDE.md)** — shared human + AI orientation: build commands, conventions, hardware notes.
- **[AGENTS.md](https://github.com/andymai/stackchan-kai/blob/main/AGENTS.md)** — agent-specific playbook: session shapes, debugging recipes, memory pointer.

## Pages

- [Architecture overview](architecture) — the firmware's task graph, the modifier stack, and how the host sim mirrors it.
- [Modifier authoring guide](modifiers) — how to add a new behavior to the avatar without breaking the canonical stack ordering.
- [Signal channels](signals) — the typed `Signal<…, T>` pattern that wires sensors to the render task.
- [Typed-error catalog](errors) — every driver crate's `Error<E>` enum, with what each variant means and what to do about it.

## Source layout

```
crates/
├── stackchan-core   # Pure no_std domain library (Avatar, Modifier, Pose, Clock)
├── stackchan-sim    # Host-side simulator (FakeClock, Framebuffer, viz binary)
├── stackchan-firmware  # CoreS3 firmware binary (embassy + esp-rtos)
├── tracker          # Block-grid motion tracker for the camera path
└── 14 driver crates # axp2101, aw9523, aw88298, bm8563, bmi270, bmm150,
                    # es7210, ft6336u, gc0308, ir-nec, ltr553, py32,
                    # scservo, si12t, st25r3916
```

## Stability model

`stackchan-kai` is `v0.x` until v1.0 ships its polish milestone.
v1.0 will not graduate any items to **Stable** — that's deferred to
v2.x so the API can survive the architectural work planned there. See
[STABILITY.md](https://github.com/andymai/stackchan-kai/blob/main/STABILITY.md)
for the full graduation rules.

Standalone driver crates (axp2101, aw9523, scservo, bm8563, etc.)
graduate on their own cadence, not gated on the workspace version.
