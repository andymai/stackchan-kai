---
crate: st25r3916
role: NFC reader / writer SPI transport (scaffold)
bus: SPI
spi_mode: "CPOL=0, CPHA=0, MSB-first, 8-bit"
spi_clock_max_mhz: 6
spi_clock_default_mhz: 1.5
transport: embedded-hal-async
ic_type_id: "0b01010 @ IC_IDENTITY (0x3F), low 5 bits"
no_std: true
unsafe: forbidden
status: scaffold
---

# st25r3916

Scaffold for an async SPI driver for the STMicroelectronics ST25R3916 —
a multi-protocol NFC reader / writer. On the CoreS3 Stack-chan the IC
provides tag detection and card I/O through its integrated analog
front-end.

## Key Files

- `src/lib.rs` — module doc, SPI framing constants, `St25r3916` struct, `read_register` / `write_register` / `direct_command`, `init` stub (`SET_DEFAULT` + identity probe)

## Bus + Framing

- **SPI mode:** CPOL = 0, CPHA = 0, MSB-first, 8-bit words
- **Clock:** up to 6 MHz; ST's X-CUBE-NFC3 defaults to 1.5 MHz
- **Every transaction** starts with a 2-bit mode prefix in the first byte:

| Prefix (b7..b6) | Mode             | First-byte layout                      |
|-----------------|------------------|----------------------------------------|
| `00`            | Register write   | `00 aaaaaa` + data byte                |
| `01`            | Register read    | `01 aaaaaa`, response on next beat     |
| `10`            | FIFO read        | `10 xxxxxx`, N response bytes          |
| `11`            | Direct command   | `11 oooooo` (single byte)              |

- **Two address spaces.** Space A is `0x00..=0x3F` (direct-addressed); space B needs a `SPACE_B_ACCESS` direct command preamble. The scaffold only exposes space A
- **Chip identity:** `IC_IDENTITY` at register `0x3F`. Low 5 bits = IC type (`0b01010` for ST25R3916), upper 3 bits = silicon revision. Always check only the low 5 bits

## Register Map (planned)

Registers the scaffold will touch. Addresses are ST-datasheet-confirmed.
A full driver covers ~90 registers across spaces A and B; this is the
startup / sanity subset.

| Reg              | Addr   | Access | Purpose                                              |
|------------------|--------|--------|------------------------------------------------------|
| `IO_CONF1`       | `0x00` | W      | SPI vs I²C select, MISO pull, IRQ routing            |
| `IO_CONF2`       | `0x01` | W      | Supply voltage scaling, VSPD regulator enable        |
| `OP_CONTROL`     | `0x02` | W      | RX / TX enable, wake-up, oscillator                  |
| `MODE`           | `0x03` | W      | Operating mode: ISO14443-A/B, NFC-A/F, ISO15693, … |
| `BIT_RATE`       | `0x04` | W      | TX / RX bit rate (106 – 848 kbps for A, variable B)  |
| `ISO14443A_NFC`  | `0x05` | W      | ISO 14443-A protocol timing / framing                |
| `AM_MOD_DEPTH`   | `0x29` | W      | Amplitude modulation depth (transmit tuning)         |
| `RX_CONF1..4`    | `0x0A–0x0D` | W | Receiver gain, digitizer, filter, AGC                |
| `TX_DRIVER`      | `0x28` | W      | TX driver output stage configuration                 |
| `IC_IDENTITY`    | `0x3F` | R      | `xxx_01010` — IC type + silicon revision             |
| `MAIN_IRQ`       | `0x1A` | R      | Main interrupt register; read to clear              |

Direct commands the scaffold already exposes or will need:

| Command        | Opcode | Purpose                                             |
|----------------|--------|-----------------------------------------------------|
| `SET_DEFAULT`  | `0xC2` | Reset all registers to datasheet defaults           |
| `CLEAR_FIFO`   | `0xC3` | Drop RX + TX FIFO contents                          |
| `TRANSMIT_X`   | `0xC4–0xCD` | Various transmit-frame commands                |
| `ANALOG_PRESET`| `0xC1` | Apply analog-front-end preset for current `MODE`    |
| `MASK_RECEIVE_DATA` | `0xD1` | Suppress RX until frame boundary               |

## Init Sequence (planned)

1. `SET_DEFAULT` direct command; wait ≥500 µs
2. Read `IC_IDENTITY`; verify low 5 bits = `0b01010`
3. `IO_CONF1/2` — SPI interface mode, MISO pull-up, regulator enable
4. `OP_CONTROL` — enable oscillator, wait for stable oscillator interrupt
5. `MODE` — pick the protocol (most Stack-chan uses will want ISO 14443-A)
6. `ANALOG_PRESET` direct command — apply datasheet front-end values for the chosen mode
7. Antenna tuning — `AAT` (automatic antenna tuning) direct command or manual `CVDAC*` register sweep, depending on matching-network tolerance
8. `OP_CONTROL` — enable RX / TX

## Gotchas

1. **Mode prefix is easy to miss.** Every SPI first-byte carries a 2-bit mode in bits 7:6. A raw register address (no prefix) gets interpreted as a register write with an unexpected opcode
2. **`IC_IDENTITY` is an IC-type field, not a full ID.** Mask with `0x1F` before comparing; the upper 3 bits change between silicon revisions
3. **Two address spaces.** Space B registers (`0x40+`) require the `SPACE_B_ACCESS` direct command preamble. Don't assume `read_register(0x40)` reaches the space-B register at address 0
4. **Oscillator startup is async.** After `OP_CONTROL` enables the oscillator, the chip raises a `OSC_STABLE` interrupt some milliseconds later. Configuration issued before that interrupt can latch inconsistent analog values
5. **Antenna tuning is board-specific.** The matching network on the CoreS3 Stack-chan dictates `CVDAC*` values; use AAT to discover them and persist to flash rather than hardcoding
6. **RFAL exists for a reason.** The vendor's RF Abstraction Layer encapsulates ~15 k lines of protocol-level logic. This crate intentionally stops at the transport layer — protocol work should live in a separate crate that consumes this one

## Integration

- **Will live in `stackchan-firmware`** as an `nfc` module on a dedicated SPI peripheral (SPI3 on the CoreS3; SPI2 is the LCD)
- **IRQ pin** wires to a GPIO — the eventual driver will expose a `wait_for_irq()` that pairs with an embassy `Signal`
- **Higher-level protocol** (ISO 14443, etc.) lands in a separate crate or as a firmware-only module; this crate is the transport
- **Host-testable** via mock SPI — register framing and direct-command encoding are pure byte sequences
