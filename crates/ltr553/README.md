---
crate: ltr553
role: Ambient-light + proximity sensor driver
bus: I²C
address: "0x23"
transport: embedded-hal-async
no_std: true
unsafe: forbidden
part_id_upper: "0x9 @ register 0x86"
status: stable
---

# ltr553

`no_std` async driver for the Lite-On LTR-553ALS ambient-light +
proximity sensor. Minimal surface: one `init()`, plus `read_ambient`
and `read_proximity` polling accessors. Configures the chip for default
gain + integration so the lux math matches Lite-On's app-note piecewise
formula at unit scale — other gain / integration settings need a more
complete driver.

## Key Files

- `src/lib.rs` — `ADDRESS`, register constants, `AmbientReading` struct (`ch0` + `ch1` raw + computed `lux`), `Ltr553` driver, `init` / `read_part_id` / `read_ambient` / `read_proximity` / `read_status`, private `lux_from_channels` piecewise formula, unit tests

## Bus + Addressing

- **I²C 7-bit address:** fixed at `0x23` on the CoreS3 (no strap pin)
- **Transaction model:** single-register writes for config; 4-byte burst read for the ambient data pair; 2-byte burst for the proximity pair
- **Part ID:** upper nibble `0x9` identifies the LTR-553 family; lower nibble is silicon revision

## Register Map

| Reg    | Name            | Access | Purpose                                                   |
|--------|-----------------|--------|-----------------------------------------------------------|
| `0x80` | `ALS_CONTR`     | W      | ALS mode + gain; driver writes `0x02` (active, 1× gain)   |
| `0x81` | `PS_CONTR`      | W      | PS mode + gain; driver writes `0x03` (active)             |
| `0x86` | `PART_ID`       | R      | Upper nibble `0x9` for LTR-553 family                     |
| `0x88–0x8B` | `ALS_DATA`  | R      | 4-byte burst: `CH1_LSB, CH1_MSB, CH0_LSB, CH0_MSB`        |
| `0x8C` | `ALS_PS_STATUS` | R      | Bit 2 = ALS data ready, bit 0 = PS data ready             |
| `0x8D–0x8E` | `PS_DATA`   | R      | 2-byte burst: 11-bit proximity + saturation flag          |

## Init Sequence

1. Read `PART_ID`; verify upper nibble is `0x9`
2. `ALS_CONTR = 0x02` — ALS active, gain 1× (default)
3. `PS_CONTR = 0x03` — PS active, default gain 16×

Default ALS config: gain 1×, integration 100 ms, measurement rate 500 ms.

## Reading

- **`read_ambient()`** — returns `AmbientReading { ch0, ch1, lux }`. `ch0` is visible + IR, `ch1` is IR-only. Lux is a piecewise formula that returns `0.0` when the scene is IR-only (e.g. incandescent behind filters)
- **`read_proximity()`** — returns an 11-bit raw count, larger = closer. Bit 7 of the MSB is a saturation flag the driver ignores

## Gotchas

1. **Datasheet byte order is `CH1_LSB, CH1_MSB, CH0_LSB, CH0_MSB`** — NOT `CH0` first. Easy to get backwards; the driver's `read_ambient` decodes it correctly
2. **Lux formula is gain-dependent.** `lux_from_channels` assumes 1× gain / 100 ms integration (what `init` sets). Changing either side breaks the formula — a more general driver would need a lookup or scale
3. **Proximity is raw counts, not mm.** "Larger = closer" but there's no linear distance mapping without calibration against known reflectance
4. **Proximity saturation is silent.** Bit 15 of the MSB signals that the IR LED output saturated the photodiode. The driver masks it with `0x07` rather than reporting it — callers that care can read the status register
5. **`init()` gates on PART_ID.** A wrong chip at `0x23` fails with `Error::BadPartId`; a bus NACK fails with `Error::I2c`

## Integration

- **Firmware `ambient` module** polls ambient lux + proximity. Lux drives the `EmotionFromAmbient` avatar modifier (darkroom → sleepy face); proximity will drive a "someone is close" reaction once the integration is done
- **Shares the main `SharedI2cBus`**
- **Host-testable** — the lux formula and byte-order handling are both testable via mock I²C
