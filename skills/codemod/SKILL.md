---
name: codemod
description: Use when the same mechanical code change repeats across 3 or more places - renaming a function at every callsite, stripping all console.log/print calls, migrating an API signature, swapping an import form. Routes the change through ast-grep in one pass instead of a sequence of Edit calls. Do not use for one-off edits or changes that need judgment per site.
---

# Structural codemods with ast-grep

A repetitive change made with N `Edit` calls costs N round trips, and every one of
them re-reads context you already have. `ast-grep` does the whole sweep in one call
and shows you only what changed.

This is the local stand-in for oh-my-pi's `ast_edit`. The tool is `ast-grep`
(binary: `ast-grep` or `sg`), already installed.

## When this applies

Use it when the same transform repeats **3 or more times**. Below that, plain `Edit`
is cheaper than the cognitive overhead of writing a pattern.

Do NOT use it when each site needs a different decision. Structural search finds
every match — that is the point, and it is also the risk.

## Workflow

**Always preview before writing.** `ast-grep` rewrites in place with `-U`, so a
wrong pattern is a wrong repo-wide change.

```bash
# 1. See what matches. No -U means no writes.
ast-grep --pattern 'legacyFn($$$ARGS)' --lang ts src/

# 2. Preview the rewrite as a diff.
ast-grep --pattern 'legacyFn($$$ARGS)' --rewrite 'newFn($$$ARGS)' --lang ts src/

# 3. Apply once the diff is right.
ast-grep --pattern 'legacyFn($$$ARGS)' --rewrite 'newFn($$$ARGS)' --lang ts -U src/
```

## Narrow the path first

Never scan the repo root. Point at the narrowest directory that contains the change.
A repo-root sweep is slow, and its output is large enough to erase the savings you
came for.

## Pattern syntax

| Form | Matches |
|---|---|
| `$NAME` | exactly one node, captured |
| `$_` | exactly one node, not captured |
| `$$$ARGS` | zero or more nodes, captured |
| `$$$` | zero or more nodes, not captured |

Metavariable names must be UPPERCASE. Repeating the same name forces the code to be
identical at both positions — `$A && $A()` only matches when both sides agree.

Patterns match **structure, not text**, so whitespace, line breaks, and comments are
ignored. That is exactly why this beats `sed` for code.

## Worked examples

```bash
# Rename every callsite, preserving arguments
ast-grep --pattern 'legacyFn($$$ARGS)' --rewrite 'newFn($$$ARGS)' --lang ts -U src/api/

# Delete every console.log regardless of argument shape
ast-grep --pattern 'console.log($$$)' --rewrite '' --lang ts -U src/

# Modernize to optional chaining; $A on both sides enforces identity
ast-grep --pattern '$A && $A()' --rewrite '$A?.()' --lang ts -U src/

# CommonJS require to a const binding
ast-grep --pattern '$F = require($M)' --rewrite 'const $F = require($M)' --lang js -U lib/

# Python: swap an assertion helper
ast-grep --pattern 'assertEquals($A, $B)' --rewrite 'assert $A == $B' --lang python -U tests/
```

## Verify afterwards

A structural rewrite is not self-verifying. After applying:

1. Re-run the search pattern — it should return nothing.
2. Run the type checker or test suite for the touched area.
3. `git diff --stat` to confirm the blast radius matches what you predicted.

If the count of changed files surprises you, revert (`git checkout --`) and narrow
the path or tighten the pattern.

## When ast-grep is the wrong tool

For a **symbol-aware rename** that must follow imports, re-exports, and shadowing,
use the LSP rename if one is available. `ast-grep` matches syntax, not scope — it
will happily rename an unrelated local variable that shares a name.
