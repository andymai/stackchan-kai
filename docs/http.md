---
title: HTTP control plane
---

# HTTP control plane

A LAN-scoped HTTP/1.1 server runs on port 80 once Wi-Fi connects.
It exposes a small, fixed set of routes for live state, manual
override, persistent config, and an embedded operator dashboard.
Write routes (`PUT`, `POST`) are gated on a configurable bearer
token — empty token (default) leaves the LAN open, matching the
offline-first stance for Wi-Fi. See [Auth](#auth) for how to enable
it; [security](#security) covers what's still out of scope.

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
| GET    | `/settings`      | Persisted config (PSK + token redacted)             |
| PUT    | `/settings`      | Replace persisted config; atomic SD writeback        |

Write routes (`PUT`, all `POST`) require `Authorization: Bearer <token>`
when `auth.token` is configured. Reads are always unauthenticated.

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
 "time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]},
 "auth":{"token":"***"}}
```

`wifi.psk` and a non-empty `auth.token` are redacted to `***`. The
server rejects PUT bodies that contain `***` for either field so a
copy-paste round trip can't silently overwrite the real value with
the redaction sentinel. An empty `auth.token` (= auth disabled)
renders as `""` and round-trips losslessly.

### `PUT /settings`

```
$ curl -X PUT http://stackchan.local/settings \
       -H 'Content-Type: application/json' \
       -H 'Authorization: Bearer s3cret' \
       -d '{"wifi":{"ssid":"new-net","psk":"realkey","country":"US"},
            "mdns":{"hostname":"stackchan"},
            "time":{"tz":"UTC","sntp_servers":["pool.ntp.org"]},
            "auth":{"token":"s3cret"}}'
{"reboot_required":true}
```

Drop the `Authorization` header (and replace `auth.token` with `""`)
to disable auth on a device that previously had it enabled.

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
| 400  | Malformed JSON, missing required fields, unknown field, invalid emotion, redacted PSK or token sentinel, validation failure on PUT `/settings` |
| 401  | Write route called without a valid bearer token (when auth is enabled) |
| 404  | Path not in the matcher                                             |
| 405  | Method not allowed                                                  |
| 413  | Request body exceeds `MAX_BODY_BYTES` (1024)                        |
| 431  | Headers exceed `REQUEST_BUF_BYTES` before `\r\n\r\n` is reached     |
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

## Auth

Write routes (`PUT`, all `POST`) accept an optional bearer token.
Token is stored in `auth.token` of the persisted config and read
once at boot into a snapshot the HTTP handler consults on every
write request. Empty token (default) disables the gate; non-empty
token requires `Authorization: Bearer <token>` and returns `401` on
mismatch.

Compare is constant-time across the byte length so a co-located
attacker can't leak the token byte by byte through timing — though
on a LAN, network jitter dominates any sub-microsecond difference.

```
$ curl -X POST http://stackchan.local/emotion
HTTP/1.1 401 Unauthorized
unauthorized

$ curl -X POST http://stackchan.local/emotion \
       -H 'Authorization: Bearer s3cret' \
       -H 'Content-Type: application/json' \
       -d '{"emotion":"happy"}'
HTTP/1.1 204 No Content
```

To enable auth on a fresh kit:

```
$ curl -X PUT http://stackchan.local/settings \
       -H 'Content-Type: application/json' \
       -d '{"wifi":{...},"mdns":{...},"time":{...},
            "auth":{"token":"s3cret"}}'
$ # reboot the device — Wi-Fi keeps the boot config until restart
```

The dashboard at `GET /` reads the configured token from
`localStorage` and prompts the operator on its first `401`. The
typed value is persisted to the device on `PUT /settings` *and* to
`localStorage`, so a freshly configured browser keeps writing
without re-prompting until the device reboots.

## Security

What's covered:

- LAN scope (port 80 binds to `0.0.0.0`; reachable inside the
  network the device joined).
- Bearer-token gate on writes.
- PSK and auth-token redaction on `GET /settings`.
- Atomic SD writeback for `PUT /settings`.

What isn't:

- **No TLS.** Tokens cross the wire in cleartext. A network
  observer on the same broadcast domain can capture and replay.
- **No rate limiting.** A misconfigured client can hammer `401`s
  without throttling.
- **No replay protection.** A captured request can be re-sent.
- **No CSRF protection on the dashboard.** Any page on the same
  LAN that can fetch the dashboard can also drive writes through
  the operator's browser.

These are acceptable for a desktop toy on a trusted home LAN and
explicitly out of scope for v0.x. If you put Stack-chan on an
untrusted network, fence it off.

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
