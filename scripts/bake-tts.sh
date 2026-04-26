#!/usr/bin/env bash
#
# bake-tts.sh — regenerate verbal-phrase PCM from the manifest.
#
# Reads `crates/stackchan-tts/assets/manifest.toml` and writes raw
# 16 kHz / 16-bit / mono / little-endian PCM to
# `crates/stackchan-tts/assets/<locale>/<phrase_id>.pcm`.
#
# Pipeline: Piper synthesises 22050 Hz / 16-bit / mono raw PCM on
# stdout; sox resamples to 16 kHz. No WAV header at any step —
# matches the firmware audio task's I²S format so playback is
# decode-free.
#
# Voice models live under $PIPER_VOICE_DIR (defaults to
# ~/.local/share/piper). Locale → model mapping is at the top of
# this script — extend when adding locales.
#
# Usage:
#   scripts/bake-tts.sh                      # all locales × all phrases
#   scripts/bake-tts.sh en greeting          # one specific entry
#
# Prereqs (errors out with install hints if missing):
#   - piper  https://github.com/rhasspy/piper
#   - sox    apt install sox / brew install sox
#   - python3 (toml parsing via stdlib `tomllib`)

set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
ASSETS="$ROOT/crates/stackchan-tts/assets"
MANIFEST="$ASSETS/manifest.toml"
PIPER_VOICE_DIR="${PIPER_VOICE_DIR:-$HOME/.local/share/piper}"

# Locale → Piper voice model name. Extend as locales are added.
declare -A VOICE_FOR=(
    [en]="en_US-amy-medium"
    [ja]="ja_JP-test-medium"
)

# ----- prereq checks -----
command -v piper >/dev/null 2>&1 || {
    echo "bake-tts: 'piper' not on PATH." >&2
    echo "  install: https://github.com/rhasspy/piper" >&2
    exit 1
}
command -v sox >/dev/null 2>&1 || {
    echo "bake-tts: 'sox' not on PATH." >&2
    echo "  install: apt install sox  (Linux) / brew install sox  (macOS)" >&2
    exit 1
}
command -v python3 >/dev/null 2>&1 || {
    echo "bake-tts: 'python3' required for manifest parsing." >&2
    exit 1
}

# Parse manifest into "<locale>\t<phrase>\t<text>" lines.
parse_manifest() {
    python3 - "$MANIFEST" <<'PY'
import sys
import tomllib
with open(sys.argv[1], "rb") as f:
    data = tomllib.load(f)
for phrase, locales in data.items():
    if not isinstance(locales, dict):
        continue
    for locale, text in locales.items():
        print(f"{locale}\t{phrase}\t{text}")
PY
}

# Render one entry. `$1` = locale, `$2` = phrase_id, `$3` = text.
bake_one() {
    local locale="$1" phrase="$2" text="$3"
    local voice="${VOICE_FOR[$locale]:-}"
    if [[ -z "$voice" ]]; then
        echo "bake-tts: no Piper voice configured for locale '$locale'" >&2
        return 1
    fi
    local model_path="$PIPER_VOICE_DIR/$voice.onnx"
    if [[ ! -f "$model_path" ]]; then
        echo "bake-tts: voice model missing: $model_path" >&2
        echo "  download from https://github.com/rhasspy/piper/blob/master/VOICES.md" >&2
        return 1
    fi
    local out_dir="$ASSETS/$locale"
    local out_path="$out_dir/$phrase.pcm"
    mkdir -p "$out_dir"

    # Piper: text on stdin, raw 22050 Hz / 16-bit / mono on stdout.
    # sox: resample to 16 kHz, same bit-depth/channels, raw out.
    printf '%s' "$text" | piper \
        --model "$model_path" \
        --output_raw \
      | sox -t raw -r 22050 -e signed -b 16 -c 1 - \
            -t raw -r 16000 -e signed -b 16 -c 1 "$out_path"

    local size
    size=$(stat -c %s "$out_path" 2>/dev/null || stat -f %z "$out_path")
    local samples=$((size / 2))
    local ms=$((samples * 1000 / 16000))
    printf "  %-8s  %-20s  %5d ms  %6d bytes  %s\n" \
        "$locale" "$phrase" "$ms" "$size" "$out_path"
}

# ----- main -----
echo "bake-tts: reading $MANIFEST"

if [[ $# -eq 2 ]]; then
    # Single-entry mode: locale + phrase.
    locale="$1"
    phrase="$2"
    line=$(parse_manifest | awk -F'\t' -v l="$locale" -v p="$phrase" \
        '$1 == l && $2 == p { print; exit }')
    if [[ -z "$line" ]]; then
        echo "bake-tts: no manifest entry for $locale/$phrase" >&2
        exit 1
    fi
    IFS=$'\t' read -r _l _p text <<<"$line"
    bake_one "$locale" "$phrase" "$text"
elif [[ $# -eq 0 ]]; then
    # Full bake.
    while IFS=$'\t' read -r locale phrase text; do
        bake_one "$locale" "$phrase" "$text"
    done < <(parse_manifest)
else
    echo "usage: $0 [<locale> <phrase>]" >&2
    exit 1
fi

echo "bake-tts: done. Commit the regenerated .pcm files alongside any manifest changes."
