---
crate: bm8563
role: Real-time clock driver
bus: I²C
address: "0x51"
transport: embedded-hal-async
no_std: true
unsafe: forbidden
status: stable
---

# bm8563

`no_std` async driver for the NXP BM8563 real-time clock (a
pin-compatible clone of the PCF8563). Scope is tight: set / read
date-time and detect the voltage-low flag. Alarm, timer, and CLKOUT
features are deliberately out of scope.

## Key Files

- `src/lib.rs` — `ADDRESS`, register constants, `DateTime` struct (year / month / day / weekday / hours / minutes / seconds), `Bm8563` driver, `init` / `read_datetime` / `write_datetime`, pure `encode_datetime` / `decode_datetime` / `format_datetime` / BCD helpers, round-trip + century-bit + saturation tests

## Bus + Addressing

- **I²C 7-bit address:** fixed at `0x51`
- **Transaction model:** single-register writes for `Control_1` / `Control_2` init; one 7-byte `write_read` burst for the date-time block; one 8-byte burst-write (register address + 7 payload) to set the time
- **No chip-ID register** — the BM8563 has no identity byte; presence is inferred from I²C ACKs

## Register Map

| Reg    | Name                   | Purpose                                                              |
|--------|------------------------|----------------------------------------------------------------------|
| `0x00` | `Control_1`            | Top bit `STOP`: `0` = RTC running, `1` = stopped                     |
| `0x01` | `Control_2`            | Alarm / timer interrupt flags (zeroed at init)                       |
| `0x02` | `VL_SECONDS`           | Bit 7 = `VL` voltage-low flag, bits 6:0 = BCD seconds                |
| `0x03` | `MINUTES`              | Bits 6:0 BCD                                                         |
| `0x04` | `HOURS`                | Bits 5:0 BCD                                                         |
| `0x05` | `DAYS`                 | Bits 5:0 BCD                                                         |
| `0x06` | `WEEKDAYS`             | Bits 2:0; 0 = Sunday convention                                      |
| `0x07` | `CENTURY_MONTHS`       | Bit 7 = century (1 → 1900s, 0 → 2000s), bits 4:0 BCD month           |
| `0x08` | `YEARS`                | Two-digit BCD year; combined with `CENTURY_MONTHS` for 1900..=2099   |

## `DateTime`

Plain struct, no timezone — the RTC is timezone-agnostic. Year is
expanded to four digits using the century bit; the rest are plain
decimals inside their natural ranges (hour `0..=23`, month `1..=12`,
weekday `0..=6` Sunday-based).

## Init Sequence

1. `Control_1 = 0x00` — clear STOP, RTC starts ticking
2. `Control_2 = 0x00` — disable lingering alarm / timer interrupts

That's it. `init` does NOT set the time; a freshly-powered RTC has
arbitrary values and a set `VL` flag. Set the time explicitly via
`write_datetime`.

## Gotchas

1. **`VL` flag surfaces as a typed error.** `read_datetime` returns `Error::VoltageLow` if bit 7 of the seconds register is set — callers can retry, treat it as unset-time, or display the raw value anyway
2. **BCD masks differ per register.** Seconds uses `0x7F` (strip VL); hours uses `0x3F`; days uses `0x3F`; month uses `0x1F` (leave century bit alone). Hard-coded in `decode_datetime`
3. **Century encoding is inverted from intuition.** Bit 7 SET = 1900s, CLEAR = 2000s. The driver hides this by producing a 4-digit `year` on decode and reconstructing the bit on encode
4. **`format_datetime` saturates pathological inputs** — a garbled register read producing `month = 99` becomes `"99"` in the output, so the function always produces exactly 19 ASCII bytes. Tests cover the pathological case
5. **No `alloc`, no `chrono`.** The crate intentionally avoids both: `chrono` pulls in tons of date arithmetic; callers that need it can convert `DateTime` at their boundary

## Integration

- **Firmware `wallclock` module** sets the RTC from Wi-Fi NTP (eventually) or manual boot-time settings, and reads it for log timestamps + status line
- **Host-testable** — `encode_datetime`, `decode_datetime`, `format_datetime`, and `bcd_to_u8`/`u8_to_bcd` are all pure functions with round-trip tests
