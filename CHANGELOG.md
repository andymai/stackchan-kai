# Changelog

All notable changes are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[SemVer](https://semver.org/) with the v0.x caveats in
[STABILITY.md](STABILITY.md).

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
