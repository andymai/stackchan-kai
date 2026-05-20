---
title: Voice agent — wake word, sidecar, on-device loop
---

# Voice agent

End-to-end setup for the operator-visible "say a wake phrase, get a
reply" path. The firmware ships with no LLM, STT, or TTS — instead it
runs an on-device wake-word detector and forwards captured audio to an
HTTP sidecar the operator points it at. Everything is opt-in; without
configuration the device stays local-first and the voice path is
inert.

This page is the onboarding flow. For the wire protocol the firmware
speaks to the sidecar, see [Sidecar agent](sidecar.md); for the
underlying audio capture path, see [UDP audio debug](audio-debug.md).

## The loop, as seen on the device

1. **Wake** — wake word fires, or the operator hits `POST /listen` on
   the dashboard. The avatar plays an acknowledge chirp, the head
   eases into a cocked-head listen pose, and the ear decorator
   appears in the upper-left of the face.
2. **Capture** — the firmware records ~4 s of 16 kHz mono PCM from
   the ES7210 microphone (the wake-word fire defaults to 4 000 ms;
   `POST /listen` defaults to 3 000 ms; the body's `duration_ms`
   field overrides).
3. **Round-trip** — the captured PCM uploads to the sidecar over
   plain HTTP/1.1. The ear fades and a thought-bubble appears at
   the upper-right while the request is in flight.
4. **Reply** — the sidecar responds with `{"text": "...", "emotion":
   "..."}`. The thought-bubble fades, the avatar's emotion mirrors
   the tag, and the reply text scrolls in the toast band beneath
   the face.

If the sidecar fails or times out, the thought-bubble fades, the
face flips to Sad for ~2.5 s, and a warn-class toast (`sidecar:
post failed`, `sidecar: timed out`, `sidecar: link down`) explains
what happened. The avatar returns to autonomous behavior after the
hold expires.

## Prerequisites

- Wi-Fi configured via `/sd/STACKCHAN.RON` so the firmware has a
  route to the sidecar. SD-less boots and units without a Wi-Fi
  station keep the voice task parked.
- SD card present and writable. The runtime config + wake-word
  model both live on it.
- Optional but recommended: a sidecar reachable from the device's
  LAN, served on a raw IPv4 literal (DNS resolution is not wired
  up).

## Enable the sidecar (cloud-or-not, you choose)

Add the agent block to `/sd/STACKCHAN.RON`:

```ron
behavior: (
    agent_sidecar_url: "http://192.168.1.42:8080/v1/listen",
    agent_sidecar_token: "sk-sidecar-shared-secret",
    // ... other behavior flags ...
)
```

Empty `agent_sidecar_url` parks the agent task entirely — the
firmware still runs the cosmetic listen window (ear decorator + ack
chirp) on every `POST /listen`, but never posts audio anywhere.

The dashboard's Settings panel exposes both fields and round-trips
them through `PUT /settings` with `***`-redaction on `GET`, the same
shape as the Wi-Fi PSK and the dashboard auth token.

### Reference sidecar

The repo's [`sidecar/`](../sidecar/README.md) directory carries a
working implementation: Python 3.12 + FastAPI,
faster-whisper for STT, configurable LLM backend (Anthropic, OpenAI,
or Ollama), per-`X-Session-Id` conversation memory, structured JSON
logs, `/healthz` probe, Dockerfile, and an example systemd unit.
Operators who want a turnkey loop run `uv sync && uv run
stackchan-sidecar` inside `sidecar/` and point the firmware at it.

The persona prompt is a single editable markdown file at
`sidecar/personas/stack-chan.md`. Copy it for a different voice.

### Multiple personas on one sidecar

One sidecar can serve several personas — the firmware picks which
one to use per request via the `X-Persona-Name` header.

1. Drop more persona files into `sidecar/personas/`, one `.md` per
   voice (`desk-buddy.md`, `wake-only.md`, etc.). Each follows the
   same frontmatter shape as the bundled `stack-chan.md`.
2. Set `behavior.persona_name = "desk-buddy"` in the device's
   `/sd/STACKCHAN.RON` (or via `PUT /settings`). Empty / unset keeps
   the firmware's wire surface header-free, and the sidecar falls
   back to its baked-in `settings.persona`.
3. Confirm with `curl http://sidecar:port/v1/personas` — the
   response lists every deployed slug plus the configured default
   and a `default_deployed` flag that tells you upfront whether
   the install is healthy.

Validation runs at both boundaries: the firmware rejects slugs
longer than 64 bytes, with control characters, or containing path
separators / `..`; the sidecar re-checks per request so a curl
caller bypassing the firmware can't pivot the filesystem lookup.
Invalid slug → `400`; well-formed but missing file → `404`;
sidecar's default missing → `500` (operator misconfig, not your
caller's fault).

Conversation memory is partitioned per `(session_id, persona)`.
A device that switches personas under the same session_id (a
firmware reflash, or a future mid-runtime swap) gets a clean
slate for the new voice — the old voice's history stays
addressable under its own bucket until TTL expires.

## Enable the wake word (optional)

Wake-word detection is local-only — microWakeWord v2 streaming
inference via TFLite Micro + ESP-NN on the ESP32-S3. Two pieces
both have to be in place or the wake task parks at boot.

### 1. Drop a model on the SD card

Place a TFLite model at `/sd/WAKE_WORD.tflite`. The firmware reads
it once at boot and never re-reads — config changes via
`PUT /settings` take effect on the next reboot.

Compatible models: any int8-quantized streaming microWakeWord v2
`.tflite` from the [microWakeWord
library](https://github.com/kahrendt/microWakeWord) — `hey_jarvis`,
`okay_nabu`, `alexa`, custom-trained, and so on. The 20-operator
set the firmware registers matches what ESPHome's `streaming_model.cpp`
exposes, so any model that loads under ESPHome should load here.

A unit booted without `/sd/WAKE_WORD.tflite` still works — the wake
task parks; the sidecar still fires on `POST /listen`. The model is
only required for the hands-free path.

### 2. Flip the config flag

```ron
behavior: (
    wake_word_enabled: true,
    wake_word_threshold: 100,   // int8 cut-point; 100 ≈ 0.95 confidence
    wake_word_arena_kib: 64,    // TFLM tensor arena, in KiB
    // ... other behavior flags ...
)
```

- `wake_word_enabled` — falls back to `false` if the field is missing
  or the model isn't found. Both have to be true for the task to wire
  up.
- `wake_word_threshold` — int8 score above which the firmware treats
  the model output as a positive detection. Lower values (≈80)
  increase sensitivity at the cost of false positives; higher values
  (≈115) tighten it. Tune per model.
- `wake_word_arena_kib` — size of the TFLM tensor arena in KiB.
  The microWakeWord v2 family declares 17–26 KiB nominal; `64` leaves
  headroom for the TFLM planner's scratch space. Bump if a custom
  `.tflite` fails `allocate_tensors` at boot (the firmware logs a
  warning and parks).

A successful boot logs something like:

```
INFO  wake-word: interpreter ready (1 inputs, 1 outputs, arena=65536 B, threshold=100)
```

When the model fires, the log line is `wake-word: fired (score=…)`.

## Verify

### Without the wake word

Smoke-test the sidecar leg first — it removes a moving part.

`$HTTP_TOKEN` below is the device-level bearer token configured as
`auth.token` in `STACKCHAN.RON` (distinct from `agent_sidecar_token`,
which the firmware presents to the sidecar). Drop the
`Authorization` header entirely if `auth.token` is empty.

```sh
curl -H "Authorization: Bearer $HTTP_TOKEN" \
     -H "Content-Type: application/json" \
     -d '{"duration_ms": 3000}' \
     http://stackchan.local/listen
```

The avatar should chirp, show the ear decorator, capture 3 s of
audio, then post it to the sidecar. The reply text lands in the
toast band; if the reply tagged an emotion, the face flips for ~2.5
seconds before returning to autonomous behavior.

### With the wake word

Reboot the unit, watch the defmt monitor for `wake-word: interpreter
ready`, then speak the wake phrase. The path through the firmware is
identical to the route-driven test above — the wake-word detector
fires the same `RemoteCommand::StartListen` the HTTP route does.

The cooldown after a fire is 5 seconds (capture + 1 s padding), so
back-to-back triggers within that window are suppressed silently.

## Troubleshooting

| Symptom | First thing to check |
|---|---|
| `wake-word: disabled (enabled=…, model=…); idle` at boot | Either the config flag is `false` or `/sd/WAKE_WORD.tflite` wasn't found. |
| `wake-word: allocate_tensors failed` at boot | Bump `wake_word_arena_kib`; the model needs more arena than the default. |
| `wake-word: Interpreter::new failed (bad model or schema mismatch)` | The `.tflite` is malformed or uses operators outside the registered set. Try a stock microWakeWord library model first. |
| Wake fires constantly on ambient noise | Raise `wake_word_threshold` (try 110 → 120 in steps). |
| Wake never fires on real utterances | Lower `wake_word_threshold` (try 90 → 80 in steps). |
| `sidecar: link down` toast | Wi-Fi disconnected between the wake fire and the POST. Check the dashboard's network status. |
| `sidecar: post failed` toast | Sidecar unreachable, returned non-2xx, replied with malformed JSON, or the JSON body has no `text` field. Try `curl`ing the sidecar's `/healthz` directly and confirm the response includes a top-level `"text": "..."` key. |
| `sidecar: timed out` toast | Sidecar took longer than 15 s. Almost always means the LLM call upstream is slow; the firmware bound is fixed. |
| Avatar visibly listens but the reply never renders | Sidecar returned a 2xx but the body has no `text` field. The firmware drops the exchange and surfaces `sidecar: post failed`. |
| Audio capture sounds wrong / silent | Use the [UDP audio debug](audio-debug.md) stream to verify the microphone path independently of the sidecar. |

The firmware logs every fire and every failure via `defmt::info!` /
`defmt::warn!` over the USB-Serial-JTAG monitor. When something
breaks, `just reattach` is the fastest way to see what the device
saw without rebooting it.

## Security posture

Both legs are LAN-only and operator-trusted:

- The wake model runs entirely on-device; no audio leaves the unit
  until the wake fires (or `POST /listen` does).
- The sidecar POST is plain HTTP (no TLS) with optional bearer-token
  auth. Safe on a trusted LAN; never expose the sidecar to the open
  internet without a reverse proxy that terminates TLS and re-issues
  the bearer.
- The captured PCM does not stream — the firmware uploads it in one
  request at the end of the capture window.

Per-tenant or per-session policy belongs in the sidecar (where the
LLM and STT live). The firmware only ferries audio and renders the
reply.
