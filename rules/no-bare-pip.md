---
description: Use UV, never bare pip
condition: "(?<!uv )(?<![\w./-])pip3? install"
scope: "tool:Bash, tool:Write(*.sh), tool:Write(Dockerfile), tool:Write(Makefile)"
repeat: once
interrupt: true
---
This machine uses UV for Python, so a bare `pip install` writes to the wrong
environment and the package will not be importable from the project.

Use instead:
- `uv add <pkg>` to add a project dependency (updates pyproject.toml + lockfile)
- `uv pip install <pkg>` for a throwaway install into the active venv
- `uv run --with <pkg> <cmd>` to run something without installing at all

Re-issue the command with the UV form.
