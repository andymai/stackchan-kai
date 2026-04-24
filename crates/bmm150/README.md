---
crate: bmm150
role: 3-axis geomagnetic sensor driver
bus: I²C
address: "0x10 / 0x11"
transport: embedded-hal-async
no_std: true
unsafe: forbidden
chip_id: "0x32 @ register 0x40"
status: stable
---

# bmm150

`no_std` async driver for the Bosch BMM150 3-axis geomagnetic sensor.
Covers the boot dance (soft-reset → wake → trim-register readout →
regular 10 Hz preset), compensated microtesla readings, and nothing
else. Heading computation, tilt compensation, and hard-/soft-iron
calibration are intentionally left to downstream consumers.

## Key Files

- `src/lib.rs` — `ADDRESS_PRIMARY` / `SECONDARY`, register constants, `Bmm150` struct, `detect` / `new` / `init` / `read_measurement`, `Trim` + Bosch-reference compensation port, `Measurement` struct (µT output), tests

## Bus + Addressing

- **I²C 7-bit address:** `0x10` (SDO → GND) or `0x11` (SDO → VDDIO). `Bmm150::detect` probes both
- **Transaction model:** `write_read` for single registers; one 8-byte burst for the measurement block; one 21-byte burst for the trim registers (includes reserved bytes that are read + discarded)
- **CHIP_ID:** `0x32` at register `0x40`. Mismatch → `Error::ChipId { expected, actual }`

## Register Map

| Reg         | Name            | Access | Purpose                                                  |
|-------------|-----------------|--------|----------------------------------------------------------|
| `0x40`      | `CHIP_ID`       | R      | Identity byte; expected `0x32`                           |
| `0x42–0x49` | `DATA`          | R      | 8-byte burst: `X_L, X_M, Y_L, Y_M, Z_L, Z_M, RHALL_L, RHALL_M` |
| `0x4B`      | `POWER`         | W      | Bit 0 = wake; `0x82` = soft-reset while suspended        |
| `0x4C`      | `OPMODE_ODR`    | W      | ODR (bits 5:3), op-mode (bits 2:1), self-test (bit 0)    |
| `0x51`      | `REP_XY`        | W      | XY repetitions = `2 * reg + 1`                           |
| `0x52`      | `REP_Z`         | W      | Z repetitions = `reg + 1` (asymmetric — not `2 * reg + 1`) |
| `0x5D–0x71` | `TRIM`          | R      | 21-byte per-chip compensation constants                  |

## Init Sequence

1. Read `CHIP_ID`; expect `0x32`
2. Soft-reset: `POWER = 0x82` (bit 7 | bit 1, stays in suspend); wait ≥5 ms
3. Wake: `POWER = 0x01` (bit 0 set); wait ≥5 ms (I²C NACKs against a still-suspended chip)
4. Read the 21-byte trim block into a `Trim` struct; cache for every future `read_measurement`
5. Regular preset: `REP_XY = 4` (REPXY = 9), `REP_Z = 14` (REPZ = 15)
6. `OPMODE_ODR = 0x00` — 10 Hz normal mode

## Compensation

Raw ADC counts don't mean anything without per-chip trim — every BMM150
ships with factory calibration stored in the trim registers. The driver
ports Bosch's reference compensation (same fixed-point integer math,
same overflow sentinels) and converts the 1/16 µT per LSB output to
`f32` µT at the public boundary. Total earth-field magnitude is
25–65 µT at the surface; readings well outside that window indicate
hard-iron distortion (nearby magnets / motors / speaker).

## Gotchas

1. **Suspend mode is I²C-unreachable.** Issuing any register read against a chip that hasn't been woken NACKs. The `POWER = 0x01` wake step is non-negotiable
2. **`REP_Z` encoding is different from `REP_XY`.** XY uses `(n-1) / 2`, Z uses `n - 1`. One of the few asymmetries in the register map
3. **Trim registers include four reserved bytes.** The 21-byte block spans `0x5D..=0x71`; the driver reads all 21 and extracts only the Bosch-defined trim fields from it. Don't assume the block is contiguous trim
4. **Overflow is a separate error variant.** `X_OVERFLOW = -4096` (13-bit field), `Z_OVERFLOW = -16384` (15-bit field). The driver surfaces these as `Error::Overflow` rather than returning nonsense µT; the firmware's `mag` module logs them at `debug` since brief overflows on a spinning magnetometer are normal
5. **Output unit is `f32` µT, not raw counts.** Callers that want the raw 1/16-µT LSBs need to reach into the crate privately or accept the conversion cost
6. **`detect()` costs up to one extra NACK.** Similar to `bmi270` — the firmware uses it anyway to log which address answered

## Integration

- **Used by `stackchan-firmware/src/mag.rs`** as the other half of the 9-axis data path, paired with `bmi270` on the same `SharedI2cBus`
- **Overflow logging:** the firmware's mag task logs `Error::Overflow` at `defmt::debug` — the avatar's gaze-steering is fine with brief gaps in the stream
- **Host-testable** — the compensation math is pure and unit-testable against Bosch's reference vectors; the register-access surface is mockable via `embedded-hal-async` fakes
