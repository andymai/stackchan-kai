# AGENTS.md

A playbook for AI agents collaborating on `stackchan-kai`. Read this once
per session before reaching for code. CLAUDE.md is the shared
human+agent guide; this file is agent-specific.

## At a glance

- **Stack:** `no_std` + `alloc` Rust on ESP32-S3 (CoreS3), embassy executor, defmt logging over USB-Serial-JTAG.
- **Domain model:** `Entity` + `Director` + `Modifier` / `Skill` traits in `stackchan-core` (pure, host-testable). Firmware composes a fixed modifier stack at 30 FPS in `render_task`.
- **Cross-task communication:** typed `Signal<RawMutex, T>` channels. Sensors publish via `signal()`; consumers drain via `try_take()`. Latest-wins semantics throughout.
- **Tests:** `stackchan-sim` runs modifiers against `FakeClock` for deterministic golden assertions. Firmware-side tests use on-device benches (`examples/*_bench.rs`).

## Session shapes

Most useful sessions fit one of these shapes. Recognising the shape early
keeps the work scoped.

### Shape 1: behavior change (modifier authoring)
1. Read the relevant modifier in `crates/stackchan-core/src/modifiers/`.
2. Add or change behavior; update the unit tests in the same file.
3. Add a sim-level integration test in `crates/stackchan-sim/src/lib.rs`.
4. `just check` — gates pass before flashing.
5. Optional: `just fmr` to confirm on hardware.

### Shape 1b: skill authoring
Skills are predicate-fired capabilities that write `mind.intent` /
`mind.attention` / `voice` / `events`; modifiers in later phases
translate that into face / motor.
1. Read `crates/stackchan-core/src/skill.rs` for the trait and pick
   any existing impl in `crates/stackchan-core/src/skills/` as a
   starting point.
2. Add the new file under `crates/stackchan-core/src/skills/`,
   re-export from `mod.rs`.
3. Register in `render_task` via `director.add_skill(&mut x)`.
4. If the skill's intent needs a visible response, add or extend a
   modifier (`Phase::Motion` for pose, `Phase::Expression` for
   face) that reads `mind.intent` / `mind.attention`.

### Shape 2: driver work (one of the 14 driver crates)
1. Read the crate's README to understand current scope.
2. Add registers, driver methods, or init steps; keep `embedded-hal-async` boundary clean.
3. Unit-test what you can on host (packet construction, register encoding).
4. Add or update a `crates/stackchan-firmware/examples/<chip>_bench.rs` that exercises the new path on hardware.
5. `just <chip>-bench` to verify on device.

### Shape 3: firmware integration
1. Identify the `Signal<…, T>` channel(s) the new feature needs.
2. Producer goes in a per-peripheral task (`src/<chip>.rs`); consumer reads in `render_task`.
3. Update `Entity` (or a sub-component like `Face` / `Motor` / `Perception`) if the feature surfaces persistent state; remember to extend `Face::frame_eq` only if it's pixel-affecting.
4. `cargo +esp clippy --release -- -D warnings` from the firmware crate.
5. Hardware-verify boot and runtime via `just fmr`.

### Shape 4: docs / tooling
1. CLAUDE.md is shared (humans + agents). AGENTS.md is agent-only. `docs/` is for cross-cutting reference (e.g., `docs/errors.md`, `docs/http.md`).
2. Per-crate READMEs document API + gotchas — the pre-commit hook reminds you to review them when source changes.
3. justfile recipes are the project's idiomatic invocation surface — prefer adding a recipe to documenting a long invocation in prose.

### Shape 5: HTTP route or network feature
1. Wire format: parsers + validators in `crates/stackchan-net/src/{config,http_command,http_parse,bare_json}.rs`. Host-testable; unit tests live beside the parser.
2. Handler: `crates/stackchan-firmware/src/net/http.rs` matches requests by `(method, path)`; each route is a handler function.
3. Persisted state rides the RON schema (`stackchan_net::config::Config`) through `PUT /settings`'s atomic writeback — no parallel persistence paths.
4. Operator-driven routes update the dashboard at `crates/stackchan-firmware/src/net/dashboard.html` (embedded via `include_bytes!`).
5. `just check-firmware && just clippy-firmware && just build-firmware`, then curl smoke after flashing.
6. Document the route in `docs/http.md` (the canonical reference).

## Decision frameworks

- **Ask vs assume:** ask when the change spans multiple PRs, when a public API surface changes, or when a doc rewrite is implied. Otherwise assume and proceed; the user can course-correct.
- **One PR per feature:** the recent network arc (#134–#168) shows the cadence — a large new surface broken into thematically tight PRs, capped with a self-audit (#159) and a string of refactor lifts that pull duplicated firmware helpers up into `stackchan-net`. Greptile reviews are tighter on small PRs.
- **Hardware verification:** required for any firmware-touching PR. Skip only for pure host-side changes (sim tests, doc updates, host crate refactors).
- **Memory writes:** save hardware quirks, gotchas, and corrections — but never current task state. Use the `feedback` type for behavioral preferences and `project` for unit-specific or repo-specific facts.

## Patterns + idioms

- **Signal drain pattern:** `if let Some(x) = SOME_SIGNAL.try_take() { entity.perception.x = Some(x); }` — non-blocking, drops misses (the producer's next signal overwrites), runs once per render tick. SSE fan-out is the exception: it uses `embassy_sync::pubsub::PubSubChannel` because it has multiple consumers.
- **`frame_eq` gate:** the render task short-circuits LCD blits when no pixel-affecting field changed. New `Face` fields default to *excluded* from `frame_eq` unless they affect drawing.
- **Per-modifier state:** modifiers own their state (timers, RNG, pending transitions). `update(&mut self, &mut Entity)` is the only mutation surface; time flows in via `entity.tick.now`.
- **Errors:** typed across the workspace via `thiserror` (host) or `defmt::Format` derives (firmware). See `docs/errors.md`.

## Debugging recipes

### Reading boot logs
The boot log lives at `/tmp/scfmr.log` after `just fmr` runs in tmux. Key
anchors to grep for:

```bash
grep -E "stackchan-firmware v|boot complete|panic|ERROR" /tmp/scfmr.log
```

A clean boot ends with `boot complete — idle heartbeat` around 1.4 s.

### Filtering chatty steady-state logs

The 1 Hz `head: cmd=… actual=…` and periodic `audio: DmaError(Late)`
warnings are expected. To watch only state changes:

```bash
tail -f /tmp/scfmr.log | grep -vE "head: cmd|DmaError\(Late\)|audio: RMS"
```

For compile-time filtering, set `DEFMT_LOG` before flashing:

```bash
DEFMT_LOG=info,stackchan_firmware::head=warn just fmr
```

### Known-noise log lines (do not debug)

These are pre-existing on Andy's specific kit — see memory note on unit hardware:

- `BMM150: not reachable on main I²C — likely wired to BMI270 AUX` — by design on this revision.
- `BMI270: init attempt 1/3 failed (I2c(I2c(Timeout))); retrying` — known timing wobble, succeeds on retry.
- `audio: DMA pop error ("DmaError(Late)"); publishing silence and resyncing` — periodic (every ~2 s); the resync logic recovers automatically.
- `FT6336U: vendor ID 0x01` — non-canonical but register-compatible; the touch driver works.

## Memory + context

The auto-memory store at
`/home/andy/.ccs/instances/home/projects/-var-home-andy-Git-stackchan-kai/memory/`
holds accumulated session-spanning context. Read `MEMORY.md` first; it
indexes feedback (preferences), project (repo-specific facts), and
reference entries.

Highlights to know at session start:

- USB-Serial-JTAG wedges on rapid back-to-back `espflash` invocations — prefer `just fmr`, use `just reattach` to pick up without reset.
- ES7210 has no documented chip-ID register and needs MCLK to answer I²C. AW88298 uses 16-bit big-endian registers.
- HTTP control plane lives in `crates/stackchan-firmware/src/net/`; wire formats + RON config schema are in `stackchan-net`. Auth is bearer-token, LAN-only by design — no TLS / CSRF / rate limit in v0.x (see `docs/http.md` security section). The firmware boots offline if the SD card is absent.
- The user opts out of proactive `/schedule` offers — don't end replies with "want me to schedule a follow-up?" pitches.
- Forward-looking PRDs and v2.x vision content go to the user's Obsidian vault, not the public repo. Repo docs stay tactical and present-tense.

## When you're stuck

Report blocking state explicitly: what completed, what's blocking, what
was attempted, what's needed from the user. Don't loop on the same
failing command.
