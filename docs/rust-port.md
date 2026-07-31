# Porting the hooks to Rust — analysis

Status: **not done, and not currently recommended.** This records the measurements
and blockers so the decision can be revisited with data instead of re-litigated.

## The question

Hooks are `stdin JSON → stdout JSON`, so the language is free. Rust is possible. The
question is only whether it pays.

## Measured (macOS arm64, 2026-07-31)

All figures below are from one harness — 40 runs each, stdin supplied, output
discarded. Mixing harnesses produces nonsense; an earlier draft of this file quoted
`rg` at 7.7 ms and `bun -e` at 3.8 ms, both artifacts of a sloppier loop.

Process floors:

| | Time |
|---|---|
| `/bin/echo` — fork+exec floor | 1.91 ms |
| `jq -n 0` | 2.54 ms |
| `ast-grep --version` (Rust) | 2.59 ms |
| `rg --version` (Rust, links full `regex` + Unicode tables) | 2.72 ms |
| `bun -e 0` | 7.31 ms |
| `node -e 0` | 25.01 ms |

The actual hook, real payload — and an empty script on the same runtime, to separate
interpreter cost from our code:

| | Time |
|---|---|
| `node empty.js` | 24.97 ms |
| **`node hooks/lazy-rules.js`** | **25.14 ms** |
| `bun empty.js` | 12.98 ms |
| **`bun hooks/lazy-rules.js`** | **12.32 ms** |
| Rust (projected, from `rg`/`ast-grep`) | ~2.6–3.0 ms |

**Our code is free.** The hook and an empty script cost the same within noise on both
runtimes. Essentially 100% of the time is interpreter startup — which is why the
language of the *logic* was never the question. The runtime is.

`bun --compile` was tested and rejected: 12.6 ms (no better than bun) for a 55 MB
binary.

Per session, using hook-invocation counts from real transcripts:

| Session | Hook calls | node | bun | Rust (est.) |
|---|---|---|---|---|
| busiest | 387 | 9.7 s | 4.8 s | ~1.1 s |
| typical | 260 | 6.5 s | 3.2 s | ~0.7 s |

**node → bun saves ~5 s** on a busy session, for a runtime-detection change.
**bun → Rust would save ~3.7 s more**, for a full rewrite. Real, but expensive.

## The regex problem — and its solution

Rules are **user-authored JavaScript regexes**, carried verbatim in Markdown
frontmatter along with a JS flag string. `hooks/lib/rules.js` does literally
`new RegExp(meta.condition, meta.flags || '')`. Any port must preserve JS semantics
exactly, or guardrails silently stop guarding.

All three shipped rules already use lookaround:

```
no-bare-pip          (?<!uv )(?<![\w./-])pip3? install
no-bare-python       (?<![\w./-])python3?\s+[\w./-]+\.py
no-hardcoded-secrets ...(?!(replace_?me|change_?me|x{3,}|...))...
```

### Do not use the `regex` crate

No lookahead or lookbehind, permanently. Maintainer BurntSushi, in
[discussion #910](https://github.com/rust-lang/regex/discussions/910): *"To the
degree I can be certain about anything, I'd say no, general look-around will never
be supported."* It buys linear-time matching, which lookaround breaks.

### Do not use `fancy-regex` either — this is the trap

It *does* support lookaround (including variable-length, on by default) and it is
mature. But its syntax is Perl/Oniguruma-flavored, **not** ECMAScript, and the
divergences are silent:

| | JavaScript | fancy-regex / `regex` |
|---|---|---|
| `\d` | always `[0-9]` | `\p{Nd}` — matches Arabic-Indic digits, `𝟚` |
| `\w` | `[A-Za-z0-9_]` | alphabetic + marks + digits + connectors |
| `\s` | includes U+FEFF | `\p{White_Space}` — excludes U+FEFF |
| `\b` | over ASCII `\w` | Unicode word boundary |
| `.` | excludes `\n`, `\r`, U+2028, U+2029 | excludes **only** `\n` |
| `(?i)` | ASCII-only folds without `/u` | Unicode simple case folding always |

Every shipped rule uses at least one of `\w`, `\s`, or `.`. Compile errors would be
loud; **these would not be.** `no-bare-python`'s `(?<![\w./-])` would start treating
non-ASCII letters as word characters, changing when the lookbehind fires — with no
error, no warning, and a test suite that still passes on ASCII fixtures.

### Use [`regress`](https://crates.io/crates/regress)

An ECMAScript-syntax regex engine — it is the regex backend of the
[Boa](https://github.com/boa-dev/boa) JavaScript engine, and is used by `oxc_resolver`,
`rspack`, and `swc_config`. v0.11.1, ~37M downloads.

It maps 1:1 onto the current code. `Flags: From<&str>` parses a JavaScript flag
string, so `rules.js:107` becomes:

```rust
let re = regress::Regex::with_flags(&meta.condition, meta.flags.as_deref().unwrap_or(""))?;
```

`\d`, `\w`, `\b`, `.`, `(?i)` and `\p{...}` all behave as rule authors already
expect. Variable-width lookbehind with capture groups is supported. Two dependencies.

Caveats: `g` is unsupported and ignored (we don't use it). No linear-time guarantee —
it backtracks, exactly like V8, which is the *correct* fidelity trade here since our
current Node implementation has identical exposure. No documented step-limit knob, so
a hard bound would need a thread with a timeout.

Whichever engine is chosen, the port needs a **differential test harness**: run every
rule condition plus adversarial patterns through both Node and the Rust binary and
assert identical match/no-match. The failure mode here is silent, and only
differential testing catches it.

## Distribution

The repo's selling point is `git clone && bash install.sh` on any machine.

| Option | Cost |
|---|---|
| GitHub Actions matrix → Release → `install.sh` downloads the right triple | **best.** `taiki-e/upload-rust-binary-action` or `cargo-dist`; needs network at install |
| `cargo build --release` at install | requires a Rust toolchain and a multi-second first-run stall |
| Commit prebuilt binaries | bloats the repo permanently; every release re-commits megabytes |
| Hybrid: download → `cargo build` → fall back to the existing JS hooks | preserves the current no-dependency guarantee |

**macOS arm64 requires a code signature — even an ad-hoc one.** Apple Silicon refuses
to execute unsigned arm64 code, and the failure is `Killed: 9` with no useful message.
Native `clang` linking signs ad-hoc automatically; a Linux-hosted cross-build does
not. Use native macOS runners (`macos-14` for arm64, `macos-13` for x64) rather than
cross-compiling — this one gotcha is worth more than all the cross-compile tooling.
Linux is easy by comparison: `*-unknown-linux-musl` via `cross` gives static binaries
and dodges glibc skew.

Files downloaded by `curl` do not get the quarantine xattr, so an install-script
download works with only ad-hoc signing. A browser-downloaded zip would not.

## Startup: the real risk is regex compilation, not linking

`rg --version` — a much larger binary linking the full `regex` crate with complete
Unicode tables — runs in **2.72 ms** on this machine, against a 1.91 ms `fork+exec`
floor. So ~2.5 ms for a single-purpose binary is realistic, and there is almost
no headroom below that.

What would blow it: **eagerly compiling regexes at startup.** `hooks/lib/outline.js`
holds ~30 patterns across 11 language families, and `rules.js` compiles every rule's
condition at load. In Rust that would dominate the runtime. The fix is structural:

- compile only the `LANGS[lang]` bucket for the file actually being read
- in `evaluate()`, filter by `scopeAllows` and `isArmed` *before* compiling a
  condition — most calls reject on the cheap checks and never need the regex

`serde` derive costs compile time, not startup. Avoid `once_cell` blocks that build
`RegexSet`s eagerly. Skip UPX — decompression on every start defeats the purpose.

## What it actually buys

Latency is the weaker argument: ~9.5 ms/hook over bun, ~3.7 s on a busy session.

The stronger one is **dropping runtime dependencies entirely.** A Rust binary needs
no `bun`, no `node` — and could do the `settings.json` editing itself, removing the
`jq` requirement too. `git clone && ./install` with zero prerequisites is a genuinely
better story than the current three. If this port ever happens, that should be the
reason, not the milliseconds.

## What would change the verdict

- **Hook invocation counts rising sharply** (10× more calls per session) — the linear
  saving would become seconds-per-minute rather than seconds-per-session.
- **A rule set with no lookaround.** That removes the need for a backtracking engine
  and lets the standard `regex` crate be used — fast, linear-time, no DoS exposure.
- **Wanting the hooks usable outside Claude Code**, where a dependency-free static
  binary is worth more than the raw speed.
- **Bun regressing** or proving unstable across versions.

## If it is done anyway

Keep the JS implementation as the reference and the oracle:

1. Port `hooks/lib/rules.js` first, on `regress` — it is the only part with tricky
   semantics (`matcherDigest` per tool, scope globs, fire-once / `after-gap`), and
   the regex-fidelity risk lives entirely here.
2. Reuse the existing rule corpus in `test/run.js` as a conformance suite; the Rust
   binary must produce **byte-identical stdout** for every payload the JS hooks
   handle. The cross-runtime equivalence test already in the suite is the template —
   it is what caught a real ordering bug between node and bun.
3. Keep `install.sh`'s runtime detection and let it prefer a compiled binary when
   present, falling back to bun then node. That makes the port incremental and
   reversible rather than a flag day.
