---
crate: si12t
role: Three-zone capacitive touch controller (back-of-head body touch)
bus: I²C
address: 0x50 (verified via just i2c-probe)
transport: embedded-hal-async
no_std: true
unsafe: forbidden
status: implemented
datasheet_access: proprietary — driver mirrors the M5Stack reference C++
---

# si12t

`no_std` async I²C driver for the Si12T three-zone capacitive touch
controller on the M5Stack Stack-chan body. The chip exposes three pads
(left / centre / right) on the back of the head and is polled by the
host at ~50 ms cadence.

## Source

Datasheet is proprietary. The driver mirrors the upstream M5Stack
reference C++ implementation:

- [`firmware/main/hal/drivers/Si12T/Si12T.h`](https://github.com/m5stack/StackChan/blob/main/firmware/main/hal/drivers/Si12T/Si12T.h)
- [`firmware/main/hal/drivers/Si12T/Si12T.cpp`](https://github.com/m5stack/StackChan/blob/main/firmware/main/hal/drivers/Si12T/Si12T.cpp)
- [`firmware/main/hal/hal_head_touch.cpp`](https://github.com/m5stack/StackChan/blob/main/firmware/main/hal/hal_head_touch.cpp)

## Bus + Addressing

- **I²C 7-bit address:** `0x50` — verified via `just i2c-probe` against
  Andy's CoreS3 + Stack-chan body. Upstream's macro
  `SI12T_GND_ADDRESS = 0x68` does not match this hardware; the
  probe is the source of truth. Override via
  [`Si12t::with_address`] if a different unit straps differently.
- **Polled at 50 ms** — there is no interrupt line. M5Stack's reference
  uses a FreeRTOS task with the same cadence.

## Register Map

| Register      | Addr   | Notes                                            |
|---------------|--------|--------------------------------------------------|
| `SENS1..SENS5`| `0x02..0x06` | Per-channel sensitivity. SENS6 (`0x07`) intentionally left at reset. |
| `CTRL1`       | `0x08` | Auto-mode + FTC config. Init writes `0x22`.      |
| `CTRL2`       | `0x09` | Reset / sleep gate. Init pulses `0x0F` then `0x07`. |
| `REF_RST1/2`  | `0x0A..0x0B` | Reference reset — zeroed at init.          |
| `CH_HOLD1/2`  | `0x0C..0x0D` | Channel hold — zeroed at init.             |
| `CAL_HOLD1/2` | `0x0E..0x0F` | Calibration hold — zeroed at init.         |
| `OUTPUT1/2/3` | `0x10..0x12` | Touch state; only `OUTPUT1` is used in practice. |

`OUTPUT1` packs all three zones into 2-bit fields:

| Bits  | Zone   | Field |
|-------|--------|-------|
| `0..1`| left   | `Intensity` |
| `2..3`| centre | `Intensity` |
| `4..5`| right  | `Intensity` |
| `6..7`| —      | reserved |

`Intensity` values: `0=None, 1=Low, 2=Mid, 3=High`. Upstream's UI
threshold for "touched" is intensity ≥ 1.

## Sensitivity

The `Sensitivity` byte encodes type + level:

- Lower nibble = level (0..7)
- Upper nibble adds `0x80` for `TYPE_HIGH`

Stack-chan default: `TYPE_LOW + LEVEL_3 → 0x33`. Exposed as
[`DEFAULT_SENSITIVITY`]; override at construction via
[`Si12t::with_sensitivity`].

## Init sequence

`init()` mirrors `si12t_setup()` upstream:

1. Burst-write zero into the 6-register reference / hold block (`REF_RST1` → `CAL_HOLD2`).
2. Pulse `CTRL2 = 0x0F` (reset), 1 ms settle, then `CTRL2 = 0x07` (sleep-disable).
3. Write `CTRL1 = 0x22` (auto-mode + FTC).
4. Burst-write the sensitivity byte to `SENS1..SENS5`.

The 1 ms settle between CTRL2 writes is a precaution on async
transports; upstream's blocking driver omits it.

## Integration

- **Verified on hardware** via `just si12t-bench` — flashes the bench
  binary, polls the chip at 50 ms, logs zone-state changes.
- **Firmware integration** (`body_touch.rs` task + `Input` field) is a
  follow-up PR.
- **Shares the main `SharedI2cBus`** with the other I²C peripherals
  on CoreS3.

## Gotchas

1. **No chip-ID register.** Probe presence by I²C ACK only — same
   pattern as ES7210 (already in memory).
2. **Address discrepancy.** Upstream's `SI12T_GND_ADDRESS = 0x68` is
   wrong for this hardware. Trust `just i2c-probe`.
3. **Datasheet proprietary.** The constants in this crate come from
   reading M5Stack's C++ — cite that source if you change them.
4. **Polled, not interrupted.** Don't waste time hunting an interrupt
   pin on the AW9523; there isn't one.
