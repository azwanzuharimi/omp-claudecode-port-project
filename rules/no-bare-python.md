---
description: Run Python through uv run, not a bare interpreter
condition: "(?<![\w./-])python3?\s+[\w./-]+\.py"
scope: "tool:Bash"
repeat: after-gap 40
interrupt: true
---
A bare `python foo.py` uses whichever interpreter is on PATH, not the project
environment, so project dependencies will not resolve.

Use `uv run foo.py` instead. It resolves the project venv and installs anything
missing first.

If you deliberately want the system interpreter (a standalone script with no project
deps), say so and re-issue — this rule will not fire again for a while.
