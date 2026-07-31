---
name: bounded-output
description: Use before running a Bash command whose output could be long - git log, npm/pip list, pytest, terraform plan, find, unbounded rg/grep, docker logs, large SQL results, cat on a big file. Captures the full output to a file and lets only a bounded head into context. Do not use for commands with small, known-size output.
---

# Keep large command output out of context

Every line a command prints is billed on this request **and on every request after
it** for the rest of the session. A 3,000-line `pytest` run costs far more than the
one number you actually wanted from it.

oh-my-pi solves this by capturing bash output to an artifact and paging in only what
the model asks for. The same discipline, done by hand:

## The pattern

```bash
OUT=/private/tmp/claude-501/-Users-azwan/44db8de5-00ed-4556-b20c-dade1c175a72/scratchpad/run.log
<command> > "$OUT" 2>&1; echo "exit=$?  lines=$(wc -l < "$OUT")  -> $OUT"
tail -30 "$OUT"
```

Then read more **only if the tail did not answer the question**, using
`Read(file_path, offset, limit)` so you page in a window rather than the whole log.

## Choose the right slice

The default of `tail` is wrong about as often as it is right:

| What you need | Slice |
|---|---|
| Did it pass or fail | `tail -30` — the summary lives at the end |
| Where it broke | `grep -nE 'Error\|FAILED\|Traceback\|panic' "$OUT" \| head -40` |
| What it did first | `head -40` |
| A count, not the content | `wc -l`, or `grep -c <pattern>` |

If you only need a count or a yes/no, do not print the body at all.

## Commands worth bounding

`git log`, `git diff` on a wide change, `npm ls`, `pip list`, `pytest`, `terraform plan`,
`find`, `rg` without `--max-count`/`head`, `docker logs`, `kubectl get -A`, `du -a`,
`aws s3 ls --recursive`, any `SELECT` without a `LIMIT`.

## Bound it at the source when you can

Cheaper than capturing and slicing:

```bash
git log --oneline -20                 # not: git log
rg -n 'pattern' src/ | head -50       # not: rg -n 'pattern'
pytest -q --tb=line                   # not: pytest -v
kubectl get pods -n prod              # not: kubectl get pods -A
find src -name '*.ts' | head -50      # not: find . -name '*.ts'
```

## What not to do

Do not pipe through `head` when you may need the rest — you throw the output away
and have to re-run the command, which costs more than capturing did. Capture the
full output to a file first, then slice. The file is free; context is not.
