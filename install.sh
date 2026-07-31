#!/usr/bin/env bash
# Install omp-lite: registers hooks in settings.json and copies example rules.
# Idempotent - re-running replaces omp-lite's own entries and touches nothing else.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
SETTINGS="$CONFIG_DIR/settings.json"
RULES_DIR="$CONFIG_DIR/rules"

command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }
command -v node >/dev/null || { echo "node is required" >&2; exit 1; }
[ -f "$SETTINGS" ] || { echo "no settings.json at $SETTINGS" >&2; exit 1; }

echo "==> validating current settings.json"
jq empty "$SETTINGS" || { echo "settings.json is not valid JSON; refusing to touch it" >&2; exit 1; }

echo "==> installing example rules to $RULES_DIR"
mkdir -p "$RULES_DIR"
for f in "$ROOT"/rules/*.md; do
  base="$(basename "$f")"
  if [ -e "$RULES_DIR/$base" ]; then
    echo "    skip (already exists, yours wins): $base"
  else
    cp "$f" "$RULES_DIR/$base"
    echo "    added: $base"
  fi
done

echo "==> registering hooks"
TMP="$(mktemp)"
LR="node \"$ROOT/hooks/lazy-rules.js\""
LRP="node \"$ROOT/hooks/lazy-rules-post.js\""
RD="node \"$ROOT/hooks/read-discipline.js\""
EDIT_TOOLS="Edit|Write|MultiEdit|NotebookEdit|Bash"

jq \
  --arg lr "$LR" --arg lrp "$LRP" --arg rd "$RD" --arg edit "$EDIT_TOOLS" '
  def strip: map(select(
    [.hooks[]?.command // ""] | map(test("omp-lite")) | any | not
  ));
  .hooks.PreToolUse  = ((.hooks.PreToolUse  // []) | strip)
    + [{matcher: $edit, hooks: [{type:"command", command:$lr,  timeout:5}]},
       {matcher: "Read", hooks: [{type:"command", command:$rd,  timeout:5}]}]
  | .hooks.PostToolUse = ((.hooks.PostToolUse // []) | strip)
    + [{matcher: $edit, hooks: [{type:"command", command:$lrp, timeout:5}]}]
  ' "$SETTINGS" > "$TMP"

if ! jq empty "$TMP" 2>/dev/null; then
  echo "generated settings.json is invalid; aborting without writing" >&2
  rm -f "$TMP"; exit 1
fi

# Sanity: the pre-existing hook entries must survive.
before=$(jq '[.hooks.PreToolUse[]?.hooks[]?.command] | map(select(test("omp-lite")|not)) | length' "$SETTINGS")
after=$(jq  '[.hooks.PreToolUse[]?.hooks[]?.command] | map(select(test("omp-lite")|not)) | length' "$TMP")
if [ "$before" != "$after" ]; then
  echo "refusing to write: would have changed $before pre-existing PreToolUse hooks to $after" >&2
  rm -f "$TMP"; exit 1
fi

cat "$TMP" > "$SETTINGS"
rm -f "$TMP"

echo "==> done"
echo "    PreToolUse : $EDIT_TOOLS -> lazy-rules"
echo "    PreToolUse : Read -> read-discipline"
echo "    PostToolUse: $EDIT_TOOLS -> lazy-rules-post"
echo
echo "Restart Claude Code (or start a new session) for hooks to take effect."
