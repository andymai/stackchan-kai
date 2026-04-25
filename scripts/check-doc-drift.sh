#!/usr/bin/env bash
# check-doc-drift.sh — flag crates whose src/ has diverged from their README.md.
#
# For each crate under crates/, compares:
#   - the latest commit touching src/
#   - the latest commit touching README.md
# If src/ has > $DRIFT_THRESHOLD commits since the last README touch,
# emits a warning. Exits 0 (warn-only) by default; pass `--strict` to
# exit non-zero so CI can fail on drift.
#
# Tunable knobs:
#   DRIFT_THRESHOLD — default 30 commits. Above this, we consider the
#                     README plausibly stale.
#
# Catches the failure mode CLAUDE.md exhibited (drift from "six crates"
# at v0.1.0 → 19 crates at v0.9.7) without anyone noticing.
set -euo pipefail

: "${DRIFT_THRESHOLD:=30}"
strict=false
[ "${1:-}" = "--strict" ] && strict=true

# Output styling — color when interactive.
if [ -t 1 ]; then
    Y=$'\033[33m'; G=$'\033[32m'; D=$'\033[2m'; N=$'\033[0m'
else
    Y=""; G=""; D=""; N=""
fi

drifted=0
clean=0
no_readme=0

for crate_dir in crates/*/; do
    crate=$(basename "$crate_dir")

    if [ ! -f "${crate_dir}README.md" ]; then
        no_readme=$((no_readme + 1))
        continue
    fi
    if [ ! -d "${crate_dir}src" ]; then
        continue
    fi

    # `git log -1 -- <path>` returns the commit hash of the most recent
    # commit affecting that path. If the README has never been touched
    # (rare) the rev is empty; we treat that as "huge drift".
    src_rev=$(git log -1 --format=%H -- "${crate_dir}src" 2>/dev/null || true)
    readme_rev=$(git log -1 --format=%H -- "${crate_dir}README.md" 2>/dev/null || true)

    if [ -z "$src_rev" ]; then
        continue
    fi

    if [ -z "$readme_rev" ]; then
        printf "  %s%s%s — README never touched (always drifted)\n" "$Y" "$crate" "$N"
        drifted=$((drifted + 1))
        continue
    fi

    # Count commits touching src/ between (readme_rev, src_rev]. If
    # readme_rev == src_rev (same commit changed both), drift is 0.
    if [ "$src_rev" = "$readme_rev" ]; then
        clean=$((clean + 1))
        continue
    fi

    drift=$(git rev-list --count "${readme_rev}..${src_rev}" -- "${crate_dir}src" 2>/dev/null || echo 0)

    if [ "$drift" -gt "$DRIFT_THRESHOLD" ]; then
        printf "  %s%s%s — %d src commits since README touch (threshold: %d)\n" \
            "$Y" "$crate" "$N" "$drift" "$DRIFT_THRESHOLD"
        drifted=$((drifted + 1))
    else
        clean=$((clean + 1))
    fi
done

echo
printf "doc-drift: %s%d clean%s, %s%d drifted%s, %s%d missing README%s\n" \
    "$G" "$clean" "$N" \
    "$Y" "$drifted" "$N" \
    "$D" "$no_readme" "$N"

if [ "$drifted" -gt 0 ] && [ "$strict" = true ]; then
    echo
    echo "doc-drift: --strict mode — exiting non-zero because $drifted crate(s) drifted." >&2
    exit 1
fi

exit 0
