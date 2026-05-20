---
crate: stackchan-desktop-protocol
role: Wire protocol for Claude Desktop Hardware Buddy BLE bridge
bus: none
transport: "newline-delimited UTF-8 JSON over Nordic UART Service"
no_std: true (with alloc)
unsafe: forbidden
status: experimental (v0.x)
---

# stackchan-desktop-protocol

Wire-format crate for the Claude Desktop Hardware Buddy BLE bridge.
`no_std` + `alloc`. Pure data and parsers — no transport, no I/O.
The firmware wraps the Nordic UART Service GATT characteristics
around this crate; host tests exercise the parsers end-to-end with
fixtures captured from the reference desktop. The complete
field-by-field spec lives upstream:
<https://github.com/anthropics/claude-desktop-buddy/blob/main/REFERENCE.md>.

Newline-delimited UTF-8 JSON over the Nordic UART Service
(`6e400001-b5a3-f393-e0a9-e50e24dcca9e`), one object per line, in
both directions.

## What's here

| Module | What it does |
|---|---|
| `types.rs` | Message types: [`Inbound`], [`Snapshot`], [`Turn`], [`ContentBlock`], [`Prompt`], [`Cmd`], [`Outbound`], [`Decision`], [`Ack`], [`StatusData`], [`BatteryStatus`], [`SysStatus`], [`UserStats`]. Plus [`is_safe_relative_path`] — call on every `Cmd::File.path` before joining onto a writable root. |
| `parse.rs` | [`parse_inbound`] / [`parse_outbound`]. Strategy: parse each line into a small generic JSON value tree (one alloc), then dispatch on the top-level discriminator (`cmd` → `Cmd`, `evt` → `Turn`, `time` → `TimeSync`, otherwise `Snapshot`). |
| `build.rs` | [`render_outbound`] — `Outbound` → newline-terminated JSON line. Exhaustive — every field emitted is named in the spec. |
| `frame.rs` | [`LineFramer`] — accumulates a stream of byte slices (BLE notifications fragment at the MTU boundary) and yields one complete line at a time. |
| `error.rs` | [`ProtoError`] — typed errors via `thiserror`. Firmware wraps with `defmt::Debug2Format`. |

## What it speaks

### Inbound (desktop → device)

- **Snapshot** — session counts, recent transcript, token totals,
  battery / heap / uptime, and a pending [`Prompt`] when a
  permission request is waiting. Sent on change and at least every
  ~10 s as a keepalive.
- **Turn** — one-shot record of a completed assistant turn (`evt`
  envelope; carries the raw content array — text + tool calls).
- **TimeSync** — `{"time":[epoch_secs, tz_offset_secs]}`; sent once
  per connect.
- **Cmd** — `owner`, `name`, `status`, `unpair`, and the folder-push
  sequence (`char_begin`, `file`, `chunk`, `file_end`, `char_end`).

### Outbound (device → desktop)

- **Permission** — `{"cmd":"permission","id":"…","decision":"once"|"deny"}`.
- **Ack** — `{"ack":"<cmd>","ok":<bool>,"n":<u32>,"error":"…"}`.
- **Status ack** — same shape with a [`StatusData`] payload (name,
  sec, battery, uptime, heap, counters).

## API

```rust
use stackchan_desktop_protocol::{parse_inbound, Inbound, Outbound, Decision, render_outbound};

let line = br#"{"total":1,"running":0,"waiting":1,"msg":"approve: Bash"}"#;
let Inbound::Snapshot(snap) = parse_inbound(line)? else { unreachable!() };
assert_eq!(snap.waiting, 1);

let reply = render_outbound(&Outbound::Permission {
    id: "req_abc".into(),
    decision: Decision::Once,
});
assert!(reply.starts_with(r#"{"cmd":"permission","id":"req_abc","decision":"once"}"#));
```

## Hand-rolled JSON

Same rationale as [`stackchan_net::bare_json`](../stackchan-net/src/bare_json.rs):
avoid `serde` / `serde_json` so the firmware compiles cleanly on
`xtensa-esp32s3-none-elf` without pulling `serde/std`. Unknown keys
in inbound messages are **skipped**, not rejected — the desktop is
the protocol's authority and can add fields without bricking firmware
that pre-dates them. Outbound builders are exhaustive.

## Gotchas

1. **Always validate paths.** [`is_safe_relative_path`] rejects
   empty input, absolute paths, `..` / `.` segments, leading `~`,
   embedded NUL, backslash separators, and Windows drive prefixes.
   Call it on every `Cmd::File.path` before joining onto a writable
   root — the desktop sends whatever filenames appeared in the
   dropped folder.
2. **BLE notifications fragment at the MTU.** [`LineFramer::push`]
   accumulates partial chunks and yields complete lines into a
   caller-owned `Vec<Vec<u8>>`. Per-connection state; reset on
   disconnect via [`LineFramer::reset`].
3. **Unknown keys are intentional.** Don't add strict
   "extra-key-rejected" checks to the parser; future desktop
   versions add fields and the firmware must keep parsing.
4. **`alloc` is unconditional.** The parser alloc's a small JSON
   tree per line. The firmware target has PSRAM; host tests run
   trivially. Don't try to make this `no_alloc`.

## Integration

- Consumed by [`stackchan-firmware`'s BLE
  desktop session](../stackchan-firmware/src/ble/desktop.rs) and
  the four `desktop_*` consumer tasks (render, control, permission,
  time).
- The reference upstream
  [REFERENCE.md](https://github.com/anthropics/claude-desktop-buddy/blob/main/REFERENCE.md)
  is the source of truth for fields; this crate tracks it with
  per-message fixtures committed alongside the parser.
- **Stability:** Experimental in v0.x. Tracks the upstream
  reference protocol; field additions land as additive type
  changes.

[`Inbound`]: src/types.rs
[`Snapshot`]: src/types.rs
[`Turn`]: src/types.rs
[`ContentBlock`]: src/types.rs
[`Prompt`]: src/types.rs
[`Cmd`]: src/types.rs
[`Outbound`]: src/types.rs
[`Decision`]: src/types.rs
[`Ack`]: src/types.rs
[`StatusData`]: src/types.rs
[`BatteryStatus`]: src/types.rs
[`SysStatus`]: src/types.rs
[`UserStats`]: src/types.rs
[`is_safe_relative_path`]: src/types.rs
[`parse_inbound`]: src/parse.rs
[`parse_outbound`]: src/parse.rs
[`render_outbound`]: src/build.rs
[`LineFramer`]: src/frame.rs
[`LineFramer::push`]: src/frame.rs
[`LineFramer::reset`]: src/frame.rs
[`ProtoError`]: src/error.rs
