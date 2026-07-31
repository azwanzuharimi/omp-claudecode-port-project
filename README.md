# omp-claudecode-port-project

Context-efficiency tricks ported from [oh-my-pi](https://omp.sh) (omp) into Claude
Code, using **only hooks and skills** — no MCP tools, no patched binaries, nothing
that can break when Claude Code updates.

The idea worth stealing from omp is not its edit format. It is the discipline of
controlling *what enters context in the first place*.

---

## What you get

**Reads that return a map instead of the whole file.**
Ask to read a 1,500-line file and you get a declaration outline with line numbers,
not 19,000 tokens of source. You then read only the ranges you need.
**94% fewer tokens** on intercepted reads ([measured](#measured), not estimated).

**Rules that cost nothing until you break them.**
Every line in `CLAUDE.md` is billed on *every single request*, forever. A rule like
"use `uv`, never bare `pip`" only matters at the moment you reach for `pip` — so it
should cost nothing until then. These rules sit dormant and fire only on the tool
call that violates them.

**Guardrails that catch mistakes before they happen, not after.**
A rule denies the bad call *before* it runs and hands back an explanation, so you
skip the whole write-it-wrong → notice → fix-it cycle. Ships with three: bare `pip`,
bare `python`, and hardcoded credentials.

**Two skills for the expensive habits.**
`codemod` routes repeated mechanical edits through `ast-grep` in one pass instead of
N round trips. `bounded-output` keeps 3,000-line command output out of your context.

**Safe to install and trivial to remove.**
The installer snapshots your config with sha256 before touching anything, refuses to
write if it would disturb another tool's hooks, and leaves a verified one-command
undo. It coexists with existing hook chains rather than replacing them.

**Fast and quiet.** 3.3 ms per hook on the Rust engine, 13.8 ms on bun, 29.0 ms on
node. Every hook silent-fails and exits 0 — a crashed hook can never block a tool
call.

---

## Requirements

| | Needed for | Install |
|---|---|---|
| `bun` **or** `node` | the hooks | `curl -fsSL https://bun.sh/install \| bash` |
| `cargo` | **optional** — builds the Rust hooks, [9× faster](#performance) | [rustup.rs](https://rustup.rs) |
| `jq` | the installer | `brew install jq` / `apt install jq` |
| `ast-grep` | **optional** — only the `codemod` skill | `brew install ast-grep` |
| `uv` | **optional** — only the token benchmark | [astral.sh/uv](https://docs.astral.sh/uv/) |

Claude Code with hook support. macOS or Linux.

```bash
# macOS
brew install jq ast-grep && curl -fsSL https://bun.sh/install | bash

# Debian/Ubuntu
sudo apt install jq && curl -fsSL https://bun.sh/install | bash
```

`install.sh` picks the fastest engine available — the Rust binary if built, else bun,
else node — and bakes absolute paths into the hook commands. Force one with
`OMP_PORT_ENGINE=js` or `OMP_PORT_RUNTIME=/path/to/runtime`. Re-run `install.sh`
after installing a faster option to switch.

### Performance

Nearly all of a hook's cost is process startup, not our code — the hook and an empty
script measure the same within noise. So the engine is the only lever.

| Engine | Per hook call | Busy session (387 calls) | |
|---|---|---|---|
| **rust** | **3.3 ms** | **1.3 s** | `bash build-rust.sh` |
| bun | 13.8 ms | 5.3 s | default if installed |
| node | 29.0 ms | 11.2 s | fallback |

Measured on macOS arm64, 40 runs. The Rust hooks are a straight port — 894 KB binary,
59 differential cases assert their output is byte-identical to the JS. See
[docs/rust-port.md](docs/rust-port.md), including the one known divergence. A Rust rewrite would reach ~3–4 ms; why that is not
worth doing is written up in [docs/rust-port.md](docs/rust-port.md).

## Install

```bash
git clone https://github.com/azwanzuharimi/omp-claudecode-port-project.git
cd omp-claudecode-port-project
bash build-rust.sh    # optional: 9x faster hooks; needs cargo
bash install.sh
```

Then **restart Claude Code** so the hooks load. `build-rust.sh` is optional — without
it the hooks run on bun or node exactly as before, and `install.sh` says which engine
it picked.

The installer is self-contained and safe on a machine that has never seen this repo.
Before touching anything it writes a sha256-verified snapshot of `settings.json`,
`settings.local.json`, `CLAUDE.md`, the two plugin JSONs and any existing `rules/`
into `~/.claude/backups/omp-claudecode-port-project-<timestamp>/`, with a matching
`undo.sh` beside it.

It also:

- refuses to run if `settings.json` is not valid JSON
- counts every pre-existing hook across every event before and after, and **aborts
  without writing** if that number would change — other tools' hooks cannot be clobbered
- never overwrites a rule file you have edited
- is idempotent — re-running replaces only this plugin's own entries
- identifies its own entries by hook *script name*, so a future rename cannot orphan them

Hook paths are absolute and computed at install time, so the repo can live anywhere.
`CLAUDE_CONFIG_DIR` is honored if your config is not at `~/.claude`. Nothing
machine-specific is committed — the same clone works on any box.

## Uninstall

```bash
bash uninstall.sh            # dry run - shows exactly what would change
bash uninstall.sh --apply
```

Finds the newest backup, verifies each file's sha256, restores `settings.json`, and
deletes what the install created (`~/.claude/rules/`, the state dir). It deliberately
leaves `ast-grep` installed — that is a standalone tool, not ours to remove.

## Test

```bash
node test/run.js         # 44 tests, zero dependencies
bun  test/run.js         # same suite under the other runtime
bash build-rust.sh       # builds Rust hooks + 35 unit tests + 59 differential cases
```

Runs against an isolated `CLAUDE_CONFIG_DIR`, so it never reads the rules you
actually have installed. Covers rule matching and scoping, fire-once and `after-gap`,
the hook JSON contracts, outline quality and the coverage gate, false-positive checks
on every shipped rule, deterministic rule ordering, byte-identical output across
node and bun, and the latency budget.

---

## What's inside

```
.claude-plugin/plugin.json   plugin manifest (hook registrations)
hooks/
  lazy-rules.js              PreToolUse  - Edit|Write|MultiEdit|NotebookEdit|Bash
  lazy-rules-post.js         PostToolUse - delivers soft-mode reminders
  read-discipline.js         PreToolUse  - Read
  lib/rules.js               rule parsing, scoping, matching
  lib/outline.js             declaration outlines, 25 extensions / 11 language families
  lib/state.js               per-session state (fire-once, deny-once)
  package.json               pins CJS so require() survives type:module ancestors
rules/                       three example rules, copied to ~/.claude/rules on install
skills/codemod/              ast-grep for repeated mechanical edits
skills/bounded-output/       keeping large command output out of context
rust/src/                    the same hooks in Rust (regress = ECMAScript regex)
bench/read_savings.py        deterministic token measurement, no API calls
test/differential.js         asserts rust output == js output, byte for byte
docs/rust-port.md            the Rust port: measurements, tradeoffs, known divergence
build-rust.sh                build + verify the Rust hooks
lib/undo.sh                  generic undo, copied into every backup
test/run.js                  44 tests
install.sh / uninstall.sh
```

### Lazy rules — the TTSR port

omp's argument: *"Your rules sit dormant until the model goes off-script. You get
course-correction without paying context tax on every turn."*

Rules live in `~/.claude/rules/*.md` (user) and `<repo>/.claude/rules/*.md` (project,
which shadows user rules by filename).

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
| `scope` | `tool:NAME` or `tool:NAME(glob)`, comma-separated. Omit to match every tool |
| `repeat` | `once` (default) or `after-gap N` tool calls |
| `interrupt` | `true` denies the call; `false` attaches a reminder to the result instead |

The regex matches **only the new content** — `new_string` for `Edit`, `content` for
`Write`, `command` for `Bash`. Never the existing file. Matching pre-existing content
would fire on every unrelated edit to a file that happens to contain the pattern.

`interrupt: false` is the cheap variant: it lets the call run and attaches the
reminder to the result, costing no extra round trip.

**Shipped rules:** `no-bare-pip`, `no-bare-python`, `no-hardcoded-secrets`.

**Known limitation:** a regex cannot tell `pip install x` from
`grep -r "pip install" docs/`. Both fire. `repeat: once` caps the cost at a single
denial per session. This is asserted in the test suite rather than papered over with
a fragile lookahead.

### Read discipline

omp's `read` returns declarations with bodies elided. Claude Code cannot rewrite a
tool result — but `permissionDecisionReason` *is* delivered to the model. So the hook
builds the outline itself and returns it as the denial reason. The model gets the
outline for the price of one denied call, then re-reads only what it needs.

It fires only when **all** of these hold:

- recognized source extension (25 extensions across 11 language families)
- no `offset`/`limit` already set
- over 400 lines (`OMP_PORT_READ_THRESHOLD` to change)
- the outline actually represents the file — at least `max(4, lines/80)` declarations
- this path has not already been denied in this session

The last two are what keep it from being annoying. A sparse outline over a long file
teaches nothing, so the model would re-read anyway and the denial cost a round trip
for free. And a path is denied at most **once** per session — a second request means
the model genuinely wants the file, and a denial loop would wedge the session.

---

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
usually follows. It applies only to files that clear the 400-line gate — 5 files in a
mid-size repo, not every read.

**The lazy-rules saving is not measured.** Its value is avoided bad-edit-then-fix
cycles, which needs paired end-to-end runs to quantify. Treat it as unproven.

## What this deliberately is not

**Not a hashline port.** omp's headline feature replaces the edit tool with
content-hash anchors. Claude Code's `Edit` is compiled into a ~257 MB binary and
cannot be replaced. That part does not travel.

Worth knowing if you came here from the blog posts: the widely-described "2-char
per-line hash" format is omp v1 and superseded. Shipping v2 is a single 4-hex
whole-file tag plus plain line numbers.

**The famous numbers do not transfer either.** The quoted "−61% output tokens" is
Grok 4 Fast, and the mechanism is *retry-loop elimination* — weak models failing at
`str_replace` whitespace matching. Claude models were already good at that. From the
same benchmark: Sonnet 4.5 −24%, Haiku 4.5 −22%, and **no Opus measurement exists** —
the "~30% on Opus" figure repeated in several guides is not in the source data. Two
models got *worse* (GPT-5.2 Codex +26%, DeepSeek V3.2 +20%).

**Also not portable:** snapcompact (rendering context to PNG bitmaps), tool-result
pruning, and `xd://` lazy tool schemas all need control over the provider request or
the harness internals. If you want those, use omp itself.

## Design notes

- Coexists with other hook chains rather than replacing them — verified against
  `*`-matcher and specific-matcher hooks from other tools: exit 0, no stdout or
  stderr collision.
- `hooks/package.json` pins CJS so `require()` survives an ancestor `package.json`
  declaring `type: module`.
- `CLAUDE_CONFIG_DIR` is honored throughout; nothing hardcodes `~/.claude`.
- Every hook silent-fails and exits 0. A crashed hook must never block a tool call.
- The full hook output surface in Claude Code is four fields: `additionalContext`,
  `permissionDecision`, `permissionDecisionReason`, and `updatedInput` (PreToolUse
  only). There is no tool-*result* rewrite hook, which is why read discipline works
  by denying with an outline rather than by shrinking the result.

## Credits

This project exists because of [oh-my-pi (omp)](https://github.com/can1357/oh-my-pi)
by **Can Bölük**, itself a fork of [Pi](https://github.com/badlogic/pi-mono) by
**Mario Zechner**. Both are MIT licensed. The ideas here are theirs; the mistakes
are mine.

Worth reading directly — they are better written than this README:

- [The Harness Problem](https://stencil.so/blog/the-harness-problem) — the edit-format
  benchmark across 16 models, and the argument that harness design, not model
  capability, is what most often fails
- [snapcompact](https://stencil.so/blog/snapcompact) — compacting context by rendering
  it to bitmap images and letting vision models read it back
- [omp.sh/docs](https://omp.sh/docs) — the full harness

**What is borrowed:** the lazy-rule concept (omp calls it TTSR, "time-traveling
stream rules"), the structural-outline read, the `matcherDigest` idea of matching
only the content a call introduces, and the `<system-interrupt>` envelope format —
the last reproduced verbatim from omp's TTSR interrupt template.

**What is not:** any source code. Everything here was written from scratch for a
different runtime — these are Node scripts driving Claude Code hooks; omp is
TypeScript and Rust, and the two harnesses share no execution model. omp does its
work by controlling the provider request; this can only deny a tool call or rewrite
its input.

If you want the real thing rather than this partial port, [use omp](https://omp.sh).
It is a better tool than what a hook layer can reach.

Not affiliated with, sponsored by, or endorsed by the omp or Pi projects.

## License

MIT — see [LICENSE](LICENSE), which also carries the upstream MIT notices as
attribution.
