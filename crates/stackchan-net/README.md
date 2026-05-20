---
crate: stackchan-net
role: Networking domain types, wire formats, and on-disk config schema
bus: none
transport: "pure data + parsers"
no_std: true
unsafe: forbidden
status: experimental (v0.x)
---

# stackchan-net

Networking domain types for Stack-chan. Pure data and parsers — no
transport, no I/O, no `esp-hal`. The firmware does the I/O wrapping;
this crate is what the firmware (and host tests) agree on as the
shape of every wire format the avatar speaks.

Anything that has a serializer, validator, or parser the firmware and
the host both need to agree on belongs here. That includes the on-disk
RON config, the JSON bodies of the HTTP control plane, the MCP server
transport, the BLE control characteristics, the BluFi provisioning
frames, the ESP-NOW frame envelope, the OTA image header, the
crash-latch byte layout, and the mDNS pose-TXT formatting. Holding
all of it in a host-testable crate keeps the wire surface pinned by
`cargo test`, rather than living inside the firmware's
`xtensa-esp32s3-none-elf`-only `#[cfg(test)]` modules that CI never
runs.

## On-disk config schema

```ron
(
    wifi:    ( ssid: "home", psk: "redacted", country: "US" ),
    mdns:    ( hostname: "stackchan" ),
    time:    ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
    auth:    ( token: "" ),
    audio:   ( volume_pct: 50, muted: false ),
    tracker: ( fov_h_deg: 62.0, fov_v_deg: 49.0,
               target_smoothing_alpha: 1.0,
               flip_x: false, flip_y: false ),
    esp_now: ( enabled: false, pmk_hex: "", peer_mac: "",
               lmk_hex: "", channel: None, tx_rate_hz: 5 ),
    behavior: (
        soliloquy_enabled: false,
        hourly_chime_enabled: false,
        battery_icon_enabled: false,
        toast_overlay_enabled: false,
        auto_torque_release_ms: 0,
        audio_debug_udp_target: "",
        agent_sidecar_url: "",
        agent_sidecar_token: "",
        follower_leader_hostname: "",
        wake_word_enabled: false,
        wake_word_threshold: 100,
        wake_word_arena_kib: 64,
    ),
)
```

Top-level blocks land via `serde(default)` — a `STACKCHAN.RON` that
omits a block keeps the firmware's compile-time defaults. Per-field
documentation, validators, and the rationale behind each default live
in [`config.rs`](src/config.rs).

[`validate`] is the lenient gate (run on every `PUT /settings` so a
dashboard form re-submitting a redacted body still works);
[`validate_for_disk`] is `validate` plus rejection of the literal
redaction sentinel `"***"` in any secret field, run on the SD-load
path so an operator can't accidentally persist a redacted snapshot
back to disk.

## Offline-first stance

The avatar must boot fully and animate with no SD card and no Wi-Fi.
The firmware therefore treats [`Config`] as **always available**:
missing SD or missing file falls back to [`Config::default`].
Validators reject malformed input, but the firmware never propagates
a [`ConfigError`] up to a panic — it logs and uses defaults.

## Wire formats

| Module | What it pins |
|---|---|
| [`config`](src/config.rs)             | Top-level [`Config`], sub-block structs, validators, [`tz_offset_minutes`], [`parse_mac`] |
| [`bare`](src/bare.rs)                 | Hand-rolled RON parser / renderer used by the firmware. The `parse` feature gates the serde + `ron` path; the firmware target leaves it off |
| [`bare_json`](src/bare_json.rs)       | Hand-rolled JSON parser / renderer for the HTTP control plane's `GET` / `PUT /settings`, plus [`merge_settings_with_current`] which substitutes the persisted value back in wherever the redaction sentinel `"***"` arrives |
| [`http_command`](src/http_command.rs) | JSON body parsers for `POST /emotion`, `/look-at`, `/speak`, `/volume`, `/mute`, `/enter_pairing` |
| [`http_parse`](src/http_parse.rs)     | Byte-level helpers (`parse_content_length`, `parse_bearer_token`, `ct_eq`, …) for the firmware's HTTP request parser |
| [`mcp`](src/mcp.rs)                   | Minimal Model Context Protocol server: JSON-RPC 2.0 envelope, MCP `initialize` / `tools/list` / `tools/call`, the tool catalogue used by `POST /mcp` |
| [`ble_command`](src/ble_command.rs)   | Fixed-length codecs for the BLE audio + avatar-control characteristics; stable byte mappings for `Emotion`, `PhraseId`, `Locale` |
| [`blufi`](src/blufi.rs)               | BluFi frame parsing + building for BLE-based Wi-Fi provisioning from the official Espressif provisioning apps |
| [`esp_now`](src/esp_now.rs)           | ESP-NOW frame envelope (`STKC` magic, version, kind byte, JSON body); body is byte-identical to the corresponding HTTP route so the same payloads work over both transports |
| [`ota`](src/ota.rs)                   | OTA image header (`SCFW` magic, version, length, Ed25519 signature trailer) and host-testable image parser. The on-device verification + partition swap ride on `esp-hal-ota` in the firmware |
| [`crash_latch`](src/crash_latch.rs)   | Encoder / decoder for the firmware's persistent crash latch (lives in RTC fast RAM across resets; decoded on next boot and written to `/sd/CRASH.LOG`) |
| [`mdns_pose`](src/mdns_pose.rs)       | Throttling decision + TXT key formatting for the mDNS responder's live `yaw=` / `pitch=` advertisement. The embassy task itself lives in firmware; the pure logic stays here for the host CI test path |
| [`dance`](src/dance.rs)               | JSON parser for `POST /dance`; the [`DanceScript`] schema and the player modifier live in `stackchan-core` so the player stays driver-agnostic |
| [`error`](src/error.rs)               | [`ConfigError`] — parse, serialize, and validation variants |

`tests/golden_config.rs` + `tests/fixtures/*.ron` cover round-trip
and validation behaviour against hand-written fixtures.

## Why hand-rolled JSON / RON

`ron 0.10` (and the JSON crates) hard-pin `serde/std + base64/std`,
both unbuildable on `xtensa-esp32s3-none-elf`. The `parse` feature
gates the serde-backed `parse_ron` / `render_ron` path so host builds
(sim, tests, host-side tooling) get the convenient round-trip while
the firmware uses a hand-rolled bare parser. The hand-rolled parsers
match the serde derives' shape; round-trip tests in
`tests/golden_config.rs` pin them together.

`bare_json.rs` and `mcp.rs` exist for the same reason — JSON over
the wire couldn't pull `serde_json` either.

## Security notes

- [`render_ron`] is **lossless** — secret fields round-trip
  verbatim so SD reads and writes stay symmetric. Any caller that
  returns rendered output over an unauthed network channel must
  redact on the read path. The firmware's `GET /settings` does this
  via a separate render variant.
- The redaction sentinel `"***"` is the cross-cutting wire marker
  for "preserve the persisted value." It applies to `wifi.psk`,
  `auth.token`, `esp_now.{pmk_hex,lmk_hex}`, and
  `behavior.agent_sidecar_token`. The HTTP path's
  [`merge_settings_with_current`] substitutes the prior value back
  in; [`validate_for_disk`] rejects the sentinel as a disk-loaded
  value so an operator can't `cp` a redacted dump back to SD and
  silently brick auth.
- `behavior.agent_sidecar_token` is additionally length-capped and
  ASCII-control-rejected so a bad value can't extend the HTTP
  request header buffer or inject extra headers via embedded
  `\r\n`.

## Defer-list

- TLS / HTTPS. Today's control plane is LAN-scoped; bearer auth via
  `auth.token` is the only authentication mechanism.
- A captive-portal / soft-AP first-boot flow. BluFi covers the BLE
  side of provisioning; a soft-AP fallback hasn't landed.
- Persona / character data. Belongs to the agent tier and the
  sidecar, not the firmware-side config schema.
- BLE bonding-key persistence. Owned by the firmware
  `crate::ble::bonds` module; the on-disk format is firmware-target
  RTC RAM rather than something host code needs to parse.

[`Config`]: src/config.rs
[`Config::default`]: src/config.rs
[`ConfigError`]: src/error.rs
[`validate`]: src/config.rs
[`validate_for_disk`]: src/config.rs
[`tz_offset_minutes`]: src/config.rs
[`parse_mac`]: src/config.rs
[`render_ron`]: src/config.rs
[`merge_settings_with_current`]: src/bare_json.rs
[`DanceScript`]: ../stackchan-core/src/dance.rs
