---
title: VoiceVox — self-hosted TTS engine
---

# VoiceVox TTS

[VoiceVox](https://voicevox.hiroshiba.jp/) is a self-hostable Japanese
TTS engine. The firmware can drive it directly over HTTP to turn
dynamic speech text into spoken audio, separate from the
[sidecar](sidecar.md) path (which owns STT + LLM). API-compatible
engines such as
[AivisSpeech](https://github.com/Aivis-Project/AivisSpeech-Engine)
speak the same wire format and work the same way.

This page covers running the engine and pointing the firmware at it.
The wire format (URL builders, the audio-query rate override, and the
WAV decoder) lives in
[`stackchan-tts::voicevox`](../crates/stackchan-tts/src/voicevox.rs);
the firmware synthesis task that fetches and enqueues audio ships in a
follow-up — until then `voicevox_url` is parsed and persisted but the
on-device fetch path is not yet wired.

## Run the engine

The upstream Docker image listens on port `50021`:

```sh
docker run --rm -p 50021:50021 voicevox/voicevox_engine:cpu-latest
```

Confirm it answers on the LAN IP the avatar will reach (not
`localhost` — the firmware connects from another host):

```sh
curl -s http://192.168.1.50:50021/speakers | head -c 200
```

## Point the firmware at it

Set `behavior.voicevox_url` (and optionally
`behavior.voicevox_speaker_id`) in `STACKCHAN.RON`:

```ron
behavior: (
    voicevox_url: "http://192.168.1.50:50021",
    voicevox_speaker_id: 1,
    // ...other behavior flags...
)
```

Empty (the default) disables the synthesis task. Both fields also
round-trip through `PUT /settings`; see [behavior](behavior.md) for
the full field table and reboot semantics.

Hostnames are not resolved — use a raw IPv4 literal, same as
`agent_sidecar_url`. DNS support is a future extension.

`voicevox_speaker_id` selects the voice. `1` is Zundamon "ノーマル";
the engine's `GET /speakers` lists the full catalogue with per-style
IDs.

## Sample rate: 16 kHz, not 24 kHz

VoiceVox renders at 24 kHz by default, but the CoreS3 I²S output path
plays a single fixed rate (16 kHz). The firmware therefore rewrites
the `outputSamplingRate` field in the engine's audio-query response to
16 000 before requesting synthesis, so no on-device resampler is
needed. If the engine returns a different rate the WAV decoder rejects
the response rather than playing it back at the wrong pitch.

To reproduce the firmware's two-step request by hand:

```sh
ENGINE=http://192.168.1.50:50021
# Step 1: audio query (prosody JSON), rewriting the output rate to 16 kHz.
curl -s -X POST "$ENGINE/audio_query?speaker=1&text=こんにちは" \
  | sed 's/"outputSamplingRate":[0-9]*/"outputSamplingRate":16000/' \
  > query.json
# Step 2: synthesis -> 16 kHz mono WAV.
curl -s -X POST "$ENGINE/synthesis?speaker=1" \
  -H 'Content-Type: application/json' --data @query.json -o out.wav
```

## Security

Like the sidecar, the firmware speaks plain HTTP on the LAN — no TLS,
no auth. VoiceVox upstream has no authentication of its own, so keep
the engine on a trusted segment and don't expose port `50021` to the
internet.

## Related

- [voice](voice.md) — the end-to-end wake-word + sidecar onboarding flow.
- [behavior](behavior.md) — the `voicevox_url` / `voicevox_speaker_id`
  config fields and reboot semantics.
- [`stackchan-tts`](../crates/stackchan-tts/README.md) — the
  `SpeechBackend` surface and the VoiceVox wire-format helpers.
