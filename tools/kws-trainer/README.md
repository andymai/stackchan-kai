# stackchan-kws-trainer

Host tooling for capturing wake-word training samples from a
stackchan-kai device. Eventually closes the loop: record samples →
build dataset → train a `microWakeWord` model → drop the `.tflite`
on the device's SD card → reboot.

This package starts that loop. Slice 1 ships only the recorder
(`kws-record`); dataset builder and training wrapper land in
follow-ups.

## Setup

```bash
cd tools/kws-trainer/
uv sync
```

## Recording samples

The firmware's `audio_debug_udp_target` feature forwards every 20 ms
ES7210 mic frame to a UDP target. Point it at your training host and
run `kws-record` to capture WAVs:

1. Edit `/sd/STACKCHAN.RON` (or use `PUT /settings`) and set:

   ```ron
   behavior: (
       audio_debug_udp_target: "192.168.1.42:5005",
       // ... other behavior flags ...
   )
   ```

2. Reboot the device.

3. On the training host:

   ```bash
   uv run kws-record --listen-port 5005 --duration 5 \
                     --output samples/positive-001.wav
   ```

   Speak the wake phrase during the recording window.

4. Repeat for ~50 positive samples (wake phrase spoken in different
   tones / distances) and ~200 negative samples (room noise, music,
   unrelated speech) to build a viable training set.

The WAV format is 16 kHz mono s16 LE — the same shape
`microwakeword`'s training pipeline ingests directly.

## What's next

- `kws-build-dataset` — directory of WAVs → microWakeWord training
  dataset shape (positive / negative / silence buckets).
- `kws-train` — wrap `microwakeword`'s training pipeline, emit a
  ready-to-flash `.tflite`.
- `kws-eval` — run a `.tflite` against a WAV, report detection
  scores. Useful for tuning the firmware's `wake_word_threshold`
  without reflashing.
