---
crate: aw88298
role: Smart-K audio amplifier control driver (scaffold)
bus: I²C
address: "0x36 (strap: 0x34–0x37)"
transport: embedded-hal-async
audio: I2S 16-bit (out of scope)
reset_pin: AW9523 P0_1 (external)
no_std: true
unsafe: forbidden
chip_id: "0x1852 @ register 0x00"
status: scaffold
---

# aw88298

Scaffold for an async control-path driver for the Awinic AW88298 —
a 16-bit I2S "smart K" class-D amplifier with an integrated 10.25 V
smart boost converter. On the CoreS3 Stack-chan it drives the single
1 W speaker. The I²C side handles configuration; audio data arrives
over I2S from the ESP32-S3 peripheral.

## Key Files

- `src/lib.rs` — module doc, address / chip-ID constants, `Aw88298` struct, `new` / `with_address`, `read_chip_id`, `init` stub, `set_muted` placeholder

## Bus + Addressing

- **I²C 7-bit address:** `0x36` default (CoreS3 strap). The chip's base is `0b01101xx`; `AD1` + `AD2` straps sweep `0x34–0x37`
- **Transaction model:** single-register reads / writes; `CHIPID` is a 2-byte big-endian read starting at `0x00`
- **CHIP_ID:** `0x1852`
- **External reset:** `RST` is wired to AW9523 `P0_1`. The `aw9523` crate's CoreS3 bring-up helper releases it as part of board init — the amp NACKs every I²C transaction until then
- **Audio transport:** I2S (Philips / left-justified / PCM), 16-bit mono. Typically 48 kHz; the chip will lock to whatever the MCU I2S master clocks in

## Register Map (planned)

Registers the scaffold will reach once implemented. Exact bit layouts
come from the public Awinic datasheet.

| Reg         | Addr   | Access | Purpose                                              |
|-------------|--------|--------|------------------------------------------------------|
| `CHIPID`    | `0x00` | R      | Two-byte chip ID, big-endian. Expected `0x1852`      |
| `SYSCTRL`   | `0x04` | W      | Bits 0/1/2 = `PWDN` / `AMPPD` (mute) / `I2SEN`       |
| `SYSST`     | `0x01` | R      | Status: I2S lock, PLL lock, OC / OT / UVLO flags     |
| `I2SCTRL1`  | `0x06` | W      | I2S format (Philips / LJ / PCM), BCLK polarity       |
| `I2SCTRL2`  | `0x07` | W      | Sample rate selection                                |
| `I2SCTRL3`  | `0x08` | W      | Slot width, MSB/LSB first                            |
| `BSTCTRL1`  | `0x60` | W      | Boost target voltage (8.0 V – 10.25 V)               |
| `BSTCTRL2`  | `0x61` | W      | Boost current limit, mode (adaptive vs fixed)        |
| `HAGCCFG1`  | `0x0A` | W      | Hearing-aid-style AGC limiter                        |
| `PWMCTRL`   | `0x51` | W      | PWM carrier frequency                                |
| `VOLCTRL`   | `0x0C` | W      | Volume / fade target (0 dB default)                  |

## Init Sequence (planned)

1. AW9523 releases `P0_1` (external `RST`); wait ≥1 ms
2. Read `CHIPID`; expect `0x1852`
3. `SYSCTRL = 0x00` (clear `PWDN` + `AMPPD`; enable I2S when ready)
4. Configure `I2SCTRL1/2/3` for 16-bit Philips I2S at the MCU's sample rate (48 kHz on the CoreS3 default)
5. `BSTCTRL1` = target boost voltage (8.0 V is the M5Stack default — enough for 1 W @ 4 Ω without clip)
6. `HAGCCFG1` + related — enable thermal / OC protection with safe limits
7. `SYSCTRL |= I2SEN` — amp starts passing audio
8. `VOLCTRL` ramps volume to target (use the fade curve rather than a direct jump to avoid pops)

Unmute (`AMPPD = 0`) is the final step so the speaker only drives while
`MCLK`, `BCLK`, `LRCK`, and `DATA` are all stable.

## Gotchas

1. **External reset is not optional.** The amp NACKs until AW9523 releases `P0_1`. Any test harness must release reset before I²C traffic
2. **Boost voltage affects SOA.** Setting `BSTCTRL1` above 10.25 V exceeds the part's rated range; setting it below 8 V clips loud audio. 8 V is the safe default for Stack-chan's speaker
3. **Unmute last.** Sequence `BCLK/LRCK stable → I2SEN → AMPPD = 0` or the amp thumps; ramping via `VOLCTRL` is the clean alternative
4. **Thermal throttle is aggressive.** `HAGCCFG` limiting is on by default; disabling it without heatsinking risks the amp shutting down under sustained load
5. **CHIPID is big-endian on the wire.** The driver reads two bytes starting at `0x00` and calls `u16::from_be_bytes`. A little-endian read gives `0x5218` which won't match any valid ID
6. **Mute via `AMPPD`, not `PWDN`.** `PWDN = 1` fully powers down the chip — recovering takes the full init sequence. `AMPPD = 1` just silences the output stage and keeps registers configured

## Integration

- **Will live in `stackchan-firmware`** as a `speaker` module, sharing the main `SharedI2cBus` with ES7210 for a matched audio path
- **Paired with [`es7210`](../es7210)** — the two always come up together (mic in + speaker out) and share the I2S `MCLK` domain
- **Depends on [`aw9523`](../aw9523)** for external reset release before init
- **Host-testable on the control surface** via mock I²C; audio output verification needs hardware
