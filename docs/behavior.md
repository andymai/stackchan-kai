---
title: Behavior flags — operator reference
---

# Behavior flags

`STACKCHAN.RON`'s `behavior:` block gathers the firmware's opt-in
surfaces: visual overlays, motor-saver timing, sidecar URLs,
wake-word knobs. The block is optional — leave it out entirely and
every flag falls back to the defaults below, all chosen so a stock
device boots into a quiet desk-toy posture (no chimes, no toasts,
no battery icon, no agent, motors held).

The source of truth is [`BehaviorConfig`](https://github.com/andymai/stackchan-kai/blob/main/crates/stackchan-net/src/config.rs)
in `stackchan-net`. New fields land there with their own doc comment;
this page is a thin operator reference, kept in sync via PR review
rather than as a separate process. When a field's behavior is rich
enough to warrant its own page (e.g. audio debug, sidecar), the
**Notes** column links to it.

## Reference

| Field | Type | Default | Reboot? | Notes |
|---|---|---|---|---|
| `soliloquy_enabled` | bool | `false` | live | Writes random soliloquy lines into `face.bubble`. Bubble TTL is fixed; no audio playback in this iteration. |
| `hourly_chime_enabled` | bool | `false` | live | Plays a short chirp at the top of every wall-clock hour. Waits for SNTP sync before scheduling. |
| `battery_icon_enabled` | bool | `false` | live | Small battery glyph top-left of the face. Quantised at the source so a 1 % change does not redraw. |
| `toast_overlay_enabled` | bool | `false` | live | Bottom-of-screen toast band for warn / error events (`POST /toast`, `crate::toast::push`, MCP `push_toast`). 3 s TTL. |
| `auto_torque_release_ms` | u32 | `0` | live | Idle window after which SCServo holding torque releases. `0` keeps torque on continuously (v0.1.0 behavior). Useful for units that sit idle for hours. |
| `audio_debug_udp_target` | `"host:port"` | empty | live | UDP tee of the microphone stream. Bench-only — see [audio-debug](audio-debug). |
| `agent_sidecar_url` | URL | empty | live | HTTP target for push-to-talk audio. Empty disables the agent task. See [sidecar](sidecar) for the wire protocol and [voice](voice) for the end-to-end loop. |
| `agent_sidecar_token` | string | empty | live | Bearer token for the sidecar. Wire-redacted in `GET /settings` exactly like `wifi.psk` and `auth.token`. |
| `follower_leader_hostname` | string | empty | live | Bare first label of another Stack-chan to mimic via mDNS (e.g. `"kitchen-cat"`, not `"kitchen-cat.local"`). Empty disables follower mode. |
| `wake_word_enabled` | bool | `false` | **reboot** | Runs on-device wake-word detection. Loads `/sd/WAKE_WORD.tflite` at boot; missing file parks the task. |
| `wake_word_threshold` | i8 | `100` | **reboot** | Int8 detection cut-point. Bundled-model reference ≈ 0.95 confidence lands near 100. Lower = more sensitive. |
| `wake_word_arena_kib` | u32 | `64` | **reboot** | TFLite Micro arena size in KiB. Allocated once at boot from PSRAM. Must be `>= 1`; `0` is rejected by validation. |

Reboot semantics: every field marked **live** takes effect on the
next render frame (`POST /settings` immediately signals the relevant
task). Fields marked **reboot** are read once at task spawn and need
a power-cycle or `POST /restart` to pick up the new value. `PUT /settings`
echoes `{"reboot_required": <bool>}` so operators don't have to
remember the table.

## Workflows

### Edit + apply via the dashboard

Open `http://<hostname>.local/` in a LAN browser; the embedded
dashboard surfaces the same `behavior:` block as form fields,
posts via `PUT /settings`, and shows `reboot_required: true` as a
banner when relevant.

### Edit + apply via the SD card

Power off the avatar, mount the SD card on the host, edit
`/STACKCHAN.RON` directly, eject, power on. RON is more permissive
about formatting than JSON — multi-line values, trailing commas,
comments are all fine. The firmware re-validates on boot; a malformed
file falls back to the offline defaults and logs the failure over
USB-Serial-JTAG.

### Edit + apply via curl

```sh
# Round-trip the current config (PSK + tokens redacted as "***")
curl -sS http://stackchan.local/settings > settings.json
$EDITOR settings.json
curl -sS -X PUT --data-binary @settings.json \
    -H 'Content-Type: application/json' \
    http://stackchan.local/settings
```

The redacted `"***"` sentinel echoes back unchanged on `PUT` and
the firmware substitutes the persisted value — leave `wifi.psk`,
`auth.token`, `behavior.agent_sidecar_token`, and the `esp_now.*_hex`
fields alone unless you actually want to rotate them.

### Confirm what the firmware actually has

```sh
curl -sS http://stackchan.local/settings | jq .behavior
```

`GET /state` also surfaces a few live flags (e.g. wake-word arming)
under `behavior` for telemetry consumers.

## Adding a new behavior field

Three files change in lockstep:

1. **`crates/stackchan-net/src/config.rs`** — add the field to
   `BehaviorConfig` with a doc comment, then update `Default` so
   omitting the block keeps the new field's default consistent
   with the in-parser fallback.
2. **`crates/stackchan-net/src/bare.rs` + `bare_json.rs`** — render
   + parse arms with a duplicate-key guard. Both backends have a
   `parse_behavior` test fixture; extend both.
3. **`crates/stackchan-firmware/src/net/http.rs`** — if the field
   is read once at boot (task spawn time), add it to `requires_reboot`.
   If the field has invalid values, extend `validate()` in
   `stackchan-net` with a typed `ConfigError` variant + a regression
   test for the rejection.

Greptile reliably flags missing `requires_reboot` entries and
missing `validate` guards — both have bitten this repo before
(see PRs around #354 for the latest combined audit). When in doubt,
add both gates.

## Related

- [HTTP control plane](http) — full route surface; `PUT /settings`
  semantics and the redacted-field round trip.
- [Sidecar agent](sidecar) — wire protocol for
  `agent_sidecar_url` / `agent_sidecar_token`.
- [Voice agent onboarding](voice) — end-to-end flow for wake word
  + sidecar.
- [UDP audio debug](audio-debug) — receive recipe for
  `audio_debug_udp_target`.
