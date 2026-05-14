---
title: Sidecar agent — push-to-talk + HTTP client
---

# Sidecar agent

The firmware ships without a built-in LLM or STT. The desk-toy
surface stays `no_std` + local-first. For the operator-visible
"speak to the avatar, get a reply" path, the firmware can be
pointed at an HTTP sidecar that owns STT + LLM + emotion-tagging.
The operator chooses cloud-or-not by where they point the sidecar
URL.

## Enabling the agent

Set `behavior.agent_sidecar_url` in `STACKCHAN.RON`:

```ron
behavior: (
    agent_sidecar_url: "http://192.168.1.42:8080/v1/listen",
    // ...other behavior flags...
)
```

Empty (the default) parks the agent task — no socket, no PTT
consumer. The cosmetic listen window (Ear decorator, ack chirp,
`Attention::Listening`) still runs on every `POST /listen` even
without a sidecar configured.

Hostnames are not resolved. Use a raw IPv4 literal — same shape as
`audio_debug_udp_target`. DNS support is a future extension.

## Wire protocol

### Request (firmware → sidecar)

```
POST /your/path HTTP/1.1
Host: 192.168.1.42:8080
Content-Type: audio/L16;rate=16000;channels=1
Content-Length: <n>
Connection: close

<n bytes of raw little-endian s16 PCM @ 16 kHz mono>
```

The capture window length is set by the `duration_ms` field of the
`POST /listen` body — `{"duration_ms": 5000}` for a 5 s window.
Default is the same 3 000 ms the cosmetic listen modifier uses.
The firmware clamps capture at 30 s to keep PSRAM allocation
bounded.

### Response (sidecar → firmware)

A minimal flat JSON projection of an OpenAI Chat Completions
reply — the sidecar internally calls whatever LLM it wants, then
returns:

```json
{
  "text": "Sure! Let me check the weather for you.",
  "emotion": "happy"
}
```

| Field     | Required | Notes                                                                                |
|-----------|----------|--------------------------------------------------------------------------------------|
| `text`    | yes      | Assistant reply. Surfaced on the firmware toast band (truncated to 32 chars).        |
| `emotion` | no       | One of `neutral` / `happy` / `sleepy` / `surprised` / `sad` / `angry`. Fires a 2.5 s `SetEmotion` hold. Unknown values are ignored. |

Response status must be `2xx`. Anything else (4xx, 5xx) is treated
as a failure and surfaces as a `sidecar: post failed` toast.

Backslash-escaped quotes inside the value strings are not handled
by the firmware-side parser. A well-behaved sidecar emits clean
ASCII / UTF-8 strings without embedded quotes; if literal quotes
are unavoidable, wrap or pre-substitute them on the sidecar side.

## Failure surface

Every error path surfaces a toast so the operator sees the
failure without an attached monitor:

| Toast text             | Cause                                                       |
|------------------------|-------------------------------------------------------------|
| `sidecar: link down`   | Wi-Fi disconnected between PTT trigger and POST attempt.    |
| `sidecar: post failed` | Connect / write / read / non-2xx / missing `text` field.    |
| `sidecar: timed out`   | Whole exchange exceeded 15 s.                               |

Full failure detail is logged via `defmt::warn!` over the
USB-Serial-JTAG monitor.

## Smoke-testing with curl + a minimal sidecar

A trivial sidecar that echoes a canned reply, useful to verify the
firmware's capture + POST + parse path end-to-end before plugging
in a real LLM:

```bash
# python3 -m http.server doesn't accept POST; use a 6-line nc loop.
while true; do
  printf 'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{"text":"hello from the sidecar","emotion":"happy"}' \
    | nc -lq 1 -p 8080
done
```

Then, with `behavior.agent_sidecar_url = "http://<host-ip>:8080/"`
in `STACKCHAN.RON`:

```bash
curl -X POST http://<device-ip>/listen \
  -H 'Content-Type: application/json' \
  -d '{"duration_ms": 3000}'
```

Within ~3.5 seconds the toast band should show
`hello from the sidecar` and the avatar should hold a `Happy`
emotion for ~2.5 s.

## What the firmware does *not* do

- No STT, no LLM, no TTS for the reply text. The sidecar owns those.
- No streaming response (the firmware reads until the peer closes;
  use `Connection: close` and a complete JSON body).
- No emotion vocabulary beyond the six canonical names. New
  emotion tags require a new `Emotion` enum variant in
  `stackchan-core`.
- No conversation memory between requests. Each `POST /listen`
  uploads a fresh capture; the sidecar owns any cross-turn state.
- No on-device wake-word yet (`microWakeWord` integration is a
  separate arc). Today every capture window is operator-driven via
  `POST /listen`, the MCP `start_listen` tool, or a future
  body-touch trigger.
