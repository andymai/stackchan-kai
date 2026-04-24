---
crate: bmi270
role: 6-axis IMU driver (accel + gyro)
bus: I²C
address: "0x68 / 0x69"
transport: embedded-hal-async
no_std: true
unsafe: forbidden
chip_id: "0x24"
---

# bmi270

Minimal async driver for the Bosch BMI270 6-axis IMU. Covers the boot dance,
unit-converted readings, and nothing else — motion detection, pedometer, tap,
and no-motion are intentionally out of scope (reach for Bosch's `SensorAPI` if
you need them).

## Key Files

- `src/lib.rs` — public API, register constants, `Bmi270` struct, `init()` sequence, `read_measurement()`, and the pure-function `decode_measurement()` (host-testable)
- `src/config_blob.rs` — the mandatory 8192-byte Bosch firmware blob, uploaded to the chip on every cold boot. Not re-exported outside the crate

## Bus + Addressing

- **I²C 7-bit address:** `0x68` (SDO → GND) or `0x69` (SDO → VDDIO). `Bmi270::detect()` probes both; `Bmi270::new()` with a fixed address saves one NACK round-trip on boards with known wiring
- **Transaction model:** `write_read` for single-register reads and the 12-byte data block; `write` for single-register writes and the blob burst
- **CHIP_ID:** `0x24` at register `0x00`. Mismatch surfaces as `Error::BadChipId(byte)`
- **Delay source:** caller provides a `DelayNs` impl per `init()` call — the driver holds no clock of its own

## Register Map

Only registers the driver actually touches.

| Reg              | Addr        | Access | Purpose                                                            |
|------------------|-------------|--------|--------------------------------------------------------------------|
| `CHIP_ID`        | `0x00`      | R      | Identity byte; expected `0x24`                                     |
| `DATA`           | `0x0C–0x17` | R      | 12-byte burst: ACC_X/Y/Z + GYR_X/Y/Z (i16 little-endian each)      |
| `INTERNAL_STATUS`| `0x21`      | R      | Init-complete flag; low nibble `0x01` = ok                         |
| `ACC_CONF`       | `0x40`      | W      | ODR + BWP + perf mode; driver writes `0xA8` (100 Hz, performance)  |
| `ACC_RANGE`      | `0x41`      | W      | `0x01` = ±4 g                                                      |
| `GYR_CONF`       | `0x42`      | W      | ODR + BWP + perf; driver writes `0xE9` (200 Hz, performance)       |
| `GYR_RANGE`      | `0x43`      | W      | `0x01` = ±1000 dps                                                 |
| `INIT_CTRL`      | `0x59`      | W      | `0x00` before blob upload, `0x01` after the last byte lands        |
| `INIT_ADDR_0/1`  | `0x5B/0x5C` | W      | 12-bit word offset for the next blob chunk (low 4 / upper 8 bits)  |
| `INIT_DATA`      | `0x5E`      | W      | Burst-write target for the config blob                             |
| `PWR_CONF`       | `0x7C`      | W      | Bit 0 = advanced power save. MUST be `0` during blob upload        |
| `PWR_CTRL`       | `0x7D`      | W      | `0x0E` = enable accel + gyro + temp (driver leaves aux disabled)   |
| `CMD`            | `0x7E`      | W      | `0xB6` = soft-reset                                                |

## Init Sequence

Non-negotiable order — the chip refuses to clock its ADCs if any step is
skipped or reordered.

1. Read `CHIP_ID`; expect `0x24`
2. `CMD = 0xB6` (soft-reset); wait 1 ms
3. `PWR_CONF = 0x00` (disable advanced power save); wait 1 ms
4. `INIT_CTRL = 0x00`
5. For each 128-byte chunk of the 8192-byte blob: write `INIT_ADDR_0` (low 4 bits of word offset), `INIT_ADDR_1` (upper 8 bits), then burst-write the chunk to `INIT_DATA`
6. `INIT_CTRL = 0x01`; poll `INTERNAL_STATUS` every 20 ms for up to 15 attempts (~300 ms) until `(status & 0x0F) == 0x01`
7. Configure `ACC_CONF`, `ACC_RANGE`, `GYR_CONF`, `GYR_RANGE`, then `PWR_CTRL = 0x0E`

Step 6 is the only point where `InitTimeout` can fire. Bosch's reference
allows ~150 ms; the 300 ms budget is generous for slow I²C buses.

## Gotchas

1. **The config blob is mandatory.** Without the 8192-byte upload, the chip refuses to produce sensor data. `init()` will hit `Error::InitTimeout` at step 6 rather than fail earlier — there's no cheap "is the blob loaded" probe before that point
2. **`INIT_ADDR` is indexed in 16-bit words, not bytes.** Low 4 bits → `INIT_ADDR_0`, upper 8 bits → `INIT_ADDR_1`. A byte-addressed upload silently corrupts the blob and fails step 6
3. **`BLOB_CHUNK_BYTES` must divide 8192.** `128` is chosen to fit esp-hal's 32-byte hardware I²C FIFO (which chunks transparently) and most other masters; tuning it requires keeping it a power-of-two divisor, asserted by `debug_assert!`
4. **Accel and gyro run at different ODRs** (100 Hz vs 200 Hz) but share the 12-byte burst read. The gyro's internal filter delivers the most-recent sample on every poll, so polling at the accel rate is fine
5. **Gyro scale is fixed at ±1000 dps.** Other ranges would require editing both `GYR_RANGE_VALUE` and `GYR_LSB_TO_DPS` in lockstep — the LSB scaling is hard-coded to match the range byte
6. **Advanced power save must stay off** during init. The driver leaves `PWR_CONF = 0` during normal operation too, for deterministic timing at the cost of a bit more idle draw
7. **`detect()` costs up to one extra NACK round-trip** (it tries `0x68`, falls back to `0x69`). On a board with fixed wiring, `Bmi270::new(bus, address)` skips the probe; the CoreS3 firmware in this repo still uses `detect()` so it logs which address the chip answered on

## Integration

- **Runtime-agnostic.** Caller supplies the I²C bus and a `DelayNs`; firmware passes `esp-hal`'s async I²C master + `embassy-time::Delay`, tests use mocks
- **Used by `stackchan-firmware`** as half of the 9-axis data path — paired with `crates/bmm150` (magnetometer) on the same I²C bus
- **Host-testable.** `decode_measurement()` is a pure function; the crate's unit tests exercise sign extension, ±4 g scaling, ±1000 dps conversion, and the blob-length invariant without a real bus
