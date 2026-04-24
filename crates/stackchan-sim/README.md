---
crate: stackchan-sim
role: Headless simulator for stackchan-core
bus: none
transport: "Vec<Rgb565> framebuffer + FakeClock"
no_std: false (host-only, uses alloc)
unsafe: forbidden
status: stable
---

# stackchan-sim

Host-side simulator for `stackchan-core`. Runs the full domain model —
modifier pipeline, `Avatar::draw`, `HeadDriver` — on the dev machine
with a deterministic clock and a `Vec<Rgb565>`-backed framebuffer, so
most of the firmware's behaviour is testable without flashing hardware.

## Key Files

- `src/lib.rs` — `FakeClock`, `Framebuffer`, `RecordingHead`
- `tests/head_sway.rs` — golden test for the head-sway trajectory: feed a clock forward in controlled steps, assert that the captured `(Instant, Pose)` sequence matches expectations
- `tests/leds.rs` — LED-ring rendering regression tests
- `tests/render_snapshot.rs` — one-minute full-stack cadence test that renders `Avatar::draw` into the framebuffer and compares pixel hashes

## Architecture

```mermaid
flowchart LR
    subgraph "Test / bench"
        T[Test case]
    end
    subgraph "stackchan-sim"
        FC[FakeClock<br/><i>tests.advance(dt)</i>]
        FB[Framebuffer<br/><i>Vec&lt;Rgb565&gt;</i>]
        RH[RecordingHead<br/><i>Vec&lt;(Instant, Pose)&gt;</i>]
    end
    subgraph "stackchan-core"
        A[Avatar]
        Mods[Modifier stack]
        Draw[Avatar::draw]
    end

    T -->|advance| FC
    FC --> Mods
    Mods --> A
    A --> Draw
    Draw -->|pixels| FB
    A --> RH
    FB --> T
    RH --> T
```

## What It Provides

- **`FakeClock`** — deterministic `Clock` impl backed by a `Cell<Instant>`. `advance(delta_ms)` and `set(instant)` are the only ways time moves. `now()` is re-entrant safe (takes `&self` via `Cell`)
- **`Framebuffer`** — `width × height` `Vec<Rgb565>` that implements `embedded_graphics::DrawTarget<Color = Rgb565>` with `Infallible` errors. Out-of-bounds pixels are silently clipped (matches how `embedded-graphics` clips to `OriginDimensions`). `pixel(x, y) → Option<Rgb565>` for read-back
- **`RecordingHead`** — `HeadDriver` impl that pushes every `(Instant, Pose)` call into a `Vec`, so motion-modifier tests can assert on the full trajectory

## Test Patterns

- **Golden pixel snapshots.** Render a frame, hash the pixel buffer, compare to a stored digest. Catches regressions in `Avatar::draw` or any upstream modifier that changes displayed output
- **Golden trajectory.** Advance the clock in documented steps, assert on the `(Instant, Pose)` sequence `RecordingHead` captured. Catches timing drifts in motion modifiers
- **Full-stack cadence.** Run the complete firmware modifier pipeline for one minute of simulated time, sample the framebuffer periodically, confirm the face looks right at known timestamps (emotion cycle boundaries, blink instants, etc.)

## Gotchas

1. **No `no_std` here.** The sim lives on the host; it uses `alloc` freely for the pixel buffer and pose trajectory. Don't import from this crate into firmware code
2. **FakeClock is not monotonic-enforcing.** `set()` trusts the caller; a test can intentionally go backward to exercise a pathological path. In-test assertions need to know what they're asserting
3. **Framebuffer clipping is silent.** Pixels written outside `width × height` don't error — they disappear. Match the framebuffer size to the firmware's (320×240 for CoreS3)
4. **Pixel-golden tests are sensitive to embedded-graphics upgrades.** Any sub-pixel rendering change in the upstream crate invalidates the stored hashes. Update deliberately when bumping the dep
5. **`RecordingHead` has unbounded memory.** Long-running tests will accumulate millions of `(Instant, Pose)` entries — use `clear()` between phases, or cap simulated duration

## Integration

- **Runs on every `cargo test`** as part of the host-side default-members. No hardware required
- **Paired with `stackchan-core`** — the sim consumes the same `Avatar` / `Modifier` / `Clock` / `HeadDriver` types the firmware does. Any behaviour change in core surfaces here first
- **Future:** add a `Scenario` harness that scripts sensor signals (touch taps, IR commands) against the full modifier pipeline for integration tests
