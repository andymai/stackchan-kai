---
crate: gc0308
role: VGA CMOS camera control driver (scaffold)
bus: I²C (SCCB)
address: "0x21"
transport: embedded-hal-async
video: DVP parallel (out of scope, handled by LCD_CAM)
no_std: true
unsafe: forbidden
chip_id: "0x9B @ register 0xF0"
status: scaffold
---

# gc0308

Scaffold for an async control-path driver for the GalaxyCore GC0308 —
the 0.3 MP VGA camera sensor on the CoreS3 Stack-chan. The driver
manages register access over the SCCB-compatible I²C bus; the parallel
DVP video stream is the MCU's problem (ESP32-S3 LCD_CAM peripheral in
the firmware).

## Key Files

- `src/lib.rs` — module doc, constants, `Gc0308` struct, `new`, `read_chip_id`, `init` stub

## Bus + Addressing

- **I²C 7-bit address:** fixed at `0x21`
- **Transaction model:** SCCB is I²C-compatible; the driver uses plain `write_read` / `write` via `embedded-hal-async`
- **CHIP_ID:** `0x9B` at register `0xF0`
- **Video transport:** DVP 8-bit parallel (PCLK, HSYNC, VSYNC, D0–D7). Not touched by this crate — wire it up via the ESP32-S3 LCD_CAM DMA peripheral

## Register Map (planned)

Registers the scaffold will touch once fully implemented.

| Reg               | Addr   | Access | Purpose                                            |
|-------------------|--------|--------|----------------------------------------------------|
| `CHIP_ID`         | `0xF0` | R      | Identity byte; expected `0x9B`                     |
| `PAGE_SELECT`     | `0xFE` | W      | `0x00` = sensor page, `0x01` = ISP page            |
| `RESET`           | `0xFE` | W      | Bit 7 = soft-reset (on page 0)                     |
| `OUTPUT_WINDOW_*` | `0x17–0x1A` | W | Col/row start + width/height                       |
| `OUTPUT_FORMAT`   | `0x24` | W      | `0xA6` = RGB565, `0xA2` = YCbCr 4:2:2 (UYVY)       |
| `STREAM_EN`       | `0x25` | W      | Output enable (non-zero) / high-impedance (`0x00`) |

Addresses above come from the public GalaxyCore datasheet. A full
register list is page-selected — the sensor keeps most "hard" registers
on page 0 and ISP / YCbCr formatting on page 1. The driver scaffold
will need both.

## Init Sequence (planned)

1. Release `PWDN` (external GPIO, not the driver's concern); wait ≥50 ms
2. Read `CHIP_ID`; expect `0x9B`
3. Soft-reset via the reserved page register; wait 2 ms
4. Configure the output window (default 640×480 starting at (0, 0))
5. Set output format (`RGB565` for embedded-graphics consumers, `YCbCr` for JPEG encode)
6. Configure PCLK / HSYNC / VSYNC polarity and drive strength
7. `STREAM_EN` non-zero; pixels begin clocking on DVP

Steps 3–7 are the "long init" that M5Stack's reference firmware runs —
expect ~200 register writes. Store the sequence as `const [(u8, u8)]`
and loop.

## Gotchas

1. **Page selection is sticky.** `REG_PAGE_SELECT` (`0xFE`) routes subsequent reads / writes; the scaffold must re-select page 0 before reading `CHIP_ID` if a prior caller left page 1 active
2. **No dedicated SCCB lines on CoreS3.** Control I²C shares the main `SharedI2cBus` with the PMU, touch, and sensors. Keep transactions short so the camera doesn't starve the avatar render loop
3. **PWDN is external.** The driver does not control power; the firmware board-init must raise PWDN via AW9523 / AXP2101 before `init()` runs
4. **DVP timing is the bottleneck.** VGA @ 30 fps is ~9.2 MB/s uncompressed — the ESP32-S3 LCD_CAM DMA must be set up before streaming, and the framebuffer sink (PSRAM or tile-wise processor) must keep up
5. **Register `0xFE` is overloaded.** Same address triggers page-select *and* soft-reset depending on the page context — reset is on page 0, page-select works on any page. This is a datasheet pitfall; read the full procedure before tuning

## Integration

- **Will live in `stackchan-firmware`** as a `camera` module, wired to the shared I²C bus for control and the LCD_CAM peripheral for video
- **Downstream consumers:** vision/behavior modules (face detection, motion-trigger), JPEG/MJPEG streaming if / when Wi-Fi lands
- **Not host-testable on the DVP side.** The I²C control path can be mock-tested; the pixel stream requires a real sensor or a trace-replayer
