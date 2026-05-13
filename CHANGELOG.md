# Changelog

All notable changes are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[SemVer](https://semver.org/) with the v0.x caveats in
[STABILITY.md](STABILITY.md).

## [Unreleased] — v0.2.0 parity push

The desk-toy surface keeps growing while staying fully local — no LLM, no
cloud. Per-crate version bumps land on each PR via release-please; this
section summarises the milestone work in human terms.

### Avatar surface

- On-screen battery indicator drawn in the top-left corner; opt-in
  via `behavior.battery_icon_enabled`. The percent reading is
  quantised into five buckets (Critical / Low / Medium / High /
  Full) at the source so a one-percent gauge twitch doesn't
  re-trigger the renderer's dirty-check; the charging flag adds a
  small bolt overlay when USB power is present.
- `IdleMicroExpression` modifier: random small vertical perturbation
  of `mouth.center.y` every 2–6 s. Complements the eye-only
  `IdleDrift` so a long-quiet face shows mouth-side liveness too;
  composes additively with `Breath` via independent
  per-modifier offset tracking.
- Toast overlay (firmware-only): opt-in via
  `behavior.toast_overlay_enabled`, with a `crate::toast::push` API
  any task can call to surface a 3-second warn / error band at the
  bottom of the LCD. Operator-driven verification path exposed at
  `POST /toast` (`{level, message}`).
- Named one-shot motions: `POST /motion` + MCP `play_motion` route
  the four canonical gestures (`greet` / `nod` / `shake` / `laugh`)
  through the existing dance-player path. Each returns the head to
  baseline pose on exit so a follow-up command starts from a known
  state.
- Speech-bubble text overlay drawn above the face, with a TTL the
  `BubbleExpiry` modifier sweeps each frame.
- Decorator badges (heart, sweat, dizzy, ear, pairing, angry, shy)
  sit on top of the base face. Emotion edges trigger `Angry` / `Shy`
  automatically; reactive triggers still fire `Heart` / `Sweat` /
  `Dizzy` / `Ear` / `Pairing`.
- Runtime color palette swap through named presets (`default` / `dark`
  / `cute` / `dog`) — affects the four "skin" colours of the avatar
  while symbolic overlays keep their dedicated colours.
- Lifelike-gaze fallback: `LostTargetSearch` modifier emits a brief
  directional saccade after a tracked target disappears, with sourced
  200 ms saccade latency and 0.5–1.5 s microsaccade intervals.
- Optional soliloquy mode (`behavior.soliloquy_enabled`) — random
  idle bubbles at random intervals; yields gracefully to operator-set
  bubbles. Off by default.
- Optional hourly chime (`behavior.hourly_chime_enabled`) — a short
  `WakeChirp` at every wall-clock top-of-hour. Off by default; gated
  on an SNTP-synced RTC.
- Auto-torque-release: `behavior.auto_torque_release_ms` releases
  holding torque on the `SCServo` pan / tilt motors after the
  configured idle window with no commanded pose change. The next
  commanded change re-enables torque before the position write.
  `0` (default) keeps torque on continuously.
- Head offsets persist to `/sd/RUNTIME.RON`. Previously,
  `POST /head/offsets` was runtime-only and a reboot zeroed the
  dialled trim; now the operator's value rides through the same
  SD-backed runtime store as `palette` / `mood` / `face_geometry`
  and is restored to `OFFSETS_CACHE` + signalled to the head task
  before the first tick.

### Networking + control plane

- mDNS extends from hostname-only `A` records to full DNS-SD service
  advertising for `_stackchan._tcp.local.` (PTR + SRV + TXT + A).
  TXT carries `kai=1` as a variant marker so kai-aware clients gate
  on extension endpoints; a generic Bonjour browser still lists the
  device alongside upstream stackchan units.
- ESP-NOW gains a TX path: pose-mirror frames when the head moves
  plus a heartbeat liveness signal, broadcast on the configured
  channel. RX continues to gate on the static-peer allowlist plus an
  opt-in pairing window.
- New HTTP routes: `POST /face-target` (normalised camera coordinates
  for an external CV pipeline), `POST /palette`, `POST /head/offsets`
  (runtime servo zero-point correction layered on top of the
  compile-time trim).
- MCP gains `set_volume` / `set_mute` / `create_reminder` /
  `list_reminders` / `cancel_reminder`. Reminders are monotonic
  (fire-in-N-seconds), in-RAM, capped with a five-day horizon.
- Time configuration honours the `time.tz` field at boot — a small
  catalog of named IANA zones plus `Etc/GMT±N` is enough for the
  desk-toy use case without pulling in a real timezone library.

### Stability + plumbing

- Modifier registry capacity bumped to accommodate the new
  symbolic-overlay and fallback-gaze modifiers.
- Greptile-driven post-merge fix on the ESP-NOW TX path: the
  dispatcher no longer starves under inbound traffic and no longer
  latches a NaN pose into the delta-comparison cache.

## [0.1.0] — 2026-04-23

First release. CoreS3 boots to a double-buffered 320×240 face that blinks,
breathes, drifts, and cycles through five emotions at a steady 30 FPS. The
domain library is `no_std` + host-testable; the firmware is a thin embassy
wrapper that shares its render path with a headless simulator.

### Core (`stackchan-core` + `stackchan-sim`)

- `Avatar::draw` renders to any `embedded_graphics::DrawTarget`, so firmware
  and sim exercise the same pixels.
- Modifier pipeline `EmotionCycle → EmotionStyle → Blink → Breath →
  IdleDrift`. `EmotionStyle` eases style fields (`eye_curve`, `mouth_curve`,
  `cheek_blush`, `eye_scale`, `blink_rate_scale`, `breath_depth_scale`)
  linearly over 300 ms so emotion transitions never snap. Default-sequence
  cycle: Neutral → Happy → Sleepy → Surprised → Sad on a 4 s dwell.
- `stackchan-sim` adds a `Vec<Rgb565>`-backed framebuffer for pixel-golden
  snapshot tests plus a one-minute full-stack cadence test.

### Firmware (`stackchan-firmware`)

- esp-rtos embassy boot on CoreS3 → AXP2101 LDO sequencing → AW9523 releases
  LCD reset → SPI2 + mipidsi ILI9342C init.
- 30 FPS render task with dirty-check (blits only when state changes) on a
  PSRAM-backed framebuffer; double-buffering eliminates tearing.
- defmt logs via esp-println's USB-Serial-JTAG transport; decoded host-side
  with `espflash monitor --log-format defmt`.

### `axp2101` driver

- Minimal `embedded-hal-async` I²C driver for the CoreS3 PMU covering
  ALDO1/2, BLDO1/2, DLDO1, and the power-on sequence.
- Full M5Unified-matching init (ADC, charger, button timing, reset policy)
  — keeps the LCD rails up under an idle render load.

### Hardware bring-up fixes

- `-Tlinkall.x` required; otherwise `.rodata_desc.appdesc` lands at a random
  offset and the 2nd-stage bootloader rejects the image.
- `#[used]` anchor on `ESP_APP_DESC` prevents `lto = "fat"` from stripping
  the app descriptor.
- CoreS3 internal I²C is `SCL=GPIO11`, `SDA=GPIO12` (not reversed).
- `defmt::timestamp!` needed under defmt 1.0.
- Explicit `BLDO1`/`BLDO2` voltage writes (`0x96`/`0x97 = 28`) in the
  AXP2101 init sequence; the PoR default is 0.5 V, not 3.3 V.
- `DLDO1` is the LCD backlight on CoreS3 (not a vibration motor); the
  init writes `0x99 = 28` for full brightness.
- Full M5Stack AW9523 init on LCD bring-up: both port-output + direction
  registers, GCR, LED-mode = GPIO, and `LCD_RST` pulsed on `P1_1`. The
  prior `P0_0`-only helper left the backlight-boost gate on P1 floating.

[0.1.0]: https://github.com/andymai/stackchan-kai/releases/tag/v0.1.0
