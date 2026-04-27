---
title: HTTP control plane
---

# HTTP control plane

A LAN-scoped HTTP/1.1 server runs on port 80 once Wi-Fi connects.
It exposes a small, fixed set of routes for live state, manual
override, persistent config, and an embedded operator dashboard. No
auth; the security boundary is the LAN. See the [security note](#security)
for what that means.

The wire-format implementation is hand-rolled in
[`crates/stackchan-firmware/src/net/http.rs`](https://github.com/andymai/stackchan-kai/tree/main/crates/stackchan-firmware/src/net/http.rs)
— the surface is small enough that a hand-rolled request matcher
beats pulling a full HTTP framework into the firmware target.

## Routes

| Method | Path             | Description                                         |
|--------|------------------|-----------------------------------------------------|
| GET    | `/`              | Operator dashboard (HTML + JS, embedded in firmware) |
| GET    | `/health`        | Uptime, firmware version, free heap                  |
| GET    | `/state`         | Snapshot JSON: emotion, head pose, battery, Wi-Fi    |
| GET    | `/state/stream`  | Server-Sent Events stream of state changes          |
| POST   | `/emotion`       | Set affect with hold timer                          |
| POST   | `/look-at`       | Aim head + eyes with hold timer                     |
| POST   | `/reset`         | Clear active emotion / look-at hold                 |
| GET    | `/settings`      | Persisted config (PSK redacted)                     |
| PUT    | `/settings`      | Replace persisted config; atomic SD writeback        |

## Live state

```
$ curl http://stackchan.local/state
{"emotion":"Neutral","head_pose":{"pan_deg":0.00,"tilt_deg":0.00},
 "head_actual":{"pan_deg":0.10,"tilt_deg":-0.20},
 "battery":{"percent":78,"voltage_mv":3920},
 "wifi":{"connected":true,"ip":"192.168.1.42"}}
```

`GET /state/stream` opens a Server-Sent Events stream. The server
emits an initial event on connect, then one event per change. Idle
connections receive `: heartbeat` SSE comment lines every 15 s so
proxies and NAT idle timers don't close the connection.

```
$ curl -N http://stackchan.local/state/stream
data: {"emotion":"Neutral", ...}

data: {"emotion":"Happy", ...}

: heartbeat

```

Producer-side throttling caps events at ~10 Hz even when the
underlying render loop ticks at 30 Hz.

The HTTP layer accepts on a pool of worker tasks, so a long-lived
SSE stream doesn't block other requests on the same port.

## Manual control

`POST /emotion`, `POST /look-at`, and `POST /reset` write into the
modifier pipeline through the `RemoteCommandModifier` in
[`stackchan-core`](https://github.com/andymai/stackchan-kai/tree/main/crates/stackchan-core/src/modifiers/remote_command.rs).
Each command takes effect on the next render tick.

### `POST /emotion`

```
$ curl -X POST http://stackchan.local/emotion \
       -H 'Content-Type: application/json' \
       -d '{"emotion":"happy","hold_ms":30000}'
```

| Field    | Type   | Required | Notes                                             |
|----------|--------|----------|---------------------------------------------------|
| emotion  | string | yes      | `neutral` / `happy` / `sad` / `sleepy` / `surprised` / `angry` |
| hold_ms  | u32    | no       | Default 30 000. `0` is fire-and-forget.            |

The hold blocks autonomous emotion drivers (touch, IR, ambient,
battery, EmotionCycle) for `hold_ms`. Source recorded as
`OverrideSource::Remote`.

### `POST /look-at`

```
$ curl -X POST http://stackchan.local/look-at \
       -H 'Content-Type: application/json' \
       -d '{"pan_deg":12.0,"tilt_deg":-3.0,"hold_ms":30000}'
```

| Field    | Type | Required | Notes                                       |
|----------|------|----------|---------------------------------------------|
| pan_deg  | f32  | yes      | Same coordinate system as `motor.head_pose` |
| tilt_deg | f32  | yes      | "                                           |
| hold_ms  | u32  | no       | Default 30 000.                             |

The handler writes `mind.attention = Attention::Tracking { target }`.
`HeadFromAttention` and `GazeFromAttention` translate that into
motor + eye motion. While the hold is active, fresh tracking
observations from the camera don't stomp the operator's target.

### `POST /reset`

```
$ curl -X POST http://stackchan.local/reset
```

Empty body. Clears any active emotion or look-at hold; autonomous
behaviours resume on the next render tick.

## Persistent config

The boot config lives at `/sd/STACKCHAN.RON` in the schema described
by [`stackchan-net`](https://github.com/andymai/stackchan-kai/tree/main/crates/stackchan-net).
The HTTP control plane round-trips it as JSON.

### `GET /settings`

```
$ curl http://stackchan.local/settings
{"wifi":{"ssid":"my-net","psk":"***","country":"US"},
 "mdns":{"hostname":"stackchan"},
 "time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]}}
```

`wifi.psk` is always redacted to `***`. The server rejects PUT
bodies that contain `***` for the PSK so a copy-paste round trip
can't silently overwrite the real key with the redaction sentinel.

### `PUT /settings`

```
$ curl -X PUT http://stackchan.local/settings \
       -H 'Content-Type: application/json' \
       -d '{"wifi":{"ssid":"new-net","psk":"realkey","country":"US"},
            "mdns":{"hostname":"stackchan"},
            "time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]}}'
{"reboot_required":true}
```

Full-replace. The server validates the body via
`stackchan_net::validate` (rejects empty SSID, invalid country code,
invalid hostname, empty SNTP list) and then writes back atomically:
the new config goes to `/sd/STACKCHAN.NEW`, gets copied onto
`/sd/STACKCHAN.RON`, and the staging file is removed. Mid-write
power loss leaves the old file intact.

The firmware does **not** tear down Wi-Fi on save — that would drop
the operator's HTTP session if the SSID changed. Reboot to apply.

## Status codes

| Code | Where it shows up                                                  |
|------|---------------------------------------------------------------------|
| 200  | GET responses, dashboard, successful PUT                            |
| 204  | POST `/emotion` / `/look-at` / `/reset` on success                  |
| 400  | Malformed JSON, missing required fields, unknown field, invalid emotion, redacted PSK sentinel, validation failure on PUT `/settings` |
| 404  | Path not in the matcher                                             |
| 405  | Method not allowed                                                  |
| 500  | SD write failed during PUT `/settings`                              |
| 503  | PUT `/settings` with no SD; GET `/settings` before the config snapshot is loaded; no free SSE subscriber slot |

Error responses have `Content-Type: text/plain` with a single-line
reason — operators triage from `defmt` boot logs, the HTTP body is
just a hint.

## Dashboard

`GET /` returns a self-contained HTML page embedded in the firmware
binary via `include_bytes!`. Vanilla DOM + `EventSource` + `fetch`,
no framework, no external CDN. The dashboard:

- Subscribes to `/state/stream` for live state.
- Has a button per emotion (POST `/emotion` with a 30 s hold).
- Renders pan/tilt sliders that POST `/look-at` on release.
- Loads `/settings` into an editable form; submits PUT `/settings`
  on save and surfaces the `reboot_required` hint via toast.

To customise it, edit the HTML in
[`crates/stackchan-firmware/src/net/dashboard.html`](https://github.com/andymai/stackchan-kai/tree/main/crates/stackchan-firmware/src/net/dashboard.html)
and re-flash. Embedding via `include_bytes!` instead of serving from
SD is intentional: the dashboard works on a card-less device.

## Security

There is no auth. Anyone on the same LAN can:

- Read the persisted config (with PSK redaction).
- Overwrite the persisted config (without redaction — `PUT` accepts
  the real PSK as plaintext on the wire).
- Drive the avatar to arbitrary poses and emotions.

This is acceptable for a desktop toy on a trusted home LAN and
explicitly out of scope for v0.x. If you put Stack-chan on an
untrusted network, fence it off — there's no TLS, no auth, no rate
limiting on this server.

## Worker pool sizing

The HTTP layer spawns a fixed pool of worker tasks
(`HTTP_WORKER_COUNT` in
[`net/http.rs`](https://github.com/andymai/stackchan-kai/tree/main/crates/stackchan-firmware/src/net/http.rs)).
Each worker holds its own rx/tx buffers and accepts one connection
at a time. SSE subscribers occupy a worker for the lifetime of the
stream; ordinary GET / POST / PUT requests free the worker on
response close.

`SSE_MAX_SUBSCRIBERS` in
[`net/snapshot.rs`](https://github.com/andymai/stackchan-kai/tree/main/crates/stackchan-firmware/src/net/snapshot.rs)
caps the concurrent SSE consumers. An SSE connection that arrives
when no subscriber slot is free gets `503 stream slots exhausted`
back. The two constants are coupled — bumping concurrency requires
raising both.
