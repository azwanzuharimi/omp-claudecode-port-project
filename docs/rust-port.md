# Porting the hooks to Rust

Status: **done, and now the only engine.** `rust/` builds `omp-hooks`, a single
binary with three subcommands. `install.sh` installs nothing else.

| | Per hook call | Busy session (387 calls) |
|---|---|---|
| **rust** | **3.26 ms** | **1.3 s** |
| node (reference only) | 28.99 ms | 11.2 s |
| ~~bun~~ | ~~13.78 ms~~ | removed - see below |

## Why bun was removed

bun was the engine for exactly one commit. `fs.readFileSync(0, 'utf8')` under bun
allocates catastrophically when fd 0 is a **pipe** rather than a regular file:

| Engine | Peak RSS, 20 MB piped payload |
|---|---|
| rust | 1 MB |
| node | 29 MB |
| **bun 1.3.14** | **6,551 MB** -> OOM |

The same payload delivered by file redirect (`< file`) was fine on all three, which
is why ordinary testing missed it. Claude Code delivers hook payloads over a pipe.

This was found the hard way, by OOM-killing a developer machine. `test/differential.js`
now carries 1 MB and 4 MB piped-payload cases so any future regression shows up as a
test failure rather than a dead laptop.

894 KB binary, 35 unit tests, and 62 differential cases asserting byte-identical
output against the JS hooks. The analysis that led here is kept below, corrected
where building it proved the estimates wrong.

## Known divergence: one, and it is unfixable

Outline rows are truncated at 160 UTF-16 units, matching JS `.slice(0, 160)`. If an
astral character (emoji, rare CJK) **straddles that exact boundary** on a declaration
line, JS keeps a lone high surrogate and Rust drops the character:

```
js  : ...xxxxx\ud83d      (a lone surrogate, escaped by JSON.stringify)
rust: ...xxxxx            (character dropped)
```

Rust `String` cannot hold a lone surrogate, so there is no faithful option — and the
JS output here is arguably the broken one, since `\ud83d` alone is not valid text.
Found by fuzzing 300 files with emoji planted at offsets 155–161; 20 rows diverged,
all of them this case and nothing else. Reproduced and confirmed independently.

Everything else — all 62 differential cases, and a 14-file corpus spanning every
supported language plus CRLF, BOM, astral characters, and trailing whitespace — is
byte-identical.

## Cost that did not show up in the estimates

`outline()` scan time is roughly **26 µs/KB**, because ~30 anchored backtracking
patterns run per line. Files above ~80 KB therefore blow the 2.5 ms budget on scan
alone; `read-discipline`'s 4 MB ceiling means a worst-case call is far slower than
the startup figure suggests. The JS has the same shape of cost, so this is not a
regression — but if it matters, the fix is a size cutoff before `outline()` runs,
not a change to the engine.

---

## Original analysis

This recorded the measurements and blockers before the port was done. Kept because
the reasoning about regex engines is what made the port safe.

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

**node → bun saves ~5 s** on a busy session; **bun → Rust saves ~4 s more.**
Both were done. The measured Rust figure (3.26 ms) landed within 0.7 ms of the
estimate projected from `rg` and `ast-grep`.

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

## What it actually bought

Latency: 10.5 ms/hook over bun, ~4 s on a busy session.

The bigger win is **dropping runtime dependencies.** A Rust binary needs
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

## How it was actually done

The plan above survived contact, and all three points held:

1. **`hooks/lib/rules.js` ported first, on `regress`.** As predicted, that is where
   the semantic risk lives. Subtleties that needed deliberate handling and would
   otherwise have diverged silently: a JS `Map` keeps the *original* slot when a key
   is overwritten, so a project rule shadowing a user rule inherits the user
   directory's position; `entries.sort()` is UTF-16 order, not UTF-8 byte order;
   `.trim()` disagrees between the languages on U+FEFF and U+0085; `path.basename`
   and `path.extname` needed byte-for-byte ports of Node's posix algorithms;
   `matcher_digest` uses `??` for most tools but truthiness for MultiEdit.

2. **`test/differential.js` is the conformance suite** — 62 payloads through both
   engines, compared byte for byte, covering every tool type, every regex class that
   can diverge between engines, malformed input, piped payloads up to 4 MB, and
   shared state files. `outline.rs` was additionally diffed against `outline.js` over
   a 14-file corpus and 300 fuzzed files.

3. **`install.sh` registers the binary and nothing else.** The plan had it falling
   back to bun then node; that fallback is gone. bun's `fs.readFileSync(0)` on a pipe
   allocated 6.5 GB on a 20 MB payload (see above), and carrying three engines meant
   carrying three failure modes. One engine, measured and bounded, won.

The JS implementation stays in the repo as the reference and the oracle. It is not
dead code and it is not a fallback — it is what the Rust binary is tested against,
and it is the only reason a silent regex divergence would be caught.
