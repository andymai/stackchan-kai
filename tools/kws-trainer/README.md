# stackchan-kws-trainer

Host tooling for capturing wake-word training samples from a
stackchan-kai device. Eventually closes the loop: record samples →
build dataset → train a `microWakeWord` model → drop the `.tflite`
on the device's SD card → reboot.

This package walks the loop one step at a time:

- `kws-record` — capture a fixed-duration WAV from a running device.
- `kws-build-dataset` — turn a directory of labelled WAVs into a
  deterministic train/val/test manifest the trainer can consume.

The training wrapper (`kws-train`) and eval tool land in follow-ups.

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

## Building a dataset manifest

After collecting recordings (the recorder names files
`positive-001.wav`, `negative-042.wav`, etc.), turn them into a
deterministic train/val/test split:

```bash
uv run kws-build-dataset --input samples/ --output manifest.json
```

The output manifest is a JSON file with each split's recordings
listed as `(path, label, duration_seconds)` triples, plus a
per-split label-count summary. Same `--seed` + same input directory
gives a bit-stable manifest — re-run after adding more samples and
the previously-classified files stay in their original split.

Operator-facing knobs:

- `--val-fraction` / `--test-fraction` (defaults 0.15 each) —
  per-label stratified hold-out, so a small `silence` bucket isn't
  crowded out of validation by a much larger `negative` bucket.
- `--strict` — exit non-zero if any WAV is rejected (wrong sample
  rate, stereo, < 0.5 s). Useful in CI; default behaviour logs and
  continues so a single bad file doesn't stop manifest generation.

## What's next

- `kws-train` — wrap `microwakeword`'s training pipeline, emit a
  ready-to-flash `.tflite`.
- `kws-eval` — run a `.tflite` against a WAV, report detection
  scores. Useful for tuning the firmware's `wake_word_threshold`
  without reflashing.
