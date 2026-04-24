#!/usr/bin/env bash
# check-readme-reminders.sh - Remind to review READMEs when Rust source changes
#
# Walks up from each staged .rs file's directory looking for README.md on disk.
# Zero-config: adding a README anywhere (crates/<name>/README.md, root README,
# etc.) automatically joins the reminder list.
#
# Non-blocking: always exits 0. This is a reminder for humans and LLMs
# committing changes — not a gate.

# ERR trap ensures this script never blocks a commit, even on unexpected
# failures.
trap 'exit 0' ERR

STAGED=$(git diff --cached --name-only --diff-filter=ACMR || true)
[ -z "$STAGED" ] && exit 0

declare -A STAGED_READMES DIR_FILE_COUNTS
HAS_SOURCE=false

while IFS= read -r FILE; do
    [ -z "$FILE" ] && continue
    case "$FILE" in */README.md|README.md) STAGED_READMES["$FILE"]=1; continue ;; esac
    case "$FILE" in *.rs) ;; *) continue ;; esac
    # Skip obvious test files — changes there rarely affect the architecture
    # that a README describes.
    case "$FILE" in
        */tests/*|*_test.rs|*_tests.rs|*/benches/*) continue ;;
    esac

    DIR="${FILE%/*}"
    [ "$DIR" = "$FILE" ] && DIR="."
    DIR_FILE_COUNTS["$DIR"]=$(( ${DIR_FILE_COUNTS["$DIR"]:-0} + 1 ))
    HAS_SOURCE=true
done <<< "$STAGED"

$HAS_SOURCE || exit 0

# Walk up from each directory, collecting READMEs on disk.
# WALKED cache: if a deeper dir already walked through this ancestor, skip it.
declare -A FOUND_READMES WALKED
for DIR in "${!DIR_FILE_COUNTS[@]}"; do
    CURRENT="$DIR"
    while true; do
        [ "${WALKED[$CURRENT]+_}" ] && break
        WALKED["$CURRENT"]=1
        [ -f "${CURRENT}/README.md" ] && FOUND_READMES["${CURRENT}/README.md"]=1
        PARENT="${CURRENT%/*}"
        [ "$PARENT" = "$CURRENT" ] && break
        CURRENT="$PARENT"
    done
done

[ ${#FOUND_READMES[@]} -eq 0 ] && exit 0

readarray -t SORTED < <(printf '%s\n' "${!FOUND_READMES[@]}" | sort)

LINES=()
for README_PATH in "${SORTED[@]}"; do
    [ "${STAGED_READMES[$README_PATH]+_}" ] && continue

    README_DIR="${README_PATH%/*}"
    FILE_COUNT=0
    for SDIR in "${!DIR_FILE_COUNTS[@]}"; do
        case "$SDIR" in "$README_DIR"|"$README_DIR"/*) FILE_COUNT=$(( FILE_COUNT + DIR_FILE_COUNTS["$SDIR"] )) ;; esac
    done

    NAME="${README_DIR##*/}"
    [ "$NAME" = "." ] && NAME="<root>"
    SUFFIX="s"; [ "$FILE_COUNT" -eq 1 ] && SUFFIX=""
    LABEL="${NAME} (${FILE_COUNT} file${SUFFIX})"
    PAD=$(( 28 - ${#LABEL} ))
    [ "$PAD" -lt 1 ] && PAD=1
    LINES+=("  ${LABEL}$(printf '%*s' "$PAD" '')→ ${README_PATH}")
done

if [ ${#LINES[@]} -gt 0 ]; then
    printf '\nREADME review reminder:\n'
    printf '%s\n' "${LINES[@]}"
    printf '  (Review these READMEs if your changes affect architecture, key files, or gotchas)\n\n'
fi

exit 0
