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

### Audio

- ES7210 PCM capture beyond RMS. The audio task publishes 20 ms
  (320-sample) frames onto `AUDIO_FRAME_PUBSUB` (a `PubSubChannel`
  with one publisher and up to four subscribers) alongside the
  existing `AUDIO_RMS_SIGNAL`. Per-subscriber lag surfaces as
  `WaitResult::Lagged(n)` rather than producer-side failure;
  `AUDIO_FRAME_CAPTURED` tracks total publishes for health probing.
  Foundation for the wake-word + push-to-talk + UDP audio debug
  paths; consumer-side code lands in follow-up PRs.
- UDP audio debug stream — opt-in via
  `behavior.audio_debug_udp_target`. A new task subscribes to
  `AUDIO_FRAME_PUBSUB` and forwards each 20 ms frame as a raw
  little-endian `s16` UDP datagram, listenable on the host with
  `nc -lu <ip> <port> | aplay -r 16000 -f S16_LE -c 1 -t raw`. Empty
  target parks the task with no resource cost. Bench-only; see
  [docs/audio-debug.md](./docs/audio-debug.md).
- Sidecar agent client — opt-in via `behavior.agent_sidecar_url`.
  Closes the loop on the operator-visible "speak to the avatar,
  get a reply" path. A new firmware task intercepts every
  `POST /listen` (or MCP `start_listen`), drains pre-trigger
  pubsub backlog so capture starts at the trigger edge,
  accumulates the requested capture window into a PSRAM `Vec<i16>`,
  and POSTs the raw little-endian s16 PCM to the operator's
  sidecar URL with `Content-Type: audio/L16;rate=16000;channels=1`.
  The sidecar's JSON reply (`{"text":"...","emotion":"..."}`, a
  minimal projection of OpenAI Chat Completions) surfaces on the
  firmware toast band, and any `emotion` tag fires a `SetEmotion`
  hold. STT, LLM, and emotion-tagging live in the operator's
  sidecar — kai stays no_std + local-first; the operator chooses
  cloud-or-not by where they point the URL. Schema, curl recipe,
  and minimal `nc`-based echo sidecar in
  [docs/sidecar.md](./docs/sidecar.md). HTTP request/response
  buffers in the control plane bumped from 1 KiB to 2 KiB to fit
  realistic sidecar URLs alongside the existing config fields.
- New `stackchan-audio-features` crate — `no_std` + `alloc`
  streaming mel-spectrogram frontend (Hann window → 512-point
  real FFT → 40-channel mel filterbank 125–7 500 Hz → log →
  int8 quantize). Matches the published `microWakeWord` /
  `TFLite-micro` keyword-spotting feature shape so any future
  on-device wake-word inference path (TFLite-micro via FFI,
  pure-Rust port, custom MixConv interpreter) can plug in
  without re-deriving the DSP layer. 23 host tests cover Hann
  symmetry, mel partition-of-unity, quantization rounding /
  saturation / log floor, streaming window/hop cadence, and a
  1 kHz pure-tone fixture. Foundation only — no firmware
  consumer yet; the classifier + bundled model land in a
  follow-up. Spectral subtraction and PCAN gain control from
  the reference frontend are out of scope for this slice.

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
  `list_reminders` / `cancel_reminder` / `push_toast`. Reminders are
  monotonic (fire-in-N-seconds), in-RAM, capped with a five-day
  horizon. `push_toast` mirrors `POST /toast` for MCP clients —
  requires `behavior.toast_overlay_enabled` for the band to render.
- Time configuration honours the `time.tz` field at boot — a small
  catalog of named IANA zones plus `Etc/GMT±N` is enough for the
  desk-toy use case without pulling in a real timezone library.
- `GET /state/ws` — WebSocket transport (RFC 6455) parallel to the
  existing `/state/stream` Server-Sent Events path. Same snapshot
  payload; same publisher; operators pick whichever transport
  their dashboard library speaks. Server-push only (handshake +
  text frames + a 15 s ping); client-sent frames are ignored.
  Bidirectional support, binary opcodes, and per-frame masking
  are out of scope for v1. RFC 6455 handshake + frame primitives
  in `net/websocket.rs` cover handshake-key SHA-1 / base64,
  case-insensitive header lookup, and short / extended-length
  frame headers.
- Mimic-follower — opt-in via `behavior.follower_leader_hostname`.
  The firmware already advertises live `yaw` / `pitch` on its own
  mDNS TXT record (the leader half of the meganetaaan `mimic_main`
  protocol); this PR closes the loop by inspecting every inbound
  mDNS multicast packet and applying any TXT record from the
  configured leader as a 1.5 s `RemoteCommand::LookAt` hold. No
  HTTP round-trip; both devices must share a multicast LAN
  segment. `parse_response_pose` in `stackchan-net::mdns_pose` is
  the host-testable parser — 11 host tests cover synthetic
  fixtures, case-insensitive matching, compression pointers, and
  a one-cycle pointer-loop bail-out.

### Stability + plumbing

- Modifier registry capacity bumped to accommodate the new
  symbolic-overlay and fallback-gaze modifiers.
- Greptile-driven post-merge fix on the ESP-NOW TX path: the
  dispatcher no longer starves under inbound traffic and no longer
  latches a NaN pose into the delta-comparison cache.

### Documentation

- README gains a `Known limitations` section spelling out
  single-unit hardware test scope (BMM150 bench-only), LAN-only
  HTTP control plane with no TLS, Experimental-until-v2.x API
  contract, and the best-effort single-maintainer cadence. Pairs
  with the existing `Non-goals` section so a 30-second-rubric
  reader sees both what the firmware won't do by design and where
  the present implementation has gaps.
- README `Why` section trimmed of unverified competitive
  framings; the factual contrast against the upstream `xiaozhi`
  firmware (cloud-dependent LLM agent in C++) stays.
- Self-applied `unsafe denied` shield removed from the README
  header; the same claim already appears under `Features`, and
  the remaining badges (`CI`, `Release`, `License`, `Rust 1.88+`)
  are independently validated by GitHub Actions, GitHub Releases,
  the `LICENSE` files, and `rust-toolchain.toml`.

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
