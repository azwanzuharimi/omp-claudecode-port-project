#!/usr/bin/env bash
# Remove omp-claudecode-port-project from this machine.
#
# Default mode is surgical: strip only this plugin's hook entries out of
# settings.json, drop the rules it installed (keeping any you edited), and delete
# its state. Everything else in settings.json is left byte-for-byte alone.
#
# Why not just restore the backup? Because after a second install the newest
# snapshot was taken while the plugin was ALREADY installed, so restoring it
# would leave the plugin in place. Surgical removal is correct however many times
# you have installed.
#
#   bash uninstall.sh                      dry run (default)
#   bash uninstall.sh --apply              remove
#   bash uninstall.sh --restore-snapshot   instead: roll config back to a snapshot
set -euo pipefail

NAME="omp-claudecode-port-project"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
SETTINGS="$CONFIG_DIR/settings.json"
RULES_DIR="$CONFIG_DIR/rules"
STATE_DIR="$CONFIG_DIR/state/$NAME"

# Same identifier install.sh uses: hook script names, not the plugin/dir name.
HOOK_MATCH="lazy-rules|read-discipline|omp-hooks"

APPLY=0
MODE="surgical"
for a in "$@"; do
  case "$a" in
    --apply) APPLY=1 ;;
    --restore-snapshot) MODE="snapshot" ;;
    -h|--help) sed -n '2,14p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done

say() { [ "$APPLY" -eq 1 ] && echo "  $*" || echo "  [dry-run] $*"; }

# --------------------------------------------------------------- snapshot mode
if [ "$MODE" = "snapshot" ]; then
  newest="$(ls -d "$CONFIG_DIR/backups/$NAME-"*/ 2>/dev/null | sort | tail -1 || true)"
  [ -n "$newest" ] || { echo "No snapshot found under $CONFIG_DIR/backups/$NAME-*" >&2; exit 1; }
  echo "Restoring snapshot: $newest"
  echo "NOTE: a snapshot taken during a re-install may still contain this plugin."
  echo "      Use the default surgical mode for a clean removal."
  echo
  [ "$APPLY" -eq 1 ] && exec bash "$newest/undo.sh" --apply
  exec bash "$newest/undo.sh"
fi

# --------------------------------------------------------------- surgical mode
command -v jq >/dev/null || { echo "ERROR: jq is required." >&2; exit 1; }
[ -f "$SETTINGS" ] || { echo "ERROR: no settings.json at $SETTINGS" >&2; exit 1; }
jq empty "$SETTINGS" 2>/dev/null || { echo "ERROR: $SETTINGS is not valid JSON; refusing to touch it." >&2; exit 1; }

[ "$APPLY" -eq 1 ] || echo "DRY RUN - nothing will change. Re-run with --apply."
echo
echo "config: $CONFIG_DIR"
echo

# 1. hooks -------------------------------------------------------------------
ours="$(jq -r --arg m "$HOOK_MATCH" \
  '[.hooks[]?[]?.hooks[]?.command] | map(select(test($m))) | length' "$SETTINGS")"
others="$(jq -r --arg m "$HOOK_MATCH" \
  '[.hooks[]?[]?.hooks[]?.command] | map(select(test($m)|not)) | length' "$SETTINGS")"

if [ "$ours" -eq 0 ]; then
  echo "hooks:  none of ours registered (nothing to remove)"
else
  say "remove $ours hook entr$([ "$ours" -eq 1 ] && echo y || echo ies) from settings.json"
  echo "        keeping $others hook(s) belonging to other tools"
  if [ "$APPLY" -eq 1 ]; then
    TMP="$(mktemp)"
    jq --arg m "$HOOK_MATCH" '
      def strip: map(select([.hooks[]?.command // ""] | map(test($m)) | any | not));
      if .hooks then .hooks |= with_entries(.value |= strip) else . end
      | if .hooks then .hooks |= with_entries(select(.value | length > 0)) else . end
    ' "$SETTINGS" > "$TMP"

    jq empty "$TMP" 2>/dev/null || { echo "ERROR: generated settings.json invalid; nothing written." >&2; rm -f "$TMP"; exit 1; }
    left="$(jq -r --arg m "$HOOK_MATCH" '[.hooks[]?[]?.hooks[]?.command] | map(select(test($m)|not)) | length' "$TMP")"
    if [ "$left" != "$others" ]; then
      echo "ERROR: refusing to write - other tools' hooks would go from $others to $left." >&2
      rm -f "$TMP"; exit 1
    fi
    cat "$TMP" > "$SETTINGS"; rm -f "$TMP"
  fi
fi

# 2. rules -------------------------------------------------------------------
# Only remove a rule that is still byte-identical to the one we shipped. Anything
# you edited is yours; it stays, and we say so.
kept_edited=0
if [ -d "$RULES_DIR" ]; then
  for f in "$ROOT"/rules/*.md; do
    base="$(basename "$f")"
    inst="$RULES_DIR/$base"
    [ -e "$inst" ] || continue
    if cmp -s "$f" "$inst"; then
      say "remove rule $base"
      [ "$APPLY" -eq 1 ] && rm -f "$inst"
    else
      echo "        KEEPING $base - you edited it (delete by hand if unwanted)"
      kept_edited=$((kept_edited+1))
    fi
  done
  if [ "$APPLY" -eq 1 ] && [ -d "$RULES_DIR" ] && [ -z "$(ls -A "$RULES_DIR" 2>/dev/null)" ]; then
    rmdir "$RULES_DIR" && echo "  removed empty $RULES_DIR"
  elif [ "$APPLY" -eq 0 ] && [ "$kept_edited" -eq 0 ]; then
    say "remove $RULES_DIR if it ends up empty"
  fi
else
  echo "rules:  $RULES_DIR does not exist"
fi

# 3. state -------------------------------------------------------------------
if [ -d "$STATE_DIR" ]; then
  say "delete $STATE_DIR"
  [ "$APPLY" -eq 1 ] && rm -rf "$STATE_DIR"
else
  echo "state:  none"
fi

# 4. what we deliberately leave behind ---------------------------------------
echo
echo "left in place (remove by hand if you want them gone):"
if [ -d "$ROOT/rust/target" ]; then
  echo "  - $ROOT/rust/target  ($(du -sh "$ROOT/rust/target" 2>/dev/null | cut -f1)) - Rust build cache"
  echo "      rm -rf \"$ROOT/rust/target\""
fi
echo "  - $ROOT  - this clone; delete it when you no longer need the tool"
if [ -d "$CONFIG_DIR/backups" ] && ls -d "$CONFIG_DIR/backups/$NAME-"*/ >/dev/null 2>&1; then
  n="$(ls -d "$CONFIG_DIR/backups/$NAME-"*/ | wc -l | tr -d ' ')"
  echo "  - $CONFIG_DIR/backups/$NAME-*  ($n snapshot(s)) - your rollback safety net"
fi
command -v ast-grep >/dev/null && echo "  - ast-grep - a standalone tool, not ours to remove"

echo
if [ "$APPLY" -eq 1 ]; then
  echo "Done. Restart Claude Code so it stops loading the hooks."
else
  echo "Nothing changed. Re-run with --apply to remove."
fi
