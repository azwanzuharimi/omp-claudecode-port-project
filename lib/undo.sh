#!/usr/bin/env bash
# Undo an omp-claudecode-port-project installation.
# Generic: reads MANIFEST.txt from its own directory. Copied into each backup.
# Dry-run by default. Pass --apply to actually change anything.
set -uo pipefail

BK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MANIFEST="$BK/MANIFEST.txt"
APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1

[ -f "$MANIFEST" ] || { echo "no MANIFEST.txt in $BK" >&2; exit 1; }

if [ "$APPLY" -eq 0 ]; then
  echo "DRY RUN - nothing will change. Re-run with --apply to execute."
  echo
fi
echo "snapshot: $BK"
echo

restored=0; deleted=0; skipped=0

while read -r kind hash path; do
  case "$kind" in
    MODIFIED)
      # Manifest paths are absolute; the snapshot mirrors them under files/.
      [ "$hash" = "dir" ] && { src="$BK/files$path"; dst="$path"
        [ -d "$src" ] || continue
        echo "restore dir  $path"
        [ "$APPLY" -eq 1 ] && { rm -rf "$dst"; cp -R "$src" "$dst"; }
        restored=$((restored+1)); continue; }
      src="$BK/files$path"; dst="$path"
      if [ ! -f "$src" ]; then echo "MISSING from snapshot, skipping: $path"; skipped=$((skipped+1)); continue; fi
      actual="$(shasum -a 256 "$src" | cut -d' ' -f1)"
      if [ "$actual" != "$hash" ]; then
        echo "REFUSING $path - snapshot copy is corrupt (sha mismatch)"; skipped=$((skipped+1)); continue
      fi
      if [ -f "$dst" ] && [ "$(shasum -a 256 "$dst" | cut -d' ' -f1)" = "$hash" ]; then
        echo "unchanged    $path"; continue
      fi
      echo "restore      $path"
      [ "$APPLY" -eq 1 ] && cp -p "$src" "$dst"
      restored=$((restored+1))
      ;;
    CREATED)
      # For CREATED rows the manifest has only two fields, so $hash holds the path.
      p="$hash"; dst="$p"
      [ -e "$dst" ] || continue
      echo "delete       $p"
      [ "$APPLY" -eq 1 ] && rm -rf "$dst"
      deleted=$((deleted+1))
      ;;
  esac
done < <(grep -E '^(MODIFIED|CREATED)' "$MANIFEST")

echo
echo "restored: $restored   deleted: $deleted   skipped: $skipped"
echo
echo "NOT touched by this script:"
echo "  - ast-grep (brew package). Standalone tool, harmless to keep."
echo "    Remove yourself with: brew uninstall ast-grep"
if [ "$APPLY" -eq 0 ]; then
  echo
  echo "This was a DRY RUN. Nothing changed. Run: $0 --apply"
fi
