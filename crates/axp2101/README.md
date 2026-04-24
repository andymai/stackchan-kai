---
crate: axp2101
role: PMIC driver + CoreS3 bring-up
bus: I²C
address: "0x34"
transport: embedded-hal-async
no_std: true
unsafe: forbidden
status: stable
---

# axp2101

`no_std` async driver for the X-Powers AXP2101 power-management IC on
the M5Stack CoreS3. Scope is the minimum needed to boot the LCD rails
*and* configure the chip not to auto-shutdown on idle, plus one-shot
power-key IRQ helpers for the hardware button. Battery-state readout
and charging config are left for future releases.

## Key Files

- `src/lib.rs` — `ADDRESS`, register + bit constants, `CORES3_INIT_SEQUENCE` (the full `M5Unified` register sequence as a `[(reg, value)]` slice), `Axp2101` struct, `init_cores3`, `read_reg` / `write_reg`, power-key IRQ helpers, mock-I²C golden-sequence tests

## Bus + Addressing

- **I²C 7-bit address:** fixed at `0x34` on the CoreS3
- **Transaction model:** single-register reads + writes (`write_read` / `write`)
- **No chip-ID probe** — the driver doesn't expose one; `init_cores3` is fire-and-forget, and a missing / wrong PMIC fails at the first I²C transaction

## CoreS3 Init Sequence

Thirteen writes in order, copied verbatim from `M5Unified`'s
`Power_Class.cpp`. Applied by `init_cores3()` in one shot.

| Reg    | Value  | Purpose                                                    |
|--------|--------|------------------------------------------------------------|
| `0x92` | `13`   | ALDO1 = 1.8 V — AW88298 audio codec                        |
| `0x93` | `28`   | ALDO2 = 3.3 V — ES7210 audio ADC                           |
| `0x94` | `28`   | ALDO3 = 3.3 V — camera                                     |
| `0x95` | `28`   | ALDO4 = 3.3 V — TF card slot                               |
| `0x96` | `28`   | BLDO1 = 3.3 V — LCD backlight (voltage BEFORE enable)      |
| `0x97` | `28`   | BLDO2 = 3.3 V — LCD logic                                  |
| `0x90` | `0xBF` | Enable ALDO1..4 + BLDO1..2                                 |
| `0x27` | `0x00` | Power-key timing: 1 s hold to wake, 4 s to power off       |
| `0x10` | `0x30` | PMU common config — internal off-discharge enable          |
| `0x12` | `0x00` | BATFET disable (prevents undervoltage shutdown, no battery)|
| `0x68` | `0x01` | Battery-detect enable                                      |
| `0x69` | `0x13` | CHGLED on charger, flashing on charge                      |
| `0x99` | `28`   | DLDO1 = 3.3 V — LCD backlight brightness                   |
| `0x30` | `0x0F` | ADC block enable (battery / VBUS voltage reads)            |

## Power-Key IRQ

Two helpers for the CoreS3 hardware button:

- `enable_power_key_short_press_irq()` — `IRQ_EN_1` bit 4 set, preserves other bits via RMW
- `check_short_press_edge()` — reads `IRQ_STATUS_1` (`0x49`), returns `true` + write-1-clears the bit, or `false` if no edge

Bit layout of `IRQ_STATUS_1`: bit 0 = key release, bit 1 = key press,
bit 4 = short-press (< 1 s), bit 5 = long-press, bit 6 = over-press (> 2 s).

## Gotchas

1. **Voltage setpoints MUST precede the enable bitmap.** BLDO1 default is 0.5 V — enabling it before writing `0x96 = 28` lights the panel with no backlight. The test `init_cores3_writes_backlight_voltage_before_enable_bitmap` guards this
2. **BATFET disable is load-bearing when no battery is attached.** Leaving it enabled routes the chip through the battery FET; with no battery, that path triggers an undervoltage shutdown
3. **Power-key timing (`0x27`)** must be written or the chip treats mild button glitches as shutdown requests. Default values are aggressive
4. **ADC must be enabled (`0x30 = 0x0F`)** before any battery / VBUS voltage read returns meaningful data
5. **`into_inner()` releases the bus.** Useful for single-task firmware bringing up AW9523 after the PMIC — avoids pulling in a shared-bus abstraction

## Integration

- **Runs first in firmware boot**, before `aw9523::init_cores3` or the LCD SPI init. Rails must be up before any downstream peripheral is touched
- **Shares the main `SharedI2cBus`** once embassy's shared-bus wrapper is installed
- **Host-testable** via mock I²C — tests assert the init sequence byte-for-byte, verify ordering invariants, and confirm battery-detect + ADC are in the sequence
