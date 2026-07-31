# /// script
# requires-python = ">=3.10"
# dependencies = ["tiktoken"]
# ///
"""Measure what read-discipline saves: full-file tokens vs outline tokens.

Deterministic - no API calls. Walks a real corpus, and for every file the hook
would actually intercept, compares the tokens of the whole file against the
tokens of the outline the hook returns instead.

Tokenizer is OpenAI o200k (tiktoken). It approximates Claude's tokenizer, so
ratios are meaningful and absolute counts are close but not exact - the same
caveat the caveman evals carry.

Usage: uv run bench/read_savings.py <dir> [<dir> ...]
"""

import json
import subprocess
import sys
from pathlib import Path

import tiktoken

ROOT = Path(__file__).resolve().parent.parent
ENC = tiktoken.get_encoding("o200k_base")
THRESHOLD = 400
SKIP_DIRS = {".git", "node_modules", ".venv", "venv", "dist", "build", "__pycache__", ".next"}

OUTLINE_JS = """
const o = require(process.argv[1] + '/hooks/lib/outline');
const fs = require('fs');
const p = process.argv[2];
const text = fs.readFileSync(p, 'utf8');
const { lines, rows } = o.outline(text, p);
process.stdout.write(JSON.stringify({
  lines, rows: rows.length,
  isSource: o.isSource(p),
  rendered: rows.length >= 3 ? o.render(p, lines, rows) : null,
}));
"""


def outline_for(path: Path):
    out = subprocess.run(
        ["node", "-e", OUTLINE_JS, str(ROOT), str(path)],
        capture_output=True, text=True,
    )
    if out.returncode != 0 or not out.stdout.strip():
        return None
    return json.loads(out.stdout)


def main(dirs):
    rows = []
    for d in dirs:
        for path in Path(d).rglob("*"):
            if not path.is_file() or any(p in SKIP_DIRS for p in path.parts):
                continue
            try:
                if path.stat().st_size > 4 * 1024 * 1024:
                    continue
                text = path.read_text(encoding="utf8")
            except Exception:
                continue

            info = outline_for(path)
            # Mirror the hook's own gates exactly, so we measure only files it
            # would really intercept.
            if not info or not info["isSource"]:
                continue
            if info["lines"] < THRESHOLD or info["rows"] < 3:
                continue

            full = len(ENC.encode(text))
            reduced = len(ENC.encode(info["rendered"]))
            rows.append((str(path), info["lines"], full, reduced))

    if not rows:
        print("No files matched the hook's intercept criteria.")
        return 1

    rows.sort(key=lambda r: r[2] - r[3], reverse=True)
    tot_full = sum(r[2] for r in rows)
    tot_red = sum(r[3] for r in rows)

    print(f"\nFiles the hook would intercept: {len(rows)}")
    print(f"{'file':<58} {'lines':>6} {'full':>8} {'outline':>8} {'saved':>7}")
    print("-" * 92)
    for p, lines, full, red in rows[:15]:
        short = p if len(p) <= 56 else "..." + p[-53:]
        print(f"{short:<58} {lines:>6} {full:>8} {red:>8} {1 - red / full:>6.0%}")
    if len(rows) > 15:
        print(f"... and {len(rows) - 15} more")

    print("-" * 92)
    print(f"{'TOTAL':<58} {'':>6} {tot_full:>8} {tot_red:>8} {1 - tot_red / tot_full:>6.0%}")
    print(f"\nPer full-file read intercepted, median saving: "
          f"{sorted(1 - r[3] / r[2] for r in rows)[len(rows) // 2]:.0%}")
    print("\nNote: this is the saving IF the model would have read the whole file.")
    print("It does not count the follow-up ranged re-read, which costs some of it back.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:] or [str(Path.home() / "projects")]))
