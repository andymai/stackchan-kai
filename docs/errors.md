---
title: Typed-error catalog
---

# Typed-error catalog

Every driver crate in this workspace exposes a generic `Error<E>` enum.
The `E` parameter is the underlying transport error (I²C, SPI, or UART),
which the firmware wraps with `Debug2Format` for `defmt` logging.

This page summarises every variant across the workspace so a new
contributor (or an AI agent debugging a panic trace) can map an error
log line to its root cause without grepping the source.

## Common pattern: transport vs protocol

Most drivers split errors into two categories:

- **Transport** — the bus rejected the operation. Bus glitch, missing
  pull-up, chip-not-present, wrong address. Carries the inner bus error
  (e.g. `embedded_hal_async::i2c::ErrorKind::ArbitrationLoss`).
- **Protocol** — the chip answered but the response was wrong. Bad
  chip-ID, malformed packet, checksum mismatch, value out of range.
  Carries the offending byte(s) so logs are self-describing.

Recovery hint: transport errors usually warrant a retry; protocol errors
usually mean a wiring or firmware bug and should not be retried blindly.

## Per-crate catalog

### `axp2101::Error<E>`
PMU init / battery gauge.
- `I2c(E)` — bus error.

### `aw9523::Error<E>`
I/O expander init.
- `I2c(E)` — bus error.

### `aw88298::Error<E>`
Mono Class-D amplifier (TX path).
- `I2c(E)` — bus error.
- `BadChipId(u16)` — `CHIPID` register didn't return expected ID. AW88298 uses 16-bit big-endian registers; raw value is in the variant.

### `bm8563::Error<E>`
RTC.
- `I2c(E)` — bus error.
- `VoltageLow` — `VL` flag set; backup battery dropped, current time unreliable. Treat as "unset."

### `bmi270::Error<E>`
6-axis IMU.
- `I2c(E)` — bus error.
- `BadChipId(u8)` — wrong device, bus glitch, or held in reset.
- `NotDetected` — neither primary nor secondary I²C address answered.
- `InitTimeout` — config blob upload didn't finish; usually truncation or defective part.

### `bmm150::Error<E>`
Magnetometer.
- `I2c(E)` — bus error.
- `ChipId { expected, actual }` — caller targeted the wrong address or part is damaged.
- `NotDetected` — both candidate addresses failed to respond.
- `Overflow` — reading exceeded ADC dynamic range; retry or ignore the sample.

### `es7210::Error<E>`
4-channel ADC (RX path).
- `I2c(E)` — bus error.
- `BadChipId(u8, u8)` — `(CHIP_ID1, CHIP_ID2)` mismatch. ES7210 needs MCLK to answer I²C — see memory note on audio codec quirks.

### `ft6336u::Error<E>`
Capacitive touch.
- `I2c(E)` — bus error.

### `gc0308::Error<E>`
Camera SCCB.
- `I2c(E)` — SCCB (I²C-compatible) bus error.
- `BadChipId(u8)` — wrong device or wiring issue.

### `ltr553::Error<E>`
Ambient + proximity.
- `I2c(E)` — bus error.
- `BadPartId(u8)` — `PART_ID` upper nibble mismatch; common cause is a related Lite-On part with a different lux formula.

### `py32::Error<E>`
Co-processor (servo power gate, LED ring).
- `I2c(E)` — bus error.
- `InvalidPin(u8)` — caller passed pin outside `0..=13`.
- `TooManyLeds(usize)` — caller passed more than `MAX_LEDS` pixels.

### `scservo::Error<E>`
Half-duplex serial servo bus.
- `Uart(E)` — UART transport error.
- `PayloadTooLarge` — write exceeded `MAX_DATA_BYTES`; v1 write surface never reaches this.
- `NoResponse` — UART closed before full response (rare on open-ended serial; usually a hung slave).
- `MalformedResponse` — wrong header bytes or response ID mismatch.
- `ChecksumMismatch` — packet arrived but checksum failed.
- `PositionOutOfRange(u16)` — position above `POSITION_MAX` (1023); the servo would mis-interpret high bits.
- `BroadcastNotAllowed` — caller used `BROADCAST_ID` on an operation requiring a response.

### `si12t::Error<E>`
- `I2c(E)` — bus error during init or `OUTPUT1` read.

### `ir-nec`
No `Error` enum — decoder returns `Option<NecCommand>` and treats noise as "no decode" rather than an error.

## Firmware-side error wrapping

`stackchan-firmware` does not introduce its own typed-error enum. Init
failures inside `#[esp_rtos::main]` panic via `defmt::panic!` because
there's no caller to bubble to. Per-task failures are logged with
`defmt::warn!` or `defmt::error!` and the task continues, parks
forever, or restarts depending on severity (see each `src/<chip>.rs`).

## Adding a new error variant

When adding a variant to a driver crate's `Error`:

1. Document **what triggered** it and **what to do about it** in the
   doc-comment — the catalog above relies on this to stay accurate.
2. Carry the offending value (`u8`, `u16`, etc.) when it aids
   debugging. Logs that just say "bad chip ID" without the byte are
   hard to triage.
3. Update this catalog in the same PR.
