---
crate: si12t
role: Three-zone capacitive touch controller (scaffold)
bus: I²C
address: "0x50 (PROVISIONAL — confirm from M5Stack reference)"
transport: embedded-hal-async
no_std: true
unsafe: forbidden
status: scaffold-minimal
datasheet_access: proprietary
---

# si12t

Scaffold for an async I²C driver for the Si12T three-zone capacitive
touch controller on the M5Stack Stack-chan body — the touch strip on
the back of the head with left / centre / right pads.

## Status

**Minimal scaffold.** The Si12T's datasheet is not publicly available
and the chip's register surface needs to be extracted from M5Stack's
reference C++ firmware before this crate can do anything useful. The
current scaffold is a shell: the crate builds, an `Si12t` struct exists,
and `init()` / `read_touch()` return defaults so firmware board-init
can wire the crate in without a bare stub.

**Before this becomes a real driver:**

- Confirm the 7-bit I²C address (current `0x50` is a guess)
- Find the chip-ID / presence register
- Find the zone-status register + bit layout (which bit is which zone)
- Find the interrupt / threshold / sensitivity registers
- Find the reset / calibration sequence

Source to mine: [`m5stack/stackchan`](https://github.com/m5stack/stackchan)
C++ firmware, specifically `main/hal/board/` where M5Stack wires up the
body peripherals.

## Key Files

- `src/lib.rs` — module doc, placeholder constants, `Si12t` struct, `new` / `with_address`, `init` / `read_touch` stubs, `Touch` struct with `left` / `centre` / `right` booleans

## Bus + Addressing

- **I²C 7-bit address:** `0x50` provisional — **must be confirmed** from the reference firmware. The public M5Stack I²C-address index does not list this chip
- **Transaction model:** expected to be standard write-then-read for single-register access (consistent with FT6336U, LTR-553, etc.)
- **Interrupt pin:** likely routed through AW9523 on the CoreS3; confirm when the register map is extracted

## Register Map (unknown)

Nothing verified. Expected registers:

- Chip-ID / presence
- Zone-touch status (3 bits, one per zone)
- Per-zone sensitivity threshold
- Interrupt / data-ready flag
- Reset / calibration trigger

## Gotchas

1. **Don't rely on the provisional address.** `0x50` is a guess from "three-zone touch controllers in M5Stack's address range." Probing `[0x48..=0x58]` against the real hardware and checking which address ACKs is the fastest way to confirm
2. **Shared I²C bus.** The Si12T will share the main `SharedI2cBus` with FT6336U, BMI270, BMM150, AW88298, ES7210, BM8563, AW9523 — seven other addresses. Rule out address collisions before committing to an address constant
3. **Zone-to-pad mapping isn't physical intuition.** Don't assume `bit 0 = left`; different silicon orders zones by internal channel number. Verify against hardware (touch one zone at a time, log the bit pattern)
4. **Datasheet access is proprietary.** This crate may stay on "whatever M5Stack's reference does" for the foreseeable future. Cite the source in the code when you port the init sequence

## Integration

- **Will live in `stackchan-firmware`** as a `body_touch` module (distinct from `touch.rs`, which handles the front-screen FT6336U)
- **Shares the main `SharedI2cBus`** once wired up
- **Downstream:** feeds avatar-behavior input (tap left zone → look left, centre → neutral, right → look right)
- **Tests:** nothing meaningful until the register surface is known
