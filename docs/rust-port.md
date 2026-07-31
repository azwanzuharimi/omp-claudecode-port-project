# Porting the hooks to Rust — analysis

Status: **not done, and not currently recommended.** This records the measurements
and blockers so the decision can be revisited with data instead of re-litigated.

## The question

Hooks are `stdin JSON → stdout JSON`, so the language is free. Rust is possible. The
question is only whether it pays.

## Measured (macOS arm64, 2026-07-31)

Process startup floors, 20–30 runs each:

| | Time | Note |
|---|---|---|
| `/bin/echo` | 2.5 ms | macOS process-spawn floor |
| `ast-grep --version` | 3.0 ms | a real Rust CLI — the realistic Rust target |
| `rg --version` | 7.7 ms | another Rust CLI, larger |
| `bun -e ''` | 3.8 ms | **misleading — see below** |
| `node -e ''` | 24.6 ms | |

End-to-end, the actual `lazy-rules` hook with a real payload:

| Runtime | Per call | vs node |
|---|---|---|
| node | **28.4 ms** | — |
| bun | **13.5 ms** | 2.1× faster |
| bun `--compile` binary | 12.6 ms | no better than bun, and 55 MB |
| Rust (projected) | ~3–4 ms | ~4× faster than bun |

**The trap:** `bun -e ''` is 3.8 ms but bun executing a *file* is ~16 ms even when the
file is empty. That gap is bun's file-execution path, not our modules — loading zero,
one, or three of our modules moved it only 16.0 → 16.3 ms. An early estimate of "bun
is 6× faster" was extrapolated from `-e` and was wrong. The real figure is 2.1×.

Per session, using hook-invocation counts from real transcripts:

| Session | Hook calls | node | bun | Rust (est.) |
|---|---|---|---|---|
| busiest | 387 | 10.6 s | 5.3 s | ~1.4 s |
| typical | 260 | 7.1 s | 3.6 s | ~0.9 s |

So: **node → bun saves ~5 s** on a busy session for a one-line change. **bun → Rust
would save ~4 s more**, for a full rewrite. That is a real gain, not a rounding
error — it is just bought at a high price.

## Blockers

### 1. Lookaround (the hard one)

All three shipped rules use it:

```
no-bare-pip          (?<!uv )(?<![\w./-])pip3? install
no-bare-python       (?<![\w./-])python3?\s+[\w./-]+\.py
no-hardcoded-secrets ...(?!(replace_?me|change_?me|x{3,}|...))...
```

Rust's `regex` crate does **not** support lookahead or lookbehind, by deliberate
design — it guarantees linear-time matching, which lookaround breaks.

The route is [`fancy-regex`](https://crates.io/crates/fancy-regex), which adds
lookaround and backreferences via backtracking. That has consequences:

- It reintroduces catastrophic-backtracking risk on hostile patterns. Rules are
  user-authored, so a bad rule could hang a hook. Would need a match timeout.
- Its syntax is Perl/Oniguruma-flavored, **not** JavaScript. Rules are documented as
  "JS regex, taken verbatim." Any dialect gap becomes a rule that silently behaves
  differently rather than failing loudly — the worst failure mode for a guardrail.

Anyone doing this port must first audit dialect differences (named groups,
`\p{...}` unicode property escapes, `\b` semantics, unicode case folding) and add
cross-engine equivalence tests over a corpus of real rules.

### 2. Distribution

The repo's selling point is `git clone && bash install.sh` on any machine. Rust
means one of:

| Option | Cost |
|---|---|
| `cargo build --release` at install | requires a Rust toolchain; cold build of serde_json + fancy-regex is minutes |
| Commit prebuilt binaries | 4 platform binaries in git; every release re-commits megabytes |
| GitHub Actions cross-compile + download on install | needs network at install; most infrastructure to maintain |

All three are worse than the current "clone and run."

### 3. It buys less than the rewrite costs

Roughly 4 s per busy session, against a rewrite of three hooks plus three libs, a new
regex dialect to validate, and a build/release pipeline. The hooks are also not on the
critical path in a way a user perceives — they run between tool calls, not during
model output.

## What would change the verdict

- **Hook invocation counts rising sharply** (10× more calls per session) — the linear
  saving would become seconds-per-minute rather than seconds-per-session.
- **A rule set with no lookaround.** That removes the `fancy-regex` dependency and
  lets the standard `regex` crate be used, which is fast and has no backtracking risk.
- **Wanting the hooks usable outside Claude Code**, where a dependency-free static
  binary is worth more than the raw speed.
- **Bun regressing** or proving unstable across versions.

## If it is done anyway

Keep the JS implementation as the reference and the oracle:

1. Port `hooks/lib/rules.js` first — it is the only part with tricky semantics
   (`matcherDigest` per tool, scope globs, fire-once / `after-gap`).
2. Reuse the existing rule corpus in `test/run.js` as a conformance suite; the Rust
   binary must produce **byte-identical stdout** for every payload the JS hooks
   handle. The cross-runtime equivalence test already in the suite is the template —
   it is what caught a real ordering bug between node and bun.
3. Keep `install.sh`'s runtime detection and let it prefer a compiled binary when
   present, falling back to bun then node. That makes the port incremental and
   reversible rather than a flag day.
