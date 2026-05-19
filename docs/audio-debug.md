---
title: UDP audio debug
---

# UDP audio debug

The firmware can tee its microphone capture to a LAN UDP target so a
host machine can listen along with `aplay` / `nc`. Opt-in via the
`behavior.audio_debug_udp_target` field in `STACKCHAN.RON`; empty
disables the stream entirely (the task parks and consumes no
resources).

## Enable

In `/sd/STACKCHAN.RON`:

```ron
behavior: (
    audio_debug_udp_target: "192.168.1.42:5005",
)
```

Substitute the IP of the host you'll receive on (must be on the same
LAN as the avatar) and any free UDP port. Reboot the avatar.

## Receive

```sh
nc -lu 192.168.1.42 5005 | aplay -r 16000 -f S16_LE -c 1 -t raw
```

Decoder parameters:

- `-r 16000` — sample rate matches firmware `SAMPLE_RATE_HZ`
- `-f S16_LE` — 16-bit signed little-endian, matches the on-wire layout
- `-c 1` — mono
- `-t raw` — no header

## Wire format

Each datagram is one 20 ms frame from `AUDIO_FRAME_PUBSUB`:
`320 × i16` little-endian = 640 bytes of payload. No header, no
sequence number — drops show up as audible gaps. At 50 frames/sec the
stream is ~32 KB/s of UDP traffic.

## When to use

- Verifying the ES7210 capture path works end-to-end without flashing
  a wake-word build.
- Inspecting environmental noise at a deployed unit to tune
  `EmotionFromVoice` / `IntentFromLoud` thresholds.
- Recording a sample for off-device model training.

The stream is **plaintext UDP** on the LAN; not suitable for a shared
network. Leave the field empty in any deployment outside a controlled
bench.

## Related

- [Behavior flags](behavior) — `audio_debug_udp_target` is one of
  the live-reloadable `behavior:` block fields.
- [Voice agent onboarding](voice) — the wake-word + sidecar loop
  uses the same audio capture path tapped here.
- [Sidecar agent](sidecar) — the agent the wake-word path forwards
  audio to once `audio_debug_udp_target` confirms capture works.
