#!/usr/bin/env bash
# Install omp-claudecode-port-project.
#
# Takes a timestamped, verifiable backup first, then registers hooks in
# settings.json and copies the example rules. Idempotent: re-running replaces
# only this plugin's own hook entries and never overwrites a rule you edited.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NAME="omp-claudecode-port-project"
CONFIG_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
SETTINGS="$CONFIG_DIR/settings.json"
RULES_DIR="$CONFIG_DIR/rules"

# Hook script basenames identify our entries. Matching on these rather than on
# the plugin or directory name means a future rename cannot orphan old entries.
HOOK_MATCH="lazy-rules|read-discipline"

# ------------------------------------------------------------ prerequisites
command -v jq >/dev/null || {
  echo "ERROR: 'jq' is required but not installed." >&2
  echo "  macOS:  brew install jq" >&2
  echo "  Debian: sudo apt install jq" >&2
  exit 1
}

# Pick the hook engine once, at install time, and bake absolute paths into the
# hook commands. Essentially all of a hook's cost is startup, so this is the only
# lever that matters: rust binary ~3ms, bun ~12ms, node ~25ms.
#
# Sets HOOK_ENGINE (rust|bun|node) and a hook_cmd() that builds a command line.
RUST_BIN="$ROOT/rust/target/release/omp-hooks"

pick_js_runtime() {
  if [ -n "${OMP_PORT_RUNTIME:-}" ]; then
    command -v "$OMP_PORT_RUNTIME" 2>/dev/null && return 0
    echo "ERROR: OMP_PORT_RUNTIME='$OMP_PORT_RUNTIME' is not on PATH." >&2
    exit 1
  fi
  command -v bun 2>/dev/null && return 0
  command -v node 2>/dev/null && return 0
  return 1
}

# OMP_PORT_ENGINE=js forces the JS hooks even when the binary is built.
if [ "${OMP_PORT_ENGINE:-}" != "js" ] && [ -x "$RUST_BIN" ] \
   && printf '' | "$RUST_BIN" lazy-rules >/dev/null 2>&1; then
  HOOK_ENGINE="rust"
  ENGINE_DESC="$RUST_BIN"
  hook_cmd() { printf '"%s" %s' "$RUST_BIN" "$1"; }
else
  RUNTIME="$(pick_js_runtime)" || {
    echo "ERROR: need 'bun' or 'node' on PATH (or a built Rust binary); found none." >&2
    echo "  macOS:  brew install oven-sh/bun/bun   (or: brew install node)" >&2
    echo "  Debian: curl -fsSL https://bun.sh/install | bash" >&2
    echo "  Rust:   bash build-rust.sh" >&2
    exit 1
  }
  # Refuse a runtime that cannot actually load the hooks, rather than discovering
  # it later as silently dead hooks.
  "$RUNTIME" -e "require('$ROOT/hooks/lib/rules.js')" >/dev/null 2>&1 || {
    echo "ERROR: '$RUNTIME' cannot load the hook modules. Refusing to install." >&2
    exit 1
  }
  HOOK_ENGINE="$(basename "$RUNTIME")"
  ENGINE_DESC="$RUNTIME"
  hook_cmd() { printf '"%s" "%s/hooks/%s.js"' "$RUNTIME" "$ROOT" "$1"; }

  if [ "$HOOK_ENGINE" = "node" ] && ! command -v bun >/dev/null; then
    echo "NOTE: using node (~25ms per hook). bun runs the same code in ~12ms:"
    echo "        curl -fsSL https://bun.sh/install | bash"
    echo "      Or build the Rust hooks (~3ms):  bash build-rust.sh"
    echo "      Then re-run this installer."
    echo
  elif [ "$HOOK_ENGINE" = "bun" ] && command -v cargo >/dev/null && [ ! -x "$RUST_BIN" ]; then
    echo "NOTE: using bun (~12ms per hook). The Rust hooks run in ~3ms:"
    echo "        bash build-rust.sh && bash install.sh"
    echo
  fi
fi

if ! command -v ast-grep >/dev/null; then
  echo "NOTE: ast-grep not found. The hooks work without it; only the 'codemod'"
  echo "      skill needs it. Install later with: brew install ast-grep"
  echo
fi

[ -f "$SETTINGS" ] || { echo "ERROR: no settings.json at $SETTINGS" >&2; exit 1; }
jq empty "$SETTINGS" 2>/dev/null || { echo "ERROR: $SETTINGS is not valid JSON; refusing to touch it." >&2; exit 1; }

# ------------------------------------------------------------------- backup
TS="$(date -u +%Y%m%dT%H%M%SZ)"
BK="$CONFIG_DIR/backups/$NAME-$TS"
mkdir -p "$BK/files"

sha() { shasum -a 256 "$1" 2>/dev/null | cut -d' ' -f1 || sha256sum "$1" | cut -d' ' -f1; }

{
  echo "# $NAME backup manifest"
  echo "# created: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# host: $(hostname)"
  echo
  echo "# MODIFIED = existed before; undo restores it"
  echo
} > "$BK/MANIFEST.txt"

for rel in settings.json settings.local.json CLAUDE.md plugins/installed_plugins.json plugins/known_marketplaces.json; do
  src="$CONFIG_DIR/$rel"
  # Manifest paths are relative to $HOME, which is what undo.sh expects.
  home_rel="${src#"$HOME"/}"
  if [ -f "$src" ]; then
    mkdir -p "$BK/files/$(dirname "$home_rel")"
    cp -p "$src" "$BK/files/$home_rel"
    printf 'MODIFIED %s %s\n' "$(sha "$src")" "$home_rel" >> "$BK/MANIFEST.txt"
  fi
done

if [ -d "$RULES_DIR" ]; then
  rules_rel="${RULES_DIR#"$HOME"/}"
  mkdir -p "$BK/files/$(dirname "$rules_rel")"
  cp -R "$RULES_DIR" "$BK/files/$rules_rel"
  printf 'MODIFIED dir %s\n' "$rules_rel" >> "$BK/MANIFEST.txt"
fi

{
  echo
  echo "# CREATED = did not exist before; undo deletes it"
  echo
} >> "$BK/MANIFEST.txt"
for abs in "$RULES_DIR" "$CONFIG_DIR/state/$NAME"; do
  [ -e "$abs" ] || printf 'CREATED  %s\n' "${abs#"$HOME"/}" >> "$BK/MANIFEST.txt"
done

cp "$ROOT/lib/undo.sh" "$BK/undo.sh"
chmod +x "$BK/undo.sh"
echo "==> backup: $BK"

# -------------------------------------------------------------------- rules
mkdir -p "$RULES_DIR"
for f in "$ROOT"/rules/*.md; do
  base="$(basename "$f")"
  if [ -e "$RULES_DIR/$base" ]; then
    echo "    rule kept (yours wins): $base"
  else
    cp "$f" "$RULES_DIR/$base"
    echo "    rule added: $base"
  fi
done

# -------------------------------------------------------------------- hooks
TMP="$(mktemp)"
LR="$(hook_cmd lazy-rules)"
LRP="$(hook_cmd lazy-rules-post)"
RD="$(hook_cmd read-discipline)"
EDIT_TOOLS="Edit|Write|MultiEdit|NotebookEdit|Bash"

jq --arg lr "$LR" --arg lrp "$LRP" --arg rd "$RD" \
   --arg edit "$EDIT_TOOLS" --arg m "$HOOK_MATCH" '
  def strip: map(select([.hooks[]?.command // ""] | map(test($m)) | any | not));
  .hooks = (.hooks // {})
  | .hooks.PreToolUse  = ((.hooks.PreToolUse  // []) | strip)
    + [{matcher: $edit,  hooks: [{type:"command", command:$lr,  timeout:5}]},
       {matcher: "Read", hooks: [{type:"command", command:$rd,  timeout:5}]}]
  | .hooks.PostToolUse = ((.hooks.PostToolUse // []) | strip)
    + [{matcher: $edit,  hooks: [{type:"command", command:$lrp, timeout:5}]}]
  ' "$SETTINGS" > "$TMP"

jq empty "$TMP" 2>/dev/null || { echo "ERROR: generated settings.json is invalid; nothing written." >&2; rm -f "$TMP"; exit 1; }

# Every pre-existing hook, on every event, must survive untouched.
count_foreign() {
  jq --arg m "$HOOK_MATCH" '[.hooks[]?[]?.hooks[]?.command] | map(select(test($m)|not)) | length' "$1"
}
before="$(count_foreign "$SETTINGS")"
after="$(count_foreign "$TMP")"
if [ "$before" != "$after" ]; then
  echo "ERROR: refusing to write - would change $before pre-existing hooks to $after." >&2
  rm -f "$TMP"; exit 1
fi

cat "$TMP" > "$SETTINGS"
rm -f "$TMP"
echo "    hooks registered ($before pre-existing hooks preserved)"
echo "    engine : $HOOK_ENGINE ($ENGINE_DESC)"

# --------------------------------------------------------------------- done
cat <<EOF

==> installed

    PreToolUse   $EDIT_TOOLS -> lazy-rules
    PreToolUse   Read -> read-discipline
    PostToolUse  $EDIT_TOOLS -> lazy-rules-post

    Restart Claude Code (or open a new session) for hooks to load.

    Verify:  node "$ROOT/test/run.js"
    Undo:    bash "$BK/undo.sh"           # dry run
             bash "$BK/undo.sh" --apply
EOF
