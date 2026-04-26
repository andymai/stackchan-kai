---
crate: ft6336u
role: Capacitive touch controller driver (single-touch subset)
bus: I²C
address: "0x38"
transport: embedded-hal-async
no_std: true
unsafe: forbidden
vendor_id: "0x11 @ register 0xA8"
status: stable
---

# ft6336u

`no_std` async driver for the FocalTech FT6336U capacitive touch
controller. MVP single-touch subset — one `read_touch()` returns finger
count + first touch coordinates. Multi-touch, native gesture registers,
and power-saving modes are intentionally out of scope; adding them
doesn't change the `Ft6336u` type surface.

## Key Files

- `src/lib.rs` — `CORES3_ADDRESS`, `VENDOR_ID_FOCALTECH`, register + offset constants, `TouchReport` (fingers + first `(x, y)`), `Ft6336u` driver, `read_vendor_id` / `read_touch` / `into_inner`, pure `decode_touch` + unit tests

## Bus + Addressing

- **I²C 7-bit address:** fixed at `0x38` on the CoreS3
- **Transaction model:** one 7-byte `write_read` burst starting at `G_MODE` (`0x00`) covers status + first touch-point in a single transaction
- **Vendor ID:** `0x11` at register `0xA8` — cheap presence check at boot
- **Reset:** external, driven by AW9523 during LCD bring-up. The driver assumes the chip is out of reset by the time `new()` is called

## Register Map

Only the registers this driver touches.

| Offset | Reg         | Contents                                          |
|--------|-------------|---------------------------------------------------|
| 0      | `G_MODE`    | Operating mode (unused, but the burst starts here) |
| 1      | `GESTURE`   | HW-detected gesture code (unused)                 |
| 2      | `TD_STATUS` | Low nibble = touch count (0..=2)                  |
| 3      | `P1_XH`     | `[7:6]` event flag, `[3:0]` x\[11:8\]             |
| 4      | `P1_XL`     | x\[7:0\]                                          |
| 5      | `P1_YH`     | `[7:4]` touch id, `[3:0]` y\[11:8\]               |
| 6      | `P1_YL`     | y\[7:0\]                                          |

Plus `0xA8` (vendor ID, read-only).

## Usage

No `init()` method — the chip's reset defaults work for this MVP.
Construct with `Ft6336u::new(bus)`, optionally probe with
`read_vendor_id()`, then call `read_touch()` on whatever cadence the
caller wants (chip caps at 60 Hz internal polling).

`TouchReport::point()` and `TouchReport::is_touched()` are convenience
accessors for the common "is a finger down, and where?" question.

## Gotchas

1. **Coordinate masking strips metadata.** Top 2 bits of `P1_XH` carry an event flag (press / release / contact / no-event) that the MVP ignores; top nibble of `P1_YH` carries the touch ID. Both are masked out by `COORD_HIGH_MASK = 0x0F`
2. **`TD_STATUS` high nibble is reserved.** Don't read it as "many fingers" — masked with `0x0F` in `decode_touch`. Test `td_status_high_nibble_is_ignored` guards this
3. **Unpowered chip reads as all-ones.** A chip in reset or with no power returns `0xFF` bytes, which decode to `fingers = 15`. Callers can filter as a sanity check; the crate doesn't do it automatically
4. **Every read hits the bus.** No caching — the chip's ≤60 Hz internal polling is the rate-limit, not any client-side throttle
5. **No init or chip-ID probe in `new()`.** `read_vendor_id()` exists for explicit boot-time checks; construction is infallible and doesn't touch the bus
6. **Native coordinate space is 12-bit.** Panel orientation may require callers to swap / invert axes to match the framebuffer — the driver doesn't know which orientation the panel is mounted in

## Integration

- **Firmware `touch` module** polls via `read_touch()` and feeds results to the avatar's `EmotionFromTouch` modifier (tap → emotion change)
- **Shares the main `SharedI2cBus`** with AXP2101, AW9523, BMI270, BMM150, BM8563, and the upcoming audio codecs
- **Host-testable** — `decode_touch` is a pure `[u8; 7] → TouchReport` function with coverage for zero-touch, one-touch, two-touch, and reserved-bit edge cases
