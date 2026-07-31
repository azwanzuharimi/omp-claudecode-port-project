#!/usr/bin/env node
'use strict';

// Differential test: the JS hooks and the Rust binary must produce BYTE-IDENTICAL
// stdout for every payload. This is the only thing that makes the Rust port
// trustworthy - the dangerous failure mode (a regex dialect divergence) is silent,
// and only a comparison like this catches it.
//
// Usage: node test/differential.js [path/to/omp-hooks]
//
// Note: credential-shaped fixtures below are assembled by concatenation so no
// secret-shaped literal ever appears in this file. They are fake test vectors.

const assert = require('assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { execFileSync } = require('child_process');

const ROOT = path.join(__dirname, '..');

function findBinary() {
  if (process.argv[2]) return process.argv[2];
  for (const p of ['rust/target/release/omp-hooks', 'rust/target/debug/omp-hooks']) {
    const abs = path.join(ROOT, p);
    if (fs.existsSync(abs)) return abs;
  }
  return null;
}

const BIN = findBinary();
if (!BIN) {
  console.error('omp-hooks binary not found. Build it first:');
  console.error('  cd rust && cargo build --release');
  process.exit(2);
}

// Isolated config dir so the corpus is the only rule set in play.
const ISO = fs.mkdtempSync(path.join(os.tmpdir(), 'ompdiff-cfg-'));
const WORK = fs.mkdtempSync(path.join(os.tmpdir(), 'ompdiff-work-'));
const RULES = path.join(ISO, 'rules');
fs.mkdirSync(RULES, { recursive: true });

// ---------------------------------------------------------------- rule corpus
// The three shipped rules, verbatim - these are the ones that actually matter.
for (const f of fs.readdirSync(path.join(ROOT, 'rules'))) {
  fs.copyFileSync(path.join(ROOT, 'rules', f), path.join(RULES, f));
}

// Plus adversarial rules aimed squarely at JS-vs-Rust regex divergence.
const extraRules = {
  'zz-soft.md': `---
description: soft mode
condition: "SOFTMARK"
scope: "tool:Edit"
interrupt: false
---
Soft reminder body.
`,
  'zz-unicode-w.md': `---
description: word-class semantics
condition: "\\bfoo\\w+"
scope: "tool:Write"
---
The word classes must stay ASCII, as JS defines them.
`,
  'zz-dot.md': `---
description: dot semantics
condition: "a.b"
scope: "tool:Write"
---
Dot must exclude only the characters JS excludes.
`,
  'zz-digit.md': `---
description: digit class
condition: "id-\\d\\d\\d"
scope: "tool:Write"
---
The digit class must be ASCII 0-9 only.
`,
  'zz-icase.md': `---
description: case folding
condition: "stra\\u00dfe"
flags: "i"
scope: "tool:Write"
---
Case-insensitive folding must match JS.
`,
  'zz-glob.md': `---
description: scoped by glob
condition: "GLOBHIT"
scope: "tool:Write(*.rs), tool:Edit(src/**)"
---
Scoped rule body.
`,
  'zz-gap.md': `---
description: after-gap rearm
condition: "GAPMARK"
scope: "tool:Bash"
repeat: after-gap 2
---
Gap rule body.
`,
  'zz-broken.md': `---
description: uncompilable
condition: "foo("
scope: "tool:Bash"
---
Never loads.
`,
  'zz-noscope.md': `---
description: no scope means every tool
condition: "ANYTOOLMARK"
---
Unscoped rule body.
`,
};
for (const [name, body] of Object.entries(extraRules)) {
  fs.writeFileSync(path.join(RULES, name), body);
}

// ---------------------------------------------------------------- file corpus
const bigPy = path.join(WORK, 'big.py');
{
  const L = [];
  for (let i = 0; i < 60; i++) {
    L.push(`def func_${i}(a, b):`);
    for (let j = 0; j < 9; j++) L.push(`    x_${j} = a + b  # filler`);
  }
  fs.writeFileSync(bigPy, L.join('\n'));
}

const bigTs = path.join(WORK, 'big.ts');
{
  const L = [];
  for (let i = 0; i < 50; i++) {
    L.push(`export function fn${i}(a: string): number {`);
    L.push(`  const v = a.length;`);
    for (let j = 0; j < 8; j++) L.push(`  // filler ${j}`);
    L.push('}');
  }
  fs.writeFileSync(bigTs, L.join('\n'));
}

const sparse = path.join(WORK, 'sparse.py');
{
  const L = ['def only_one():'];
  for (let i = 0; i < 800; i++) L.push(`    x${i} = ${i}`);
  fs.writeFileSync(sparse, L.join('\n'));
}

const smallPy = path.join(WORK, 'small.py');
fs.writeFileSync(smallPy, 'def only():\n    return 1\n');

const unicodeJs = path.join(WORK, 'uni.js');
{
  // Trailing whitespace, astral chars, CRLF, long lines, a trailing newline -
  // every place a UTF-16-vs-chars or line-splitting difference could show up.
  const L = [];
  for (let i = 0; i < 45; i++) {
    L.push(`function café_${i}(x) {   `);
    L.push(`  const emoji = "\u{1F600}\u{1F680}";\r`);
    L.push(`  const long = "${'z'.repeat(500)}";`);
    L.push('}');
  }
  L.push('');
  fs.writeFileSync(unicodeJs, L.join('\n'));
}

const noExt = path.join(WORK, 'Makefile');
fs.writeFileSync(noExt, 'all:\n\techo hi\n'.repeat(300));

const dotfile = path.join(WORK, '.bashrc');
fs.writeFileSync(dotfile, 'export X=1\n'.repeat(500));

// ---------------------------------------------------------------- run helpers
const JS_HOOK = {
  'lazy-rules': path.join(ROOT, 'hooks/lazy-rules.js'),
  'lazy-rules-post': path.join(ROOT, 'hooks/lazy-rules-post.js'),
  'read-discipline': path.join(ROOT, 'hooks/read-discipline.js'),
};

function clearState() {
  fs.rmSync(path.join(ISO, 'state'), { recursive: true, force: true });
}

function runJs(hook, input) {
  return execFileSync(process.execPath, [JS_HOOK[hook]], {
    input, encoding: 'utf8',
    env: { ...process.env, CLAUDE_CONFIG_DIR: ISO },
  });
}

function runRust(hook, input) {
  return execFileSync(BIN, [hook], {
    input, encoding: 'utf8',
    env: { ...process.env, CLAUDE_CONFIG_DIR: ISO },
  });
}

let pass = 0, fail = 0;

/** Runs a whole sequence of calls against one engine, from clean state. */
function sequence(runner, steps) {
  clearState();
  return steps.map(([hook, payload]) =>
    runner(hook, typeof payload === 'string' ? payload : JSON.stringify(payload)));
}

function diff(name, steps) {
  let jsOut, rsOut;
  try {
    jsOut = sequence(runJs, steps);
    rsOut = sequence(runRust, steps);
  } catch (e) {
    fail++;
    console.log(`  FAIL ${name}\n       threw: ${e.message}`);
    return;
  }
  try {
    assert.deepStrictEqual(rsOut, jsOut);
    pass++;
    console.log(`  ok   ${name}`);
  } catch {
    fail++;
    const detail = jsOut.map((j, i) =>
      j === rsOut[i] ? null
        : `       step ${i}:\n         js  : ${JSON.stringify(j).slice(0, 200)}\n         rust: ${JSON.stringify(rsOut[i]).slice(0, 200)}`
    ).filter(Boolean).join('\n');
    console.log(`  FAIL ${name}\n${detail}`);
  }
}

const S = (n) => `diff-${n}`;
const edit = (fp, ns, sid) => ['lazy-rules', { session_id: sid, cwd: WORK, tool_name: 'Edit', tool_input: { file_path: fp, new_string: ns, old_string: 'OLDCONTENT' } }];
const write = (fp, c, sid) => ['lazy-rules', { session_id: sid, cwd: WORK, tool_name: 'Write', tool_input: { file_path: fp, content: c } }];
const bash = (cmd, sid) => ['lazy-rules', { session_id: sid, cwd: WORK, tool_name: 'Bash', tool_input: { command: cmd } }];
const read = (fp, sid, extra = {}) => ['read-discipline', { session_id: sid, tool_name: 'Read', tool_input: { file_path: fp, ...extra } }];
const post = (sid) => ['lazy-rules-post', { session_id: sid }];

// Fake credential-shaped test vectors, assembled so no literal appears in source.
const P = 'p' + 'ip in' + 'stall';
const FAKE_AWS = 'AKIA' + 'IOSFODNN7' + 'EXAMPLE';
const FAKE_PW = 'hunter2' + 'swordfish';

console.log(`\ndifferential: node vs ${path.relative(ROOT, BIN)}\n`);

// --- lazy-rules: matching and non-matching -----------------------------------
diff('bash: rule hit (deny)',            [bash(`${P} pandas`, S(1))]);
diff('bash: uv form, no hit',            [bash(`uv ${P} pandas`, S(2))]);
diff('bash: unrelated command',          [bash('ls -la /tmp', S(3))]);
diff('bash: pattern inside a search str', [bash(`grep -r "${P}" docs/`, S(4))]);
diff('fire-once: same violation twice',  [bash(`${P} a`, S(5)), bash(`${P} b`, S(5))]);
diff('after-gap: fires, waits, re-arms',
  [bash('GAPMARK', S(6)), bash('GAPMARK', S(6)), bash('GAPMARK', S(6)), bash('GAPMARK', S(6))]);

// --- matcherDigest per tool --------------------------------------------------
diff('Edit: new_string matches, old ignored', [edit('/x/a.py', 'GLOBHIT ANYTOOLMARK', S(7))]);
diff('Edit: violation only in old_string', [
  ['lazy-rules', { session_id: S(8), cwd: WORK, tool_name: 'Edit',
    tool_input: { file_path: '/x/a.py', old_string: 'ANYTOOLMARK', new_string: 'clean' } }]]);
diff('Write: content matches',            [write('/x/a.rs', 'GLOBHIT here', S(9))]);
diff('Write: glob miss (.py vs *.rs)',    [write('/x/a.py', 'GLOBHIT here', S(10))]);
diff('MultiEdit: joins every new_string', [
  ['lazy-rules', { session_id: S(11), cwd: WORK, tool_name: 'MultiEdit',
    tool_input: { file_path: '/x/a.py', edits: [{ new_string: 'clean' }, { new_string: 'ANYTOOLMARK' }] } }]]);
diff('MultiEdit: empty edits array', [
  ['lazy-rules', { session_id: S(12), cwd: WORK, tool_name: 'MultiEdit', tool_input: { file_path: '/x/a.py', edits: [] } }]]);
diff('NotebookEdit: new_source + notebook_path', [
  ['lazy-rules', { session_id: S(13), cwd: WORK, tool_name: 'NotebookEdit',
    tool_input: { notebook_path: '/x/n.ipynb', new_source: 'ANYTOOLMARK' } }]]);
diff('unscoped rule matches an unlisted tool', [
  ['lazy-rules', { session_id: S(14), cwd: WORK, tool_name: 'Bash', tool_input: { command: 'ANYTOOLMARK' } }]]);

// --- regex dialect: the silent-divergence class ------------------------------
diff('word class ASCII (foobar)',        [write('/x/a.rs', 'foobar', S(20))]);
diff('word class non-ASCII (fooebar)',   [write('/x/a.rs', 'fooébar', S(21))]);
diff('digit class ASCII (id-123)',       [write('/x/a.rs', 'id-123', S(22))]);
diff('digit class Arabic-Indic',         [write('/x/a.rs', 'id-١٢٣', S(23))]);
diff('dot excludes newline',             [write('/x/a.rs', 'a\nb', S(24))]);
diff('dot vs U+2028',                    [write('/x/a.rs', 'a b', S(25))]);
diff('case folding: STRASSE',            [write('/x/a.rs', 'STRASSE', S(26))]);
diff('case folding: lowercase sharp-s',  [write('/x/a.rs', 'straße', S(27))]);
diff('case folding: capital sharp-s',    [write('/x/a.rs', 'STRAẞE', S(28))]);
diff('secrets: key-shaped vector',       [write('/x/c.py', `KEY = "${FAKE_AWS}"`, S(29))]);
diff('secrets: placeholder ignored',     [write('/x/c.py', 'password = "REPLACE_ME"', S(30))]);
diff('secrets: env read ignored',        [write('/x/c.py', 'api_key = os.environ["API_KEY"]', S(31))]);
diff('secrets: uppercase keyword (i)',   [write('/x/c.py', `PASSWORD = "${FAKE_PW}"`, S(32))]);

// --- soft mode + PostToolUse -------------------------------------------------
diff('soft rule: no deny, then post delivers', [edit('/x/a.js', 'SOFTMARK', S(40)), post(S(40))]);
diff('post drains: second call silent',        [edit('/x/a.js', 'SOFTMARK', S(41)), post(S(41)), post(S(41))]);
diff('post with nothing pending',              [post(S(42))]);

// --- malformed / hostile input ----------------------------------------------
diff('empty stdin',            [['lazy-rules', '']]);
diff('whitespace stdin',       [['lazy-rules', '   \n  ']]);
diff('malformed JSON',         [['lazy-rules', '{not json']]);
diff('JSON but not an object', [['lazy-rules', '[1,2,3]']]);
diff('missing tool_input',     [['lazy-rules', { session_id: S(50), cwd: WORK, tool_name: 'Bash' }]]);
diff('missing tool_name',      [['lazy-rules', { session_id: S(51), cwd: WORK, tool_input: { command: 'x' } }]]);
diff('null tool_input',        [['lazy-rules', { session_id: S(52), cwd: WORK, tool_name: 'Bash', tool_input: null }]]);
diff('tool_input is a string', [['lazy-rules', { session_id: S(53), cwd: WORK, tool_name: 'Bash', tool_input: 'nope' }]]);
diff('no session_id at all',   [['lazy-rules', { cwd: WORK, tool_name: 'Bash', tool_input: { command: `${P} x` } }]]);
diff('weird session_id chars', [bash(`${P} x`, 'a/b\\c:d*?<>|"é')]);
diff('empty stdin (post)',     [['lazy-rules-post', '']]);
diff('empty stdin (read)',     [['read-discipline', '']]);

// --- read-discipline ---------------------------------------------------------
diff('read: big python -> outline',      [read(bigPy, S(60))]);
diff('read: big typescript -> outline',  [read(bigTs, S(61))]);
diff('read: unicode/CRLF/long lines',    [read(unicodeJs, S(62))]);
diff('read: deny-once, second passes',   [read(bigPy, S(63)), read(bigPy, S(63))]);
diff('read: small file untouched',       [read(smallPy, S(64))]);
diff('read: sparse outline untouched',   [read(sparse, S(65))]);
diff('read: bounded with offset',        [read(bigPy, S(66), { offset: 1 })]);
diff('read: bounded with limit',         [read(bigPy, S(67), { limit: 50 })]);
diff('read: null offset is not bounded', [read(bigPy, S(68), { offset: null })]);
diff('read: no extension (Makefile)',    [read(noExt, S(69))]);
diff('read: dotfile (.bashrc)',          [read(dotfile, S(70))]);
diff('read: missing file',               [read(path.join(WORK, 'nope.py'), S(71))]);
diff('read: directory not a file',       [read(WORK, S(72))]);
diff('read: wrong tool_name',            [['read-discipline', { session_id: S(73), tool_name: 'Edit', tool_input: { file_path: bigPy } }]]);
diff('read: missing file_path',          [['read-discipline', { session_id: S(74), tool_name: 'Read', tool_input: {} }]]);

// --- large payloads ----------------------------------------------------------
// A 20 MB piped payload drove bun's fs.readFileSync(0) to 6.5 GB RSS and an OOM.
// bun is no longer a supported runtime because of it, but the binary and the node
// reference must both stay bounded on big input. Claude Code pipes hook payloads,
// so `input:` here exercises the path that actually matters.
for (const mb of [1, 4]) {
  diff(`${mb}MB payload stays bounded and agrees`, [
    ['lazy-rules', { session_id: S(85 + mb), cwd: WORK, tool_name: 'Bash',
      tool_input: { command: 'x'.repeat(mb * 1024 * 1024) } }]]);
}
diff('large payload that DOES match a rule', [
  ['lazy-rules', { session_id: S(89), cwd: WORK, tool_name: 'Bash',
    tool_input: { command: 'x'.repeat(1024 * 1024) + ` ${P} pandas` } }]]);

// --- threshold via env -------------------------------------------------------
{
  const name = 'read: OMP_PORT_READ_THRESHOLD respected';
  const env = { ...process.env, CLAUDE_CONFIG_DIR: ISO, OMP_PORT_READ_THRESHOLD: '2' };
  const inp = JSON.stringify({ session_id: S(80), tool_name: 'Read', tool_input: { file_path: smallPy } });
  clearState();
  const j = execFileSync(process.execPath, [JS_HOOK['read-discipline']], { input: inp, encoding: 'utf8', env });
  clearState();
  const r = execFileSync(BIN, ['read-discipline'], { input: inp, encoding: 'utf8', env });
  if (j === r) { pass++; console.log(`  ok   ${name}`); }
  else { fail++; console.log(`  FAIL ${name}\n         js  : ${JSON.stringify(j)}\n         rust: ${JSON.stringify(r)}`); }
}

// --- state file interoperability --------------------------------------------
{
  const name = 'state file is interoperable between js and rust';
  const sid = S(90);
  clearState();
  runRust('lazy-rules', JSON.stringify(bash(`${P} a`, sid)[1]));   // rust fires + records
  const jsSecond = runJs('lazy-rules', JSON.stringify(bash(`${P} b`, sid)[1]));
  clearState();
  runJs('lazy-rules', JSON.stringify(bash(`${P} a`, sid)[1]));     // js fires + records
  const rustSecond = runRust('lazy-rules', JSON.stringify(bash(`${P} b`, sid)[1]));
  if (jsSecond === '' && rustSecond === '') { pass++; console.log(`  ok   ${name}`); }
  else {
    fail++;
    console.log(`  FAIL ${name}\n         js after rust : ${JSON.stringify(jsSecond)}\n         rust after js : ${JSON.stringify(rustSecond)}`);
  }
}

// ------------------------------------------------------------------- teardown
fs.rmSync(ISO, { recursive: true, force: true });
fs.rmSync(WORK, { recursive: true, force: true });

console.log(`\n${pass} identical, ${fail} divergent\n`);
process.exit(fail ? 1 : 0);
