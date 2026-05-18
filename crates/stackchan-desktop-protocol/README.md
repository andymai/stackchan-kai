# stackchan-desktop-protocol

Wire protocol for the Claude Desktop Hardware Buddy BLE bridge.

`no_std` + `alloc`. Pure data and parsers — no transport, no I/O. The firmware
wraps the Nordic UART Service GATT characteristics around this crate; host
tests exercise the parsers end-to-end with fixtures captured from the
reference desktop.

## What it speaks

Claude for macOS and Windows can expose session state and permission prompts
over Bluetooth LE when developer mode is enabled. The wire format is
newline-delimited UTF-8 JSON, one object per line, over the Nordic UART
Service (`6e400001-b5a3-f393-e0a9-e50e24dcca9e`). See the [reference
protocol][ref] for the field-by-field spec.

[ref]: https://github.com/anthropics/claude-desktop-buddy/blob/main/REFERENCE.md

### Inbound (desktop → device)

- **Heartbeat snapshot** — session counts, recent transcript, token totals,
  and a pending permission prompt when one is waiting.
- **Turn event** — a one-shot record of a completed turn (assistant text,
  tool calls).
- **Time sync** — epoch seconds + timezone offset, sent once on connect.
- **Commands** — `owner`, `name`, `status`, `unpair`, and the folder-push
  sequence (`char_begin`, `file`, `chunk`, `file_end`, `char_end`).

### Outbound (device → desktop)

- **Permission decision** — `{"cmd":"permission","id":"...","decision":"once"|"deny"}`.
- **Ack** — `{"ack":"<cmd>","ok":<bool>,"n":<u32>,"error":"..."}`.
- **Status ack** — same shape with a `data` payload (name, sec, battery,
  uptime, heap, counters).

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
assert_eq!(reply.as_str(), r#"{"cmd":"permission","id":"req_abc","decision":"once"}"#);
```

[`LineFramer`] accumulates a stream of byte slices (BLE notifications fragment
at the MTU boundary) and yields one complete line at a time.

`is_safe_relative_path(&str) -> bool` rejects empty input, absolute paths,
`..` / `.` segments, leading `~`, embedded NUL, backslash separators, and
Windows drive prefixes. Call it on every `Cmd::File.path` before joining it
onto a writable root — the desktop sends whatever filenames appeared in the
dropped folder.

## Hand-rolled JSON

Same rationale as [`stackchan_net::bare_json`]: avoid `serde` / `serde_json` so
the firmware compiles cleanly on `xtensa-esp32s3-none-elf` without pulling
`serde/std`. Unknown keys in inbound messages are **skipped**, not rejected —
the desktop is the protocol's authority and can add fields without bricking
firmware that pre-dates them. Outbound builders are exhaustive (every field
emitted is named in the spec).

[`LineFramer`]: ./src/frame.rs
[`stackchan_net::bare_json`]: ../stackchan-net/src/bare_json.rs
