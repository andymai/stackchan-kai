---
crate: es7210
role: 4-channel audio ADC control driver
bus: I²C
address: "0x40 (strap: 0x40–0x43)"
transport: embedded-hal-async
audio: "I2S slave, 12.288 MHz MCLK, 16 kHz / 16-bit mono (fixed)"
active_channels: mic1 + mic2 (mic3/4 powered off)
no_std: true
unsafe: forbidden
chip_id: "(0x72, 0x10) @ 0xFD/0xFE"
status: experimental
---

# es7210

Scaffold for an async control-path driver for the Everest ES7210 —
the four-channel audio ADC on the CoreS3 Stack-chan. Two of the four
channels feed the on-board microphones; the other two are unused. Audio
data leaves the chip over I2S/TDM; this crate only handles I²C config.

## Key Files

- `src/lib.rs` — module doc, address / chip-ID constants, `Es7210` struct, `new` / `with_address`, `read_chip_id`, `init` stub (soft-reset + chip-ID check only)

## Bus + Addressing

- **I²C 7-bit address:** `0x40` default (CoreS3 strap). Moves to `0x41–0x43` with AD pin variants — use [`Es7210::with_address`] on non-standard boards
- **Transaction model:** single-register write-then-read; no burst reads for the control surface
- **CHIP_ID:** `(0x72, 0x10)` at `(0xFD, 0xFE)`. Both bytes must match
- **Audio transport:** I2S or TDM, chip-as-master or chip-as-slave. Configured via registers the scaffold doesn't write yet

## Register Map

Registers the driver touches at init. Values are a direct port of
[espressif/esp-adf][esp-adf]'s canonical sequence, simplified for our
fixed audio shape (12.288 MHz MCLK → 16 kHz, mic1+2 only).

[esp-adf]: https://github.com/espressif/esp-adf

| Reg               | Addr        | Access | Purpose                                              |
|-------------------|-------------|--------|------------------------------------------------------|
| `RESET`           | `0x00`      | W      | `0x3F` = soft-reset, `0x00` = release                |
| `CLK_ON`          | `0x01`      | W      | Enable internal clock domains                        |
| `MAINCLK`         | `0x02`      | W      | MCLK divisor + ADC clock source                      |
| `SAMPLE_RATE`     | `0x06`      | W      | Oversample ratio → sample rate (e.g. `0x02` = 48 kHz) |
| `ADC1/2/3/4_CTRL` | `0x43–0x46` | W      | Per-channel PGA gain                                 |
| `MIC_BIAS_CTRL`   | `0x11`      | W      | `0x44` = mic bias on for channels 1–2                |
| `ADC_ENABLE`      | `0x4B`      | W      | Per-channel ADC enable mask                          |
| `TDM_FORMAT`      | `0x11–0x12` | W      | TDM slot width, frame format, I2S vs PCM             |
| `CHIP_ID1/2`      | `0xFD/0xFE` | R      | `0x72 / 0x10`                                        |

## Init Sequence

Matches `Es7210::init`:

1. Read `CHIP_ID1/2`; verify `(0x72, 0x10)`
2. `RESET = 0x71` (assert soft-reset); wait ≥5 ms
3. `RESET = 0x41` (release); wait ≥5 ms
4. Clock tree for 12.288 MHz → 16 kHz:
   - `MAINCLK = 0xC3` (adc_div=3, doubler on, DLL on)
   - `OSR = 0x20` (256× oversample)
   - `LRCK_DIVH = 0x03`, `LRCK_DIVL = 0x00`
5. `CLOCK_OFF = 0x30` (mic1+2 clocks on, mic3+4 gated)
6. `POWER_DOWN = 0x00` (all blocks powered on)
7. `ANALOG = 0x43` (datasheet active-mode preset)
8. `MIC1/2/3/4_POWER = 0x08` (individual powers on)
9. `MIC12_POWER = 0x00` (group powered), `MIC34_POWER = 0xFF` (gated)
10. `MIC1/2_GAIN = 0x1A` (gain-enable bit + step `0x0A` ≈ +30 dB)
11. `ANALOG = 0x43` (re-asserted, per the reference)
12. `RESET = 0x71` / `0x41` (latch reset pulse)

Gain is re-programmable at runtime via [`Es7210::set_gain`].

MCLK / sample-rate must match the MCU's I²S master configuration. The
matching AW88298 playback side (`crates/aw88298`) should share the
same `MCLK` domain.

## Gotchas

1. **Two-byte chip ID.** Both `CHIP_ID1` and `CHIP_ID2` must match (`0x72`, `0x10`) — checking only one leaves the door open for a different Everest part that shares one half of the ID
2. **Mic bias powers the mics.** Skip `MIC_BIAS_CTRL` and the mics are silent even with the ADC enabled. The bias is analog, not PDM — watch for DC offset on the preamp
3. **MCLK must be stable before the first sample.** ESP32-S3 I2S starts MCLK when the I2S peripheral is started; configure ES7210 *after* I2S `MCLK` is running, or ADC output drifts
4. **TDM slot 0 vs I2S.** The chip supports both; the scaffold doesn't yet pick. Stack-chan's downstream audio pipeline (speech recognition or passthrough) dictates which — keep this decision coherent with the AW88298 side
5. **Mic count is 2, not 4.** The ES7210 is a 4-channel ADC; only channels 1–2 are wired. Writes to channels 3–4 register mask bits are harmless but waste bus time

## Integration

- **Will live in `stackchan-firmware`** as a `mic` module, sharing the main `SharedI2cBus` with AW88298 for a matched audio path
- **Paired with [`aw88298`](../aw88298)** — the two always come up together (mic in + speaker out) and share the I2S `MCLK` domain
- **Host-testable on the control surface** via mock I²C; audio-stream verification needs hardware or a codec simulator
