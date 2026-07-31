---
description: Never write a credential literal into a source file
condition: "AKIA[0-9A-Z]{16}|sk-[A-Za-z0-9]{20,}|ghp_[A-Za-z0-9]{36}|xox[baprs]-[A-Za-z0-9-]{10,}|(aws_secret_access_key|api_key|apikey|password|passwd|secret|token)\s*[:=]\s*[\"'](?!(replace_?me|change_?me|x{3,}|your[_-]|my[_-]|dummy|example|placeholder|fake|todo|test|sample|redacted|insert_?|none|null))[^\"'{$\s]{8,}[\"']"
flags: "i"
scope: "tool:Edit, tool:Write, tool:MultiEdit"
repeat: after-gap 25
interrupt: true
---
This edit looks like it writes a real credential into a file.

Do not commit secrets to source. Use instead:
- an environment variable read at runtime (`os.environ["API_KEY"]`)
- the AWS profile already configured on this machine, not an inline key pair
- a `.env` file that is listed in `.gitignore`
- a secrets manager reference

If this is a placeholder, a test fixture, or an obviously fake value, make that
unambiguous — `"REPLACE_ME"`, `"xxx"`, or an interpolated `${VAR}` — and re-issue.
