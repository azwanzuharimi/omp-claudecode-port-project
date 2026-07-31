#!/usr/bin/env bash
# Convenience wrapper: finds the newest backup this plugin created and runs its
# undo script. Dry-run by default; pass --apply to execute.
set -euo pipefail

NAME="omp-claudecode-port-project"
CONFIG_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"

newest="$(ls -d "$CONFIG_DIR/backups/$NAME-"*/ 2>/dev/null | sort | tail -1 || true)"
if [ -z "$newest" ]; then
  echo "No backup found under $CONFIG_DIR/backups/$NAME-*" >&2
  echo "Nothing to undo - was this ever installed on this machine?" >&2
  exit 1
fi

echo "Using backup: $newest"
exec bash "$newest/undo.sh" "$@"
