---
title: Sidecar agent — push-to-talk + HTTP client
---

# Sidecar agent

> **Looking for setup?** [`voice.md`](voice.md) is the
> onboarding flow that covers wake word, sidecar config, and the
> on-device loop end-to-end. This page focuses on the wire protocol
> the firmware speaks to the sidecar.

The firmware ships without a built-in LLM or STT. The desk-toy
surface stays `no_std` + local-first. For the operator-visible
"speak to the avatar, get a reply" path, the firmware can be
pointed at an HTTP sidecar that owns STT + LLM + emotion-tagging.
The operator chooses cloud-or-not by where they point the sidecar
URL.

## Enabling the agent

Set `behavior.agent_sidecar_url` (and optionally `behavior.agent_sidecar_token`)
in `STACKCHAN.RON`:

```ron
behavior: (
    agent_sidecar_url: "http://192.168.1.42:8080/v1/listen",
    agent_sidecar_token: "sk-sidecar-shared-secret",
    // ...other behavior flags...
)
```

Empty (the default) parks the agent task — no socket, no PTT
consumer. The cosmetic listen window (Ear decorator, ack chirp,
`Attention::Listening`) still runs on every `POST /listen` even
without a sidecar configured.

Hostnames are not resolved. Use a raw IPv4 literal — same shape as
`audio_debug_udp_target`. DNS support is a future extension.

The dashboard's Settings panel exposes both fields directly — they
round-trip through `PUT /settings` with the same `***` redaction
semantics as the Wi-Fi PSK and the HTTP auth token.

## Reference implementation

A working sidecar lives in [`sidecar/`](../sidecar/README.md) in
this repo: Python 3.12 + FastAPI, faster-whisper STT (or OpenAI
Whisper / Deepgram), Anthropic Claude (or OpenAI / Ollama),
per-`X-Session-Id` conversation memory, structured JSON logs, a
`/healthz` probe, Dockerfile, and an example systemd unit. Operators
who want a turnkey loop can `uv sync && uv run stackchan-sidecar`
inside `sidecar/` and point `behavior.agent_sidecar_url` at it.
Persona prompt lives at `sidecar/personas/stack-chan.md` — copy
and edit for a different voice.

`agent_sidecar_token` is the shared-secret bearer token presented
to the sidecar as `Authorization: Bearer <token>` on every POST.
Empty disables the header — only safe on a fully trusted LAN where
no other host can reach the sidecar. The token is wire-redacted on
`GET /settings` (echoed as `"***"`) and round-trips losslessly
through `PUT /settings` when the operator submits the same
sentinel.

## Wire protocol

### Request (firmware → sidecar)

```
POST /your/path HTTP/1.1
Host: 192.168.1.42:8080
Content-Type: audio/L16;rate=16000;channels=1
Content-Length: <n>
Connection: close
Authorization: Bearer sk-sidecar-shared-secret
X-Session-Id: 7f3c2a1d-9b40-4e8a-93f1-2bc6d4e1a7f0

<n bytes of raw little-endian s16 PCM @ 16 kHz mono>
```

The capture window length is set by the `duration_ms` field of the
`POST /listen` body — `{"duration_ms": 5000}` for a 5 s window.
Default is the same 3 000 ms the cosmetic listen modifier uses.
The firmware clamps capture at 30 s to keep PSRAM allocation
bounded.

`Authorization` is only sent when `agent_sidecar_token` is set.
`X-Session-Id` is sent on every healthy boot — it carries a
canonical UUIDv4 the firmware mints on first boot and persists to
`/sd/SESSION.UUID`. The send-side guard skips the header if the
hydrated value is empty (it never is in practice). Sidecars that
care about multi-turn context key memory off this value; sidecars
that don't can ignore it. Deleting the file rotates the identifier;
copying it across SD cards preserves it. SD-less boots get a fresh
ephemeral ID per cold start.

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

The avatar's face mirrors the round-trip in three beats:
`Listening` (Ear decorator) during PCM capture →
`Thinking` (thought-bubble) while the POST is in flight →
the emotion + speech bubble carrying the reply. The thinking hold
clears the instant a `SetEmotion` lands, so the bubble fades in
sync with the visible reply. On any failure path — link-down,
POST failure, or timeout — the firmware fires `SetEmotion` with
`Emotion::Sad` for 2.5 s, so the face visibly registers the
failure on top of the warn-class toast.

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

## Smoke-testing without a real LLM

The fastest way to verify the firmware's capture + POST + parse
path end-to-end is the in-tree reference sidecar (see [Reference
implementation](#reference-implementation)) pointed at a stub
persona, but if you want to skip the Python toolchain entirely a
6-line `nc` loop is enough:

```bash
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
emotion for ~2.5 s. The `nc` loop ignores `Authorization` and
`X-Session-Id` — set both on the firmware side to whatever you
want; nothing on the receiving end checks.

## What the firmware does *not* do

- No STT, no LLM, no TTS for the reply text. The sidecar owns those.
- No streaming response (the firmware reads until the peer closes;
  use `Connection: close` and a complete JSON body).
- No emotion vocabulary beyond the six canonical names. New
  emotion tags require a new `Emotion` enum variant in
  `stackchan-core`.
- No conversation memory of its own. Each `POST /listen` uploads
  a fresh one-shot capture; the sidecar is responsible for any
  cross-turn state. `X-Session-Id` exists precisely so the sidecar
  can scope that state to one physical device.
- Capture windows open from one of three triggers: `POST /listen`,
  the MCP `start_listen` tool, or the on-device microWakeWord
  detector (opt-in via `behavior.wake_word_enabled` plus a
  `.tflite` model at `/sd/WAKE_WORD.tflite`; detection cut-point
  is `behavior.wake_word_threshold`, signed int8, default `100`).
  All three converge on the same `RemoteCommand::StartListen`
  signal, so the sidecar request shape is identical regardless
  of trigger.
