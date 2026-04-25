#!/usr/bin/env bash
# dev-status.sh — session-start sanity check.
#
# Prints toolchain availability, device enumeration, working-tree state,
# and recent CI status so the contributor (human or AI) sees the lay of
# the land at the start of a session. Read-only; no side effects.
set -uo pipefail

# Color output if stdout is a TTY, plain otherwise (so logs stay clean).
if [ -t 1 ]; then
    G=$'\033[32m'; Y=$'\033[33m'; R=$'\033[31m'; D=$'\033[2m'; N=$'\033[0m'
else
    G=""; Y=""; R=""; D=""; N=""
fi

ok()    { echo "  ${G}✓${N} $*"; }
warn()  { echo "  ${Y}!${N} $*"; }
miss()  { echo "  ${R}✗${N} $*"; }
note()  { echo "  ${D}·${N} $*"; }

echo
echo "stackchan-kai — dev session status"
echo

# ----- Toolchain ----------------------------------------------------------
echo "Toolchain:"
for cmd in cargo just tmux; do
    if command -v "$cmd" >/dev/null 2>&1; then
        ok "$cmd ($(command -v $cmd))"
    else
        miss "$cmd not on PATH"
    fi
done
if command -v espflash >/dev/null 2>&1; then
    ok "espflash ($(espflash --version 2>/dev/null | head -1))"
else
    warn "espflash not on PATH (cargo install espflash)"
fi
if [ -f "$HOME/export-esp.sh" ]; then
    ok "esp toolchain available (source ~/export-esp.sh)"
else
    miss "no ~/export-esp.sh — esp toolchain not installed (espup install)"
fi

# ----- Device -------------------------------------------------------------
echo
echo "Device:"
acm_devices=( /dev/ttyACM* )
if [ -e "${acm_devices[0]}" ]; then
    for dev in "${acm_devices[@]}"; do
        # `udevadm info` works rootless and identifies which port is the CoreS3.
        descr=$(udevadm info "$dev" 2>/dev/null | grep -E "ID_VENDOR_FROM_DATABASE|ID_MODEL_FROM_DATABASE" | head -2 | tr '\n' ' ' | sed 's/E: //g')
        ok "$dev — ${descr:-(unknown)}"
    done
else
    warn "no /dev/ttyACM* — connect the CoreS3 over USB"
fi

# Dialout group check (Linux only).
if [ "$(uname)" = "Linux" ]; then
    if id -nG "$USER" 2>/dev/null | tr ' ' '\n' | grep -qx dialout; then
        ok "user '$USER' is in 'dialout' group"
    else
        warn "user '$USER' not in 'dialout' group — wrap espflash via 'sg dialout'"
    fi
fi

# ----- Repo -----------------------------------------------------------------
echo
echo "Repo:"
branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "(detached)")
ok "branch: $branch"
if git diff-index --quiet HEAD -- 2>/dev/null; then
    ok "working tree clean"
else
    note "working tree has unstaged changes:"
    git status -s | head -10 | sed 's/^/      /'
fi

worktree_count=$(git worktree list 2>/dev/null | wc -l)
if [ "$worktree_count" -gt 1 ]; then
    note "$((worktree_count - 1)) extra worktree(s) live in .worktrees/"
fi

# Behind / ahead of upstream, if upstream exists.
if upstream=$(git rev-parse --abbrev-ref --symbolic-full-name @{upstream} 2>/dev/null); then
    behind=$(git rev-list --count "HEAD..$upstream" 2>/dev/null || echo 0)
    ahead=$(git rev-list --count "$upstream..HEAD" 2>/dev/null || echo 0)
    if [ "$behind" -gt 0 ] || [ "$ahead" -gt 0 ]; then
        note "vs $upstream: $ahead ahead, $behind behind"
    fi
fi

# ----- Recent commits -----------------------------------------------------
echo
echo "Recent commits on $branch:"
git log --oneline -5 2>/dev/null | sed 's/^/  /'

# ----- Open PRs ------------------------------------------------------------
if command -v gh >/dev/null 2>&1; then
    echo
    echo "Open PRs:"
    if gh pr list --state open --limit 5 --json number,title 2>/dev/null \
        | grep -q '"number"'; then
        gh pr list --state open --limit 5 2>/dev/null | sed 's/^/  /'
    else
        ok "no open PRs"
    fi
fi

echo
