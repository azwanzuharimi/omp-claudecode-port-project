# omp-claudecode-port-project

Context-efficiency tricks ported from [oh-my-pi](https://omp.sh) (omp) into Claude
Code as **hooks and skills** — no MCP tools, nothing patched inside Claude Code,
nothing that breaks when it updates. The hooks ship as one small static binary.

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

**Safe to install and clean to remove.**
The installer snapshots your config with sha256 before touching anything and aborts
rather than disturb another tool's hooks. Uninstall is surgical — it strips only this
plugin's entries, keeps any rule you edited, and tells you what it deliberately left
behind. Uninstall is dry-run-first; both are idempotent.

**Fast and quiet.** 3.3 ms per hook, from a single static binary with no runtime
dependency. Every hook silent-fails and exits 0 — a crashed hook can never block a
tool call.

---

## Requirements

| | Needed for | Install |
|---|---|---|
| `cargo` | building the hooks | [rustup.rs](https://rustup.rs) |
| `jq` | the installer | `brew install jq` / `apt install jq` |
| `node` | optional — running the test suites | |
| `ast-grep` | optional — only the `codemod` skill | `brew install ast-grep` |
| `uv` | optional — only the token benchmark | [astral.sh/uv](https://docs.astral.sh/uv/) |

Claude Code with hook support. macOS or Linux.

```bash
# macOS
brew install jq ast-grep && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Debian/Ubuntu
sudo apt install jq && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

The installed hooks are a **single static binary with no runtime dependency** — no
node, no bun, no interpreter. `cargo` and `jq` are needed once, at install time.

### Performance

Nearly all of a hook's cost is process startup, not our code — the hook and an empty
script measure the same within noise. That is why the hooks are compiled.

| | Per hook call | Busy session (387 calls) |
|---|---|---|
| **rust (shipped)** | **3.3 ms** | **1.3 s** |
| node, for reference | 29.0 ms | 11.2 s |

Measured on macOS arm64, 40 runs. 894 KB binary, with 62 differential cases
asserting its output is byte-identical to the JS reference. See
[docs/rust-port.md](docs/rust-port.md), including the one known divergence.

**Why not bun?** It was the engine for one commit. `fs.readFileSync(0)` under bun
allocates catastrophically when fd 0 is a pipe — a 20 MB payload drove RSS to
**6.5 GB** and OOM-killed the machine, where node peaked at 29 MB and the binary at
1 MB. Claude Code delivers hook payloads over a pipe, so that is the path that
matters. One engine, measured and bounded, beats three with different failure modes.

## Install

```bash
git clone https://github.com/azwanzuharimi/omp-claudecode-port-project.git
cd omp-claudecode-port-project
bash install.sh
```

Then **restart Claude Code** so the hooks load.

`install.sh` builds the binary for you if it is missing and `cargo` is available
(~30 s, once). To build and run the full verification first, use
`bash build-rust.sh` — it compiles, runs 35 unit tests, then proves the binary
matches the JS reference on 62 payloads.

The installer is self-contained and safe on a machine that has never seen this repo.
If `settings.json` does not exist yet it creates an empty one.
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

Hook paths are absolute and computed at install time, so the repo can live anywhere —
but **do not move or delete the clone after installing**, or the hooks will point at
a path that no longer exists. Move it, then re-run `install.sh`.

`CLAUDE_CONFIG_DIR` is honored if your config is not at `~/.claude`. Nothing
machine-specific is committed — the same clone works on any box.

### After rebuilding

Re-run `install.sh` after any rebuild or after moving the clone — it re-resolves the
absolute path baked into the hook commands.

## Uninstall

```bash
bash uninstall.sh            # dry run - prints exactly what would change
bash uninstall.sh --apply
```

Then restart Claude Code so it stops loading the hooks.

Removal is **surgical**, not a snapshot rollback. It:

- strips only this plugin's hook entries from `settings.json`, leaving every other
  key and every other tool's hooks intact — and aborts without writing if that count
  would change. (The file is rewritten through `jq`, so whitespace and key order are
  normalized; content is preserved, exact bytes are not.)
- deletes the rules it installed, but **keeps any rule you edited** and says which
- deletes `~/.claude/state/omp-claudecode-port-project/`
- removes `~/.claude/rules/` only if it ends up empty

It is idempotent: running it twice, or on a machine where nothing is installed, is
safe and prints what it found.

**Left behind on purpose**, with paths printed so you can remove them yourself:

| | Why |
|---|---|
| `rust/target/` (~140 MB) | build cache — `rm -rf rust/target` |
| this clone | delete the directory when you no longer need the tool |
| `~/.claude/backups/omp-claudecode-port-project-*` | your rollback safety net |
| `ast-grep` | a standalone tool, not ours to remove |

### Rolling back instead of uninstalling

```bash
bash uninstall.sh --restore-snapshot            # dry run
bash uninstall.sh --restore-snapshot --apply
```

Restores `settings.json` and friends from the newest sha256-verified snapshot. Use
this to undo a bad install, not for removal — **a snapshot taken during a re-install
was captured while the plugin was already active**, so restoring it can leave the
plugin in place. Surgical removal is correct however many times you have installed.

## Test

```bash
bash build-rust.sh   # build + 35 Rust unit tests + 62 differential cases
node test/run.js     # 39 tests against the JS reference implementation
```

Runs against an isolated `CLAUDE_CONFIG_DIR`, so it never reads the rules you
actually have installed. Covers rule matching and scoping, fire-once and `after-gap`,
the hook JSON contracts, outline quality and the coverage gate, false-positive checks
on every shipped rule, deterministic rule ordering, and the latency budget.

`test/differential.js` is the one that matters: it runs 62 payloads through both the
binary and the JS reference and asserts the bytes match exactly.

---

## What's inside

```
.claude-plugin/plugin.json   plugin manifest (hook registrations)
rust/src/                    THE HOOKS - what actually runs
  main.rs                    3 subcommands: lazy-rules, lazy-rules-post, read-discipline
  rules.rs                   rule parsing, scoping, matching (regress = ECMAScript regex)
  outline.rs                 declaration outlines, 25 extensions / 11 language families
  state.rs                   per-session state (fire-once, deny-once)
hooks/                       JS REFERENCE - not installed; the oracle the binary is tested against
  lazy-rules.js              PreToolUse  - Edit|Write|MultiEdit|NotebookEdit|Bash
  lazy-rules-post.js         PostToolUse - delivers soft-mode reminders
  read-discipline.js         PreToolUse  - Read
  lib/*.js                   the same logic in JS, mirrored by rust/src/
rules/                       three example rules, copied to ~/.claude/rules on install
skills/codemod/              ast-grep for repeated mechanical edits
skills/bounded-output/       keeping large command output out of context
bench/read_savings.py        deterministic token measurement, no API calls
test/differential.js         asserts binary output == JS reference, byte for byte
docs/rust-port.md            the Rust port: measurements, tradeoffs, known divergence
build-rust.sh                build + verify
lib/undo.sh                  generic undo, copied into every backup
test/run.js                  39 tests (JS reference)
install.sh / uninstall.sh    register / remove (uninstall is dry-run-first)
rust/Cargo.toml              deps: regress + serde_json, nothing else
.github/workflows/release.yml  native builds for 4 targets on tag
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

- **One engine, on purpose.** The hooks are a compiled binary and nothing else. An
  earlier version picked between rust, bun and node at install time; bun turned out
  to allocate 6.5 GB reading a 20 MB pipe and OOM-killed a machine. Three engines
  means three failure modes to know about.
- **Regex is `regress`**, an ECMAScript engine, not Rust's `regex` (no lookaround) or
  `fancy-regex` (Oniguruma semantics would silently change `\d \w \s \b . (?i)` in
  user-written rules). Rules are user-authored JS regexes; the engine must match.
- Coexists with other hook chains rather than replacing them — verified against
  `*`-matcher and specific-matcher hooks from other tools: exit 0, no stdout or
  stderr collision.
- `CLAUDE_CONFIG_DIR` is honored throughout; nothing hardcodes `~/.claude`.
- Every hook catches panics and exits 0. A crashed hook must never block a tool call.
- The JS in `hooks/` is the reference implementation, kept because
  `test/differential.js` needs an oracle. It is never installed.
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

**What is not:** any source code. Everything here was written from scratch against a
different execution model. These are standalone binaries invoked as Claude Code
hooks — one process per tool call, stdin JSON in, stdout JSON out. omp is a whole
harness that controls the provider request; a hook can only deny a tool call or
rewrite its input. Sharing a language (both use Rust, both use `regress`-class
ECMAScript regex) is convergence on the obvious choice, not shared lineage.

If you want the real thing rather than this partial port, [use omp](https://omp.sh).
It is a better tool than what a hook layer can reach.

Not affiliated with, sponsored by, or endorsed by the omp or Pi projects.

## License

MIT — see [LICENSE](LICENSE), which also carries the upstream MIT notices as
attribution.
