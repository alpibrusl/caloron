#!/bin/bash
# ==========================================================================
# sandbox-agent.sh — Run a command in a filesystem-sandboxed environment
#
# Usage: sandbox-agent.sh <worktree-path> <command> [args...]
#
# The command runs with:
#   - Read-write access to the worktree only
#   - Read-only access to system libs, Nix store, and Claude config
#   - Network access (needed for Claude API)
#   - No access to any other host directories
#
# Requires: bubblewrap (bwrap)
# ==========================================================================
set -euo pipefail

WORKTREE="${1:?Usage: sandbox-agent.sh <worktree-path> <command> [args...]}"
shift
COMMAND=("$@")

if [ ${#COMMAND[@]} -eq 0 ]; then
    echo "Usage: sandbox-agent.sh <worktree-path> <command> [args...]"
    exit 1
fi

# Resolve absolute path
WORKTREE="$(cd "$WORKTREE" && pwd)"
HOME_DIR="${HOME:-/home/$(whoami)}"

# Build bwrap args
BWRAP_ARGS=(
    # System (read-only)
    --ro-bind /nix /nix
    --ro-bind /usr /usr
    --ro-bind /lib /lib
    --ro-bind /etc /etc
    --dev /dev
    --proc /proc

    # lib64 may not exist on all systems
    $([ -d /lib64 ] && echo "--ro-bind /lib64 /lib64")

    # Claude Code needs its config (read-only)
    --ro-bind "$HOME_DIR/.local" "$HOME_DIR/.local"
    --ro-bind "$HOME_DIR/.config" "$HOME_DIR/.config"
    --ro-bind "$HOME_DIR/.claude" "$HOME_DIR/.claude"
    $([ -d "$HOME_DIR/.claude.json" ] && echo "--ro-bind $HOME_DIR/.claude.json $HOME_DIR/.claude.json" || true)
    $([ -d "$HOME_DIR/snap" ] && echo "--ro-bind $HOME_DIR/snap $HOME_DIR/snap" || true)
    $([ -d "$HOME_DIR/.npm" ] && echo "--ro-bind $HOME_DIR/.npm $HOME_DIR/.npm" || true)

    # Worktree (read-write) — the ONLY writable directory
    --bind "$WORKTREE" "$WORKTREE"

    # Temp (read-write, needed by many tools)
    --bind /tmp /tmp

    # Runtime (needed for sockets, dbus, etc.)
    --bind /run /run

    # Namespaces
    --unshare-pid          # agent can't see/kill host processes
    --unshare-uts          # isolated hostname
    --share-net            # network needed for Claude API

    # Environment
    --setenv HOME "$HOME_DIR"
    --setenv PATH "$HOME_DIR/.local/bin:/nix/var/nix/profiles/default/bin:/usr/bin:/bin"
    --setenv CALORON_SANDBOXED "1"

    # Working directory
    --chdir "$WORKTREE"
)

exec bwrap "${BWRAP_ARGS[@]}" -- "${COMMAND[@]}"
