---
crate: ir-nec
role: NEC IR-remote protocol codec (decoder + encoder)
bus: none (pure logic)
transport: "caller-supplied Pulse timings"
no_std: true
unsafe: forbidden
frame_pulses: 67
tolerance_us: 200
status: stable
---

# ir-nec

Pure-logic NEC IR-remote protocol codec. Takes a slice of pulse timings
and returns an `Option<NecCommand>` on the decode side; turns a
`NecCommand` into a 67-pulse frame on the encode side. No hardware
dependency — consumers capture or emit the pulses through whatever
peripheral they like (ESP32-S3 RMT peripheral on the CoreS3, a GPIO
ISR, a simulator).

## Key Files

- `src/lib.rs` — protocol constants, `Pulse`, `NecCommand`, `decode()`, `NecCommand::encode()`, round-trip + jitter tests

## Protocol

One NEC frame:

```
9.0 ms mark | 4.5 ms space | 32 data bits | 560 µs stop mark
```

Each data bit is a 560 µs mark followed by a space: short (560 µs) = `0`,
long (1.69 ms) = `1`. Data bits are transmitted LSB-first *within each
byte*. The four bytes on the wire are:

```
addr_low, addr_high, command, ~command
```

Receivers validate via `command ^ ~command == 0xFF`. This crate does
**not** validate the address checksum — extended-NEC variants carry
arbitrary data in the address bytes, and enforcing the classic
`addr / ~addr` rule would break them.

A complete frame is [`FRAME_PULSES`] = 67 pulses: 2 preamble + 64 data
(2 per bit × 32 bits) + 1 stop mark.

## API Summary

- **Decode:** `decode(pulses: &[Pulse]) -> Option<NecCommand>`. Takes any length ≥ 67, looks at the first 67 entries, ignores trailing pulses. Returns `None` on preamble / bit-space / checksum failure
- **Encode:** `NecCommand::encode(&self) -> [Pulse; 67]`. Computes `~command` for the caller
- **Timing tolerance:** `TOLERANCE_US = 200 µs` applied as a symmetric ± window around each spec duration. Wide enough for cheap receivers with ~10% jitter, narrow enough to reject ambient noise

## Gotchas

1. **Active-low IR receivers need inversion.** The crate treats `level = true` as "IR carrier on" (mark). Most IR receivers (including the CoreS3's IRM56384) output active-low — callers must invert before passing pulses to `decode()` and after receiving pulses from `encode()`
2. **LSB-first within bytes.** Bit 0 of `addr_low` hits the wire first. Getting this backwards gives bytes with reversed nibble order that still pass the "32 bits received" check but decode to the wrong command
3. **Stop mark duration isn't validated.** Some receivers clip the trailing 560 µs mark short; requiring it to be in tolerance would discard otherwise-valid frames. The decoder only checks that the stop *pulse exists and is a mark*
4. **Address checksum intentionally ignored.** Classic NEC uses `addr_high = ~addr_low`; extended NEC doesn't. The decoder accepts both and exposes `address` as a plain `u16`. If you need the classic validation, do it at your callsite
5. **67 pulses is a hard minimum for decode.** Shorter slices return `None` immediately; no partial-frame handling
6. **Encoder produces a fixed-size array.** `[Pulse; 67]` goes on the stack. Senders that prefer iterators can call `.iter()` on the return; this crate doesn't expose a streaming encoder because the frame is small enough that allocation-free array-building is simpler

## Integration

- **Firmware uses this for IR-RX** in `stackchan-firmware/src/ir.rs`, feeding pulses captured from the ESP32-S3 RMT peripheral
- **IR-TX** is newly scaffolded. Feed `NecCommand::encode()` output into an RMT TX buffer, convert each `Pulse { level, duration_us }` into the ESP32-S3 RMT's level / duration word format, and transmit
- **Host-testable.** All logic is pure functions; the crate's tests cover well-formed decode, bad checksum rejection, out-of-tolerance preamble, trailing-pulse tolerance, encode-decode round-trip, and encoder preamble shape
