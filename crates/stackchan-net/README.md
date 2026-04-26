---
crate: stackchan-net
role: Networking domain types + RON config schema (host-testable)
bus: none
transport: "pure data + parsers"
no_std: true
unsafe: forbidden
status: experimental (v0.x)
---

# stackchan-net

Networking domain types for Stack-chan. Pure data and parsers — no
transport, no I/O, no esp-hal. The firmware does the I/O wrapping;
this crate is what the firmware (and host tests) agree on as the
shape of the on-disk config and the RON file `PUT /settings` round-trips.

## Schema v1

```ron
(
    wifi: ( ssid: "home", psk: "redacted", country: "US" ),
    mdns: ( hostname: "stackchan" ),
    time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
)
```

- `WifiConfig` — credentials + ISO-3166 alpha-2 country code (default `"US"`).
- `MdnsConfig` — hostname advertised on `.local` (default `"stackchan"`).
- `TimeConfig` — IANA timezone label + SNTP servers (default `"UTC"`,
  `["pool.ntp.org"]`). The TZ field is parsed but currently unused;
  the BM8563 RTC stores UTC.

## Key Files

- `src/lib.rs` — crate root, re-exports
- `src/config.rs` — `Config`, `WifiConfig`, `MdnsConfig`, `TimeConfig`,
  `parse_ron`, `render_ron`, validators
- `src/error.rs` — `ConfigError` (parse / serialize / validation variants)
- `tests/golden_config.rs` + `tests/fixtures/*.ron` — round-trip and
  validation coverage against hand-written fixtures

## Offline-first stance

The avatar must boot fully and animate even with no SD card and no
Wi-Fi. The firmware therefore treats `Config` as **always available**:
missing SD or missing file falls back to `Config::default`. Validators
in `parse_ron` reject malformed input, but the firmware never
propagates a `ConfigError` up to a panic — it logs and uses defaults.

## Defer-list (out of scope for v1)

- TLS / HTTPS config — the v1 control plane is LAN-scoped, no auth.
- OTA manifests — firmware updates are a separate concern.
- Captive portal / soft-AP setup flow — first-boot UX deferred.
- Persona / character data — belongs to the AI tier (deferred).
- BLE pairing config — companion-app pairing is a later round.

[`Config`]: src/config.rs
[`WifiConfig`]: src/config.rs
[`MdnsConfig`]: src/config.rs
[`TimeConfig`]: src/config.rs
[`ConfigError`]: src/error.rs
