# omp-lite

Context-efficiency tricks ported from [oh-my-pi](https://omp.sh) into Claude Code,
using only hooks and skills — no MCP tools, no patched binaries.

## What this is not

**It is not a hashline port.** oh-my-pi's headline feature replaces the edit tool
with content-hash anchors. Claude Code's `Edit` is compiled into a 257 MB binary and
cannot be replaced. That part does not travel.

**The famous numbers do not transfer either.** The widely-quoted "−61% output tokens"
is Grok 4 Fast, and the mechanism is *retry-loop elimination* — weak models failing
at `str_replace` whitespace matching. Claude models were already good at that.
Measured in the same benchmark: Sonnet 4.5 −24%, Haiku 4.5 −22%, and **no Opus
measurement exists at all** — the "~30% on Opus" figure in several guides is not in
the source data. Two models got *worse* (GPT-5.2 Codex +26%, DeepSeek V3.2 +20%).

What *does* travel is the part nobody quotes: controlling what enters context.

## Components

| | Mechanism | Status |
|---|---|---|
| **Lazy rules** | Regex rules over new content; fire only when you go off-script | Working, unmeasured |
| **Read discipline** | Long unbounded reads get an outline instead of the file | Working, **94% measured** |
| **codemod** skill | `ast-grep` for repeated mechanical edits | Skill only |
| **bounded-output** skill | Keep large command output out of context | Skill only |

### Lazy rules — the TTSR port

oh-my-pi's argument: *"Your rules sit dormant until the model goes off-script. You
get course-correction without paying context tax on every turn."*

An always-on `CLAUDE.md` rule is billed on **every request**. A rule that only
matters when you're about to do one specific wrong thing costs nothing until it
fires.

Rules live in `~/.claude/rules/*.md` (user) and `<repo>/.claude/rules/*.md`
(project — shadows user by filename).

```markdown
---
description: Use UV, never bare pip
condition: "(?<!uv )(?<![\w./-])pip3? install"
scope: "tool:Bash, tool:Write(*.sh)"
repeat: once
interrupt: true
---
This machine uses UV. Use `uv add <pkg>` instead.
```

| Field | Meaning |
|---|---|
| `condition` | JS regex, **taken verbatim** — write `\w`, not `\\w` |
| `flags` | optional regex flags, e.g. `"i"` |
| `scope` | `tool:NAME` or `tool:NAME(glob)`, comma-separated. Omit to match all tools |
| `repeat` | `once` (default) or `after-gap N` tool calls |
| `interrupt` | `true` denies the call; `false` attaches a reminder to the result instead |

The regex matches **only the new content** — `new_string` for `Edit`, `content` for
`Write`, `command` for `Bash`. Never the existing file. Matching pre-existing content
means every unrelated edit to a file that happens to contain the pattern fires the
rule.

Ships three rules: `no-bare-pip`, `no-bare-python`, `no-hardcoded-secrets`.

**Known limitation:** a regex cannot tell `pip install x` from
`grep -r "pip install" docs/`. Both fire. `repeat: once` keeps the cost to a single
denial per session. This is asserted in the test suite rather than papered over with
a fragile lookahead.

### Read discipline

oh-my-pi's `read` returns declarations with bodies elided. Claude Code cannot rewrite
a tool result — but `permissionDecisionReason` *is* delivered to the model. So the
hook builds the outline itself and returns it as the denial reason. The model gets
the outline for the price of one denied call, then re-reads only what it needs.

Fires only when **all** of these hold:

- recognized source extension
- no `offset`/`limit` already set
- over 400 lines (`OMP_LITE_READ_THRESHOLD` to change)
- the outline actually represents the file — at least `max(4, lines/80)` declarations
- this path has not already been denied in this session

The last two matter. A sparse outline over a long file teaches nothing, so the model
re-reads anyway and the denial cost a round trip for free. And a path is denied at
most **once** per session — a second request means the model genuinely wants the
file, and a denial loop would wedge the session.

## Measured

```
$ uv run bench/read_savings.py ~/projects/caveman-bluf

Files the hook would intercept: 5
file                                          lines     full  outline   saved
-----------------------------------------------------------------------------
caveman-bluf/bin/install.js                    1531    19258      618    97%
caveman-bluf/tests/test_caveman_stats.js        638     8729      907    90%
caveman-bluf/tests/installer/e2e...test.mjs     557     7092      579    92%
caveman-bluf/src/hooks/caveman-stats.js         534     6283      350    94%
caveman-bluf/tests/verify_repo.py               420     3811      237    94%
-----------------------------------------------------------------------------
TOTAL                                                  45173     2691    94%
```

Deterministic, no API calls. Tokenizer is o200k via tiktoken, which approximates
Claude's — ratios are meaningful, absolute counts are close but not exact.

**Read this honestly.** The 94% is the saving *if* the model would otherwise have
read the whole file, and it does not subtract the follow-up ranged re-read that
usually follows. It also only applies to the few files that clear the 400-line gate —
5 files in a mid-size repo, not every read.

**The lazy-rules saving is not measured.** Its value is avoided bad-edit-then-fix
cycles, which needs paired end-to-end runs to quantify. Treat it as unproven.

## Install

```bash
bash install.sh     # registers hooks in settings.json, copies example rules
```

Idempotent. Re-running replaces only omp-lite's own hook entries, refuses to write
if the generated settings.json is invalid or if any pre-existing hook would be lost,
and never overwrites a rule file you already have. Requires `jq`, `node`, and
`ast-grep` (for the codemod skill only).

## Uninstall

```bash
ls -d ~/.claude/backups/omp-lite-*/ | sort | tail -1   # newest snapshot
bash <that>/undo.sh            # dry run
bash <that>/undo.sh --apply    # restore settings.json, remove installed files
```

## Test

```bash
node test/run.js    # 38 tests, no dependencies
```

Runs against an isolated `CLAUDE_CONFIG_DIR` so it never reads the rules you actually
have installed. Covers rule matching and scoping, fire-once and `after-gap`, hook
JSON contracts, outline quality and the coverage gate, false-positive checks on every
shipped rule, and a 150 ms latency budget per hook (currently ~25 ms).

## Notes for this machine

- Hooks coexist with the existing Orca (`*`) and moshi (specific-matcher) chains —
  verified: exit 0, no stdout, no stderr collision.
- `hooks/package.json` pins CJS so `require()` survives an ancestor `package.json`
  declaring `type: module`. Same fix caveman needed.
- `CLAUDE_CONFIG_DIR` is honored throughout; nothing hardcodes `~/.claude`.
- Every hook silent-fails and exits 0. A crashed hook must never block a tool call.
