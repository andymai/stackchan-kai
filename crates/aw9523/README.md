---
crate: aw9523
role: I/O expander driver + CoreS3 LCD bring-up
bus: I²C
address: "0x58"
transport: embedded-hal-async
no_std: true
unsafe: forbidden
status: stable
---

# aw9523

`no_std` async driver for the AW9523B I²C IO expander, with a
CoreS3-specific bring-up helper that pulses `LCD_RST` and gates the
backlight-boost converter rail. Without this dance the ILI9342C stays
in reset or — worse — runs with no backlight while the LCD shows a
confused image.

## Key Files

- `src/lib.rs` — `CORES3_ADDRESS`, port-0 / port-1 register constants, board-specific init values, `Error<E>`, `init_cores3` (the free function that runs the full bring-up), mock-I²C tests covering the reference sequence + reset-pulse invariants

## Bus + Addressing

- **I²C 7-bit address:** `0x58` (CoreS3 wiring is `AD1 = AD0 = GND`)
- **Transaction model:** simple register-write via `bus.write`, no reads — every register value is known at compile time
- **No driver struct** — the public surface is a single free `init_cores3` function; the chip has no state worth caching and consumers never need to poke it again after boot

## Register Map (CoreS3 Init)

| Reg    | Name            | Value      | Purpose                                                     |
|--------|-----------------|------------|-------------------------------------------------------------|
| `0x02` | `OUTPUT_P0`     | `0b0000_0111` | P0_0..2 HIGH (release LCD_RST, AW88298_RST, TP_RST)      |
| `0x03` | `OUTPUT_P1`     | `0b1000_1111` | P1_1 HIGH (LCD_RST released), P1_7 HIGH (backlight boost)|
| `0x04` | `DIR_P0`        | `0b0001_1000` | Bits 3+4 input, rest output                              |
| `0x05` | `DIR_P1`        | `0b0000_1100` | Bits 2+3 input (touch IRQ + tear-effect), rest output    |
| `0x11` | `CONTROL`       | `0x10`     | Bit 4 = push-pull on P0                                     |
| `0x12` | `LEDMODE_P0`    | `0xFF`     | All P0 pins in GPIO mode (not LED-sink)                     |
| `0x13` | `LEDMODE_P1`    | `0xFF`     | All P1 pins in GPIO mode                                    |

Then the reset pulse:

| Reg    | Value             | Action                                                        |
|--------|-------------------|---------------------------------------------------------------|
| `0x03` | `0b1000_0001`     | P1_1 LOW (assert LCD_RST), keep P1_7 HIGH (boost stays on)    |
| —      | `delay 20 ms`     | Reset-pulse width (≥10 µs datasheet, 20 ms matches M5Stack)   |
| `0x03` | `0b1000_1111`     | P1_1 HIGH (release reset), P1_7 HIGH                          |
| —      | `delay 120 ms`    | ILI9342C post-reset settle (datasheet minimum)                |

## Init Sequence

1. Configure port outputs + directions with known values *before* flipping to output
2. Set push-pull on P0, GPIO mode on both ports
3. Pulse `LCD_RST` on P1_1 while keeping `BACKLIGHT_BOOST_EN` on P1_7 held HIGH
4. Wait 120 ms for the ILI9342C internal init

AXP2101 rails MUST be up before this runs — touching the expander
before LDOs stabilize latches bad state from mid-rising rails.

## Gotchas

1. **Backlight-boost enable MUST stay HIGH during reset.** Bit 7 of `OUTPUT_P1` drives the boost converter — dropping it during the reset pulse leaves the panel dark after reset even though the LCD comes up. Test `lcd_reset_pulse_keeps_backlight_boost_enable_high` guards this
2. **Output values are written BEFORE direction flips.** This order guarantees each pin drives its final level the instant the direction register enables output — no glitch window between "direction = output" and "value = correct"
3. **No driver struct; just a function.** Consumers that hold the bus across multiple calls should import `init_cores3` directly and not try to wrap it
4. **120 ms post-reset is the ILI9342C's minimum** — cutting it short risks the panel rejecting the first SPI command. Test `post_reset_settle_meets_ili9342c_datasheet_minimum` guards this

## Integration

- **Runs after AXP2101 and before the LCD SPI init** in firmware boot. Any earlier and the rails aren't up; any later and the panel stays in reset
- **Called by `board::init` once** — the firmware holds no driver struct because there's nothing to call after boot (unless a future release adds dynamic IO-expander use for other CoreS3 pins)
- **Host-testable** via a mock I²C bus that records ordered events (writes + delays) — tests assert the full reference sequence, the reset-pulse keeps boost-enable high, and the timing meets datasheet minimums
