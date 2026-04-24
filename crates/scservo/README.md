---
crate: scservo
role: Feetech SCServo (SCSCL / SCS0009) driver
bus: UART (half-duplex, 1 Mbaud)
transport: embedded-io-async
no_std: true
unsafe: forbidden
protocol: "Feetech packet (0xFF 0xFF ID len instr addr data... ~chk)"
status: stable
---

# scservo

`no_std` async driver for the Feetech SCServo family (SCSCL / SCS0009).
The servos share a half-duplex TTL bus at 1 Mbaud, each addressed by a
1-byte ID. The crate speaks the Feetech packet protocol over any
`embedded_io_async::Write` (for position / torque commands) and
optionally `Read` (for `ping`, the boot-time health check).

## Key Files

- `src/lib.rs` — protocol constants, `Instruction` enum, `Scservo` driver, `write_position` / `write_torque_enable` / `write_memory` / `read_memory` / `ping`, packet build / checksum / response parse helpers, golden-packet tests

## Bus + Wire Format

- **Transport:** half-duplex TTL UART at 1 Mbaud (default). Caller configures the baud
- **Packet:** `| 0xFF | 0xFF | ID | msgLen | Instruction | MemAddr | Data... | ~checksum |`
- **`msgLen`** = payload bytes after `msgLen` itself. For a write of N data bytes: `msgLen = N + 3` (instruction + addr + checksum). For `PING`: `msgLen = 2` and `MemAddr` is omitted
- **Checksum** = `~(ID + msgLen + Instruction + MemAddr + sum(Data))` — bitwise-NOT of the byte sum
- **16-bit values are big-endian** (SCSCL `End = 1`)
- **Broadcast ID `0xFE`** — every servo acts on the packet, no response. Reads / pings against broadcast return `Error::BroadcastNotAllowed`

## Instructions

| Instruction  | Opcode | Purpose                                                |
|--------------|--------|--------------------------------------------------------|
| `Ping`       | `0x01` | Query — servo responds with its status byte            |
| `Read`       | `0x02` | Read N bytes from a register                           |
| `Write`      | `0x03` | Write N bytes to a register                            |
| `RegWrite`   | `0x04` | Stage a deferred write                                 |
| `RegAction`  | `0x05` | Execute all pending `RegWrite`s                        |
| `SyncWrite`  | `0x83` | Broadcast write of identical-shape payload to many IDs |

## Memory Table

Register addresses the crate exposes as consts:

| Addr | Name                    | Size  | Purpose                                             |
|------|-------------------------|-------|-----------------------------------------------------|
| 40   | `ADDR_TORQUE_ENABLE`    | 1     | `0` = free, `1` = holding                           |
| 42   | `ADDR_GOAL_POSITION`    | 2+2+2 | Position (0..=1023), time (ms), speed — big-endian  |
| 56   | `ADDR_PRESENT_POSITION` | 2     | Current position in counts                          |
| 62   | `ADDR_PRESENT_VOLTAGE`  | 1     | 0.1 V per count                                     |
| 63   | `ADDR_PRESENT_TEMPERATURE` | 1  | °C (direct, no scaling)                             |
| 66   | `ADDR_MOVING`           | 1     | `0` = settled, non-zero = tracking                  |

## Position Space

0..=1023 counts across ~300° of travel, centered at 512.
`POSITION_CENTER = 512`, `POSITION_PER_DEGREE = 1023.0 / 300.0 ≈ 3.41`.
`write_position` rejects counts > 1023 with `Error::PositionOutOfRange`
— the servo would interpret the high bits as an address in the memory
table.

## Gotchas

1. **No internal timeout.** UART reads block until the full response arrives. A disconnected servo hangs forever unless the caller wraps `ping` / `read_memory` with their runtime's timeout primitive (`embassy_time::with_timeout(Duration::from_millis(10), bus.ping(1))` — 10 ms is generous, RTT at 1 Mbaud is < 200 µs)
2. **Big-endian 16-bit values.** Most embedded serial protocols are little-endian; Feetech isn't. The packet builder and response parser handle it
3. **Broadcast is for writes only.** `ping` and `read_memory` against `0xFE` garble the frame (every servo replies at once). The crate returns `Error::BroadcastNotAllowed` at the boundary
4. **`write_position` data payload is 6 bytes.** Position (2), time (2), speed (2) — commanding just the position without time / speed isn't supported by the memory layout
5. **Response parsing is strict.** Wrong headers → `MalformedResponse`. Bad checksum → `ChecksumMismatch`. Partial read → `NoResponse`. Callers log verbatim at defmt level
6. **`MAX_DATA_BYTES = 6`** caps the write payload. `WritePos` is the only instruction that reaches the limit; the bound exists so arbitrary-length writers added later stay bounded

## Integration

- **Firmware `head` module** drives pan + tilt via `write_position`. Values come from `stackchan_core::Pose` after the `EmotionHead` / `IdleDrift` / `IdleSway` modifier stack resolves
- **Calibration bench** (`crates/stackchan-firmware/examples/bench.rs`, invoked via `just bench`) sweeps each servo and reads back `ADDR_PRESENT_POSITION` to find mechanical limits
- **Host-testable** — packet encoding / decoding and checksum math are pure functions with golden-packet tests; response parsing has "malformed / checksum bad / truncated" coverage
