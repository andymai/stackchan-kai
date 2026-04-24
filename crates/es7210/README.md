---
crate: es7210
role: 4-channel audio ADC control driver (scaffold)
bus: I²C
address: "0x40 (strap: 0x40–0x43)"
transport: embedded-hal-async
audio: I2S / TDM (out of scope)
no_std: true
unsafe: forbidden
chip_id: "(0x72, 0x10) @ 0xFD/0xFE"
status: scaffold
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

## Register Map (planned)

Registers the scaffold will reach once fully implemented. Addresses are
datasheet-confirmed but the exact values the CoreS3 Stack-chan needs
have to be extracted from M5Unified's reference init.

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

## Init Sequence (planned)

1. `RESET = 0x3F`; wait ≥5 ms
2. `RESET = 0x00`; wait ≥5 ms
3. Read `CHIP_ID1/2`; verify `(0x72, 0x10)`
4. `CLK_ON = 0x3F` (enable analog / digital / MCLK clock gates)
5. `MAINCLK` + MCLK divisor — choose the divisor that maps the I2S `MCLK` pin (ESP32-S3 I2S peripheral provides 12.288 MHz on the CoreS3) to a 48 kHz-or-equivalent sample rate
6. `SAMPLE_RATE = 0x02` (48 kHz via default OSR)
7. `MIC_BIAS_CTRL = 0x44` (mic bias on for the two active channels)
8. `ADCx_CTRL` — PGA gain on channels 1 and 2 (0–30 dB in 0.5 dB steps)
9. `TDM_FORMAT` / `I2S_FORMAT` — match the ESP32-S3 I2S peripheral configuration
10. `ADC_ENABLE = 0x03` (enable channels 1–2 only — the only ones wired to mics)

Mute / unmute and MCLK ratio need to stay in lockstep with the matching
AW88298 playback side so both paths share a single `MCLK` domain.

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
