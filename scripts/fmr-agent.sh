#!/usr/bin/env bash
# fmr-agent.sh — flash + monitor in tmux + tee log + poll for boot complete.
#
# Wraps the agent-friendly flashing dance (CLAUDE.md "Flashing from an
# agent / non-TTY shell"). Returns 0 if "boot complete" appeared in the
# log within $BOOT_TIMEOUT_S, non-zero otherwise.
#
# Usage:
#   just fmr-agent
#
# Reads:
#   BOOT_TIMEOUT_S   — wall-clock seconds before giving up (default: 90)
#   SCFMR_LOG        — log path (default: /tmp/scfmr.log)
#   SCFMR_SESSION    — tmux session name (default: scfmr)
#
# After this returns, the device keeps running and the tmux session
# stays attached. Use `tmux send-keys -t <session> C-c` before re-running
# to break out of the monitor cleanly.
set -euo pipefail

: "${BOOT_TIMEOUT_S:=90}"
: "${SCFMR_LOG:=/tmp/scfmr.log}"
: "${SCFMR_SESSION:=scfmr}"

# Resolve the worktree root we were invoked from. `just` invokes recipes
# from the project root, so $PWD is the right anchor.
WORKTREE_ROOT="$PWD"

# Reset any prior monitor cleanly. `tmux send-keys C-c` is idempotent
# when no session exists; the kill ensures a fresh start so log positions
# don't compound.
tmux kill-session -t "$SCFMR_SESSION" 2>/dev/null || true

# Truncate the log so a partial prior run doesn't trip the boot-complete
# poll on the next invocation.
: > "$SCFMR_LOG"

tmux new-session -d -s "$SCFMR_SESSION" -c "$WORKTREE_ROOT" 'bash -l'
tmux send-keys -t "$SCFMR_SESSION" \
    "source ~/export-esp.sh && just fmr 2>&1 | tee $SCFMR_LOG" Enter

echo "fmr-agent: tmux session '$SCFMR_SESSION' started, build+flash in progress..."
echo "fmr-agent: tailing $SCFMR_LOG (timeout ${BOOT_TIMEOUT_S}s for 'boot complete')"

# Poll for boot-complete with a deadline. `until grep -q ...; do sleep`
# is the obvious shape but blocks indefinitely — we need a deadline so a
# panicking firmware doesn't wedge the recipe.
deadline=$(( $(date +%s) + BOOT_TIMEOUT_S ))
while true; do
    if grep -q "boot complete" "$SCFMR_LOG" 2>/dev/null; then
        break
    fi
    if grep -qiE "^.*(panic|ERROR.*panicked)" "$SCFMR_LOG" 2>/dev/null; then
        echo "fmr-agent: PANIC detected in log:"
        grep -iE "panic|ERROR" "$SCFMR_LOG" | head -20
        exit 2
    fi
    if [ "$(date +%s)" -ge "$deadline" ]; then
        echo "fmr-agent: TIMEOUT (${BOOT_TIMEOUT_S}s) — last 20 log lines:"
        tail -20 "$SCFMR_LOG"
        exit 3
    fi
    sleep 2
done

# Boot succeeded — print the canonical boot summary so the agent can
# confirm at a glance without reading the full log.
echo "fmr-agent: BOOT COMPLETE"
grep -E "stackchan-firmware v|task:|present|ready|boot complete|panic|ERROR" "$SCFMR_LOG" | head -25
