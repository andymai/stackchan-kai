# stackchan-kws-trainer

Host tooling for capturing wake-word training samples from a
stackchan-kai device. Eventually closes the loop: record samples →
build dataset → train a `microWakeWord` model → drop the `.tflite`
on the device's SD card → reboot.

This package walks the loop one step at a time:

- `kws-record` — capture a fixed-duration WAV from a running device.
- `kws-build-dataset` — turn a directory of labelled WAVs into a
  deterministic train/val/test manifest the trainer can consume.
- `kws-eval` — run a trained `.tflite` against a WAV and report
  detection scores. Useful for tuning the firmware's
  `wake_word_threshold` without reflashing.

The training wrapper (`kws-train`) lands in a follow-up — it pulls
in TensorFlow and is best left until you have recordings + compute
to drive a real training run.

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

## Evaluating a trained model

`kws-eval` loads a `.tflite` and runs it against a WAV — handy for
tuning `wake_word_threshold` without reflashing. The host's
mel-spectrogram frontend mirrors the firmware's
`stackchan-audio-features` crate bit-for-bit, so a host score
matches what the device computes on the same audio.

Install the eval extra first (brings in the TFLite runtime):

```bash
uv sync --extra eval
```

Then:

```bash
# Capture a representative clip via kws-record, or use an existing WAV
uv run kws-eval --model wakeword.tflite --input samples/positive-001.wav

# Verbose timeline (per-frame scores):
uv run kws-eval --model wakeword.tflite --input samples/positive-001.wav \
                --print-scores
```

Output:

```
clip: positive-001.wav  duration: 5.00s  frames: 481
peak score: 0.9847 at frame 122 (threshold 0.50)
triggered: YES
```

Exit code is `0` when the peak crosses `--threshold` (default 0.5)
and `1` when it doesn't — convenient for scripting threshold sweeps.
The peak score and frame index are stable across hosts because the
DSP is deterministic; small mel-scale rounding can shift the peak
bin by ±1 vs the device, but the score itself agrees.

## What's next

- `kws-train` — wrap `microwakeword`'s training pipeline, emit a
  ready-to-flash `.tflite`. Pulls in TensorFlow + the
  microwakeword library; lands once there's a recording set to
  drive a real training pass against.
