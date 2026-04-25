#!/usr/bin/env bash
# check-boot.sh — diff live boot log against tests/golden/boot.txt.
#
# Reports:
#   1. Golden lines that didn't appear in the live log (regression!)
#   2. Unexpected ERROR or WARN lines (new noise!)
#
# Known-noise (DmaError(Late), BMI270 retry, BMM150 not reachable, etc.)
# is filtered before the WARN/ERROR scan — these are documented in
# AGENTS.md as expected for this unit.
#
# Usage:
#   just verify-boot          # check /tmp/scfmr.log
#   just verify-boot --update # rewrite the golden from /tmp/scfmr.log
set -euo pipefail

GOLDEN="tests/golden/boot.txt"
LIVE="${SCFMR_LOG:-/tmp/scfmr.log}"

# Color when interactive.
if [ -t 1 ]; then
    G=$'\033[32m'; Y=$'\033[33m'; R=$'\033[31m'; D=$'\033[2m'; N=$'\033[0m'
else
    G=""; Y=""; R=""; D=""; N=""
fi

if [ "${1:-}" = "--update" ]; then
    echo "${Y}check-boot:${N} --update mode rebuilds the golden file from $LIVE."
    echo "Review the diff and commit the result."
    echo
    if [ ! -f "$LIVE" ]; then
        echo "${R}check-boot:${N} no live log at $LIVE — flash first (just fmr-agent)" >&2
        exit 1
    fi
    # Extract every "info" line from the live log, strip the timestamp
    # prefix and source-location suffix, dedupe, sort by first
    # appearance.
    grep "INFO" "$LIVE" \
        | sed -E 's/^[0-9]+ ms \[INFO \] //; s/ \(stackchan_firmware [^)]+\)$//' \
        | awk '!seen[$0]++' \
        > "$GOLDEN.candidate"
    echo "${G}check-boot:${N} candidate written to $GOLDEN.candidate. Review and:"
    echo "  diff $GOLDEN $GOLDEN.candidate"
    echo "  mv $GOLDEN.candidate $GOLDEN  # if you accept the new state"
    exit 0
fi

if [ ! -f "$GOLDEN" ]; then
    echo "${R}check-boot:${N} golden file missing at $GOLDEN" >&2
    exit 1
fi
if [ ! -f "$LIVE" ]; then
    echo "${R}check-boot:${N} live log missing at $LIVE — flash first (just fmr-agent)" >&2
    exit 1
fi

# Phase 1: golden lines that didn't appear.
missing=0
echo "Checking for missing golden lines:"
while IFS= read -r line; do
    case "$line" in ''|'#'*) continue ;; esac
    if ! grep -qF "$line" "$LIVE"; then
        printf "  %s✗%s missing: %s\n" "$R" "$N" "$line"
        missing=$((missing + 1))
    fi
done < "$GOLDEN"
if [ "$missing" -eq 0 ]; then
    printf "  %s✓%s all golden lines present\n" "$G" "$N"
fi

# Phase 2: unexpected WARN / ERROR lines (after filtering known noise).
echo
echo "Checking for unexpected WARN/ERROR lines:"
unexpected=$(grep -E "\[(WARN|ERROR)\]" "$LIVE" \
    | grep -vE "DmaError\(Late\)" \
    | grep -vE "BMI270: init attempt [0-9]+/3 failed" \
    | grep -vE "BMM150: not reachable on main I" \
    | grep -vE "BMM150: NotDetected" || true)

if [ -z "$unexpected" ]; then
    printf "  %s✓%s no unexpected WARN/ERROR (after known-noise filter)\n" "$G" "$N"
else
    echo "$unexpected" | head -20 | sed "s/^/  ${Y}!${N} /"
    unexpected_count=$(echo "$unexpected" | wc -l)
    printf "  %s%d unexpected line(s) total%s\n" "$Y" "$unexpected_count" "$N"
fi

echo
if [ "$missing" -gt 0 ]; then
    printf "%scheck-boot: FAIL — %d missing golden line(s).%s\n" "$R" "$missing" "$N" >&2
    printf "%s             update the golden if the change is intentional: just verify-boot --update%s\n" "$D" "$N" >&2
    exit 1
fi
printf "%scheck-boot: PASS%s\n" "$G" "$N"
