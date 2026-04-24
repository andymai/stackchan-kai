---
crate: py32
role: Stack-chan PY32 co-processor driver (GPIO + WS2812 fan-out)
bus: I²C
address: "0x6F"
transport: embedded-hal-async
no_std: true
unsafe: forbidden
max_leds: 32
status: stable
---

# py32

`no_std` async driver for the Stack-chan CoreS3's **PY32 co-processor**.
Despite the name, this chip is *not* a stock GPIO expander — it's a
Puya PY32-family MCU running custom M5Stack firmware that exposes GPIO
config *and* a WS2812 fan-out buffer on the pin wired to the 12-LED
ring. This crate implements the subset our firmware needs: servo-power
rail gating (pin 0) and LED frame staging + latch.

## Key Files

- `src/lib.rs` — `ADDRESS`, `MAX_LEDS`, GPIO register map, LED RAM / config map, `Py32` driver, `configure_output_pin` (RMW on dir / pullup / out), `set_led_count` / `write_led_pixels` / `refresh_leds`, unit tests

## Bus + Addressing

- **I²C 7-bit address:** `0x6F` (Stack-chan CoreS3 wiring)
- **Transaction model:** single-register writes for GPIO config; read-modify-write for all direction / pullup / level changes so other pins stay put; bulk writes for LED RAM (register address + pixel bytes in one transaction)
- **No chip-ID register** — the PY32 firmware doesn't publish one; presence is inferred from I²C ACKs
- **Firmware provenance:** register layout + LED-RAM semantics lifted from [m5stack/StackChan `PY32IOExpander_Class.cpp`](https://github.com/m5stack/stackchan) (MIT upstream)

## Register Map

| Reg    | Name          | Purpose                                                          |
|--------|---------------|------------------------------------------------------------------|
| `0x03` | `DIR_LO`      | Direction bits for pins 0..7 (1 = output)                        |
| `0x04` | `DIR_HI`      | Direction bits for pins 8..13                                    |
| `0x05` | `OUT_LO`      | Output level for pins 0..7 (1 = HIGH)                            |
| `0x06` | `OUT_HI`      | Output level for pins 8..13                                      |
| `0x09` | `PULLUP_LO`   | Pull-up enable for pins 0..7 (1 = enabled)                       |
| `0x0A` | `PULLUP_HI`   | Pull-up enable for pins 8..13                                    |
| `0x24` | `LED_CFG`     | Bits 0..5 = LED count (max 32); bit 6 = refresh-trigger          |
| `0x30+` | `LED_RAM`    | 2 bytes per pixel (LE RGB565), auto-increment; pixel `i` at `0x30 + 2i` |

## Capabilities

- **GPIO output config** — `configure_output_pin(pin, level)`:
  1. Sets direction bit via RMW
  2. Enables pull-up via RMW
  3. Writes the level bit
  Used to raise the servo-power rail on pin 0.
- **LED frame:**
  1. `set_led_count(n)` — bits 0..5 of `LED_CFG` (preserves bit 6 so an in-flight refresh isn't dropped)
  2. `write_led_pixels(&[u16])` — stages up to 32 RGB565 pixels in LED RAM (LE byte order)
  3. `refresh_leds()` — sets bit 6 of `LED_CFG`; the PY32 firmware clocks the entire RAM onto the WS2812 chain on pin 13
- **`release()`** — gives the bus back to the caller (useful when the PY32 is touched once at boot and the bus belongs to someone else afterward)

## Gotchas

1. **RMW everywhere.** Every GPIO operation reads the current register, masks, writes. Interleaving calls on different pins is safe; calling the driver while another task also touches `0x6F` is NOT — the driver assumes exclusive access
2. **LED refresh is a two-step dance.** `write_led_pixels` stages a frame in RAM without touching the chain; `refresh_leds` latches it. Skipping the refresh leaves the displayed frame unchanged — and a torn frame isn't possible because the chain doesn't update until the trigger bit is set
3. **Empty frame is a wire no-op.** `write_led_pixels(&[])` skips the I²C transaction entirely — no blank frame gets staged. Callers that want to clear the ring need to write explicit all-zero pixels
4. **Pin range is 0..=13.** Out-of-range pins return `Error::InvalidPin(pin)` rather than silently dropping. Only these pin indices exist in the firmware's register layout
5. **LED count ≤ 32.** The LED count field is 6 bits (max 63) but the RAM only holds 32 pixels; the driver rejects `count > 32` with `Error::TooManyLeds`
6. **RGB565 wire order is little-endian.** Pixel `0xFFE0` (yellowish white) becomes `[0xE0, 0xFF]` in the I²C payload — matches what the PY32 firmware expects, NOT the byte order most LED libraries use

## Integration

- **Firmware `board::init`** raises the servo-power rail via `configure_output_pin(0, true)` as part of boot
- **Firmware `leds` module** owns the WS2812 fan-out: renders a frame via `stackchan_core::render_leds`, writes it with `write_led_pixels`, latches with `refresh_leds`
- **Shares the main `SharedI2cBus`**
- **Bench recipe:** `just leds-bench` exercises the LED path end-to-end on hardware
