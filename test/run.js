#!/usr/bin/env node
'use strict';

const assert = require('assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { execFileSync } = require('child_process');

const ROOT = path.join(__dirname, '..');

// Isolate from the real ~/.claude BEFORE requiring the libs - they resolve
// CLAUDE_CONFIG_DIR at module load. Without this the suite picks up whatever
// rules the user actually has installed and every assertion becomes a lie.
const ISO = fs.mkdtempSync(path.join(os.tmpdir(), 'omp-claudecode-port-project-cfg-'));
fs.mkdirSync(path.join(ISO, 'rules'), { recursive: true });
process.env.CLAUDE_CONFIG_DIR = ISO;

const rules = require(path.join(ROOT, 'hooks/lib/rules'));
const outliner = require(path.join(ROOT, 'hooks/lib/outline'));

let pass = 0, fail = 0;
function t(name, fn) {
  try { fn(); pass++; console.log(`  ok   ${name}`); }
  catch (e) { fail++; console.log(`  FAIL ${name}\n       ${e.message}`); }
}

// These tests exercise the JS reference implementation, which is what the Rust
// binary is diffed against in test/differential.js. The JS hooks are not a
// runtime any more - install.sh only ever registers the binary.
const RUNTIME = process.execPath;

function runHook(script, payload, env = {}) {
  const out = execFileSync(RUNTIME, [path.join(ROOT, 'hooks', script)], {
    input: JSON.stringify(payload),
    env: { ...process.env, ...env },
    encoding: 'utf8',
  });
  return out.trim() ? JSON.parse(out) : null;
}

// ---------------------------------------------------------------- fixtures
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'omp-claudecode-port-project-test-'));
const ruleDir = path.join(tmp, 'proj', '.claude', 'rules');
fs.mkdirSync(ruleDir, { recursive: true });
fs.writeFileSync(path.join(ruleDir, 'no-pip.md'), `---
description: Use UV, never bare pip
condition: "(?<!uv )pip install"
scope: "tool:Bash, tool:Write(*.sh)"
repeat: once
interrupt: true
---
This project uses UV. Use \`uv add <pkg>\`.
`);
fs.writeFileSync(path.join(ruleDir, 'soft-todo.md'), `---
description: Flag TODO markers
condition: "TODO\\(nocommit\\)"
scope: "tool:Edit"
interrupt: false
---
Remove the nocommit marker before this lands.
`);
fs.writeFileSync(path.join(ruleDir, 'broken.md'), `---
description: unclosed regex group
condition: "foo("
---
never loads
`);

const loaded = rules.loadRules(path.join(tmp, 'proj'));

console.log('\nrule engine');

t('malformed rule file is skipped, others still load', () => {
  assert.strictEqual(loaded.length, 2, `expected 2 rules, got ${loaded.length}`);
  assert.ok(!loaded.some((r) => r.name === 'broken'));
});

t('matcherDigest reads only new content, never the whole file', () => {
  assert.strictEqual(rules.matcherDigest('Edit', { new_string: 'a', old_string: 'b' }), 'a');
  assert.strictEqual(rules.matcherDigest('Write', { content: 'c' }), 'c');
  assert.strictEqual(
    rules.matcherDigest('MultiEdit', { edits: [{ new_string: 'x' }, { new_string: 'y' }] }),
    'x\ny'
  );
});

t('rule fires on a violation in new content', () => {
  const hit = rules.evaluate(loaded, 'Bash', { command: 'pip install requests' }, { calls: 1, fired: {} });
  assert.ok(hit, 'expected a match');
  assert.strictEqual(hit.rule.name, 'no-pip');
});

t('same text already in the file does NOT fire (old_string ignored)', () => {
  const hit = rules.evaluate(loaded, 'Edit',
    { file_path: '/x/a.py', old_string: 'pip install requests', new_string: 'uv add requests' },
    { calls: 1, fired: {} });
  assert.strictEqual(hit, null, 'must not match on pre-existing content');
});

t('negative lookbehind lets the correct form through', () => {
  const hit = rules.evaluate(loaded, 'Bash', { command: 'uv pip install requests' }, { calls: 1, fired: {} });
  assert.strictEqual(hit, null);
});

t('scope gates by tool name', () => {
  const hit = rules.evaluate(loaded, 'Read', { command: 'pip install x' }, { calls: 1, fired: {} });
  assert.strictEqual(hit, null);
});

t('scope gates by path glob', () => {
  const mk = (fp) => rules.evaluate(loaded, 'Write', { file_path: fp, content: 'pip install x' }, { calls: 1, fired: {} });
  assert.ok(mk('/x/setup.sh'), 'should match *.sh');
  assert.strictEqual(mk('/x/setup.py'), null, 'should not match *.py');
});

t('fire-once: an already-fired rule is disarmed', () => {
  const state = { calls: 5, fired: { 'no-pip': 2 } };
  assert.strictEqual(rules.evaluate(loaded, 'Bash', { command: 'pip install x' }, state), null);
});

t('after-gap re-arms only after the gap elapses', () => {
  // By name, not by index: readdir order is runtime-dependent.
  const base = loaded.find((x) => x.name === 'no-pip');
  const r = { ...base, repeat: 'after-gap 3' };
  assert.strictEqual(rules.isArmed(r, { calls: 4, fired: { 'no-pip': 2 } }), false);
  assert.strictEqual(rules.isArmed(r, { calls: 5, fired: { 'no-pip': 2 } }), true);
});

t('rule order is deterministic regardless of readdir order', () => {
  // evaluate() fires the first match, so load order is user-visible behavior.
  const names = loaded.map((r) => r.name);
  assert.deepStrictEqual(names, [...names].sort(),
    `rules must load in sorted order, got: ${names.join(', ')}`);
});

// ---------------------------------------------------------------- hook I/O
console.log('\nlazy-rules hook');

const SID = 'test-' + process.pid;
const stateFile = path.join(ISO, 'state', 'omp-claudecode-port-project', SID + '.json');
function clearState() { try { fs.unlinkSync(stateFile); } catch {} }
clearState();

t('interrupt:true denies with the rule body in a system-interrupt envelope', () => {
  const out = runHook('lazy-rules.js', {
    session_id: SID, cwd: path.join(tmp, 'proj'),
    tool_name: 'Bash', tool_input: { command: 'pip install requests' },
  });
  assert.ok(out, 'expected output');
  const h = out.hookSpecificOutput;
  assert.strictEqual(h.permissionDecision, 'deny');
  assert.match(h.permissionDecisionReason, /<system-interrupt reason="rule_violation" rule="no-pip">/);
  assert.match(h.permissionDecisionReason, /uv add/);
});

t('second identical violation in the same session is silent (fire-once)', () => {
  const out = runHook('lazy-rules.js', {
    session_id: SID, cwd: path.join(tmp, 'proj'),
    tool_name: 'Bash', tool_input: { command: 'pip install flask' },
  });
  assert.strictEqual(out, null, 'expected no second denial');
});

t('interrupt:false never denies; PostToolUse delivers additionalContext', () => {
  clearState();
  const pre = runHook('lazy-rules.js', {
    session_id: SID, cwd: path.join(tmp, 'proj'),
    tool_name: 'Edit', tool_input: { file_path: '/x/a.js', new_string: 'x = 1 // TODO(nocommit)' },
  });
  assert.strictEqual(pre, null, 'soft rule must not deny');
  const post = runHook('lazy-rules-post.js', { session_id: SID });
  assert.ok(post, 'expected PostToolUse output');
  assert.match(post.hookSpecificOutput.additionalContext, /nocommit marker/);
  assert.strictEqual(post.hookSpecificOutput.hookEventName, 'PostToolUse');
});

t('pending queue drains - a second PostToolUse emits nothing', () => {
  assert.strictEqual(runHook('lazy-rules-post.js', { session_id: SID }), null);
});

t('empty stdin and malformed JSON exit clean with no output', () => {
  for (const body of ['', '{not json']) {
    const out = execFileSync(RUNTIME, [path.join(ROOT, 'hooks/lazy-rules.js')], { input: body, encoding: 'utf8' });
    assert.strictEqual(out.trim(), '');
  }
});

// ---------------------------------------------------------------- outline
console.log('\nread discipline');

const bigFile = path.join(tmp, 'big.py');
const body = [];
for (let i = 0; i < 60; i++) {
  body.push(`def func_${i}(a, b):`);
  for (let j = 0; j < 9; j++) body.push(`    x_${j} = a + b  # filler`);
}
fs.writeFileSync(bigFile, body.join('\n'));

const smallFile = path.join(tmp, 'small.py');
fs.writeFileSync(smallFile, 'def only():\n    return 1\n');

t('outline finds declarations with reusable 1-indexed line numbers', () => {
  const { lines, rows } = outliner.outline(fs.readFileSync(bigFile, 'utf8'), bigFile);
  assert.strictEqual(lines, 600);
  assert.strictEqual(rows.length, 60);
  assert.strictEqual(rows[0], '1:def func_0(a, b):');
  assert.strictEqual(rows[1], '11:def func_1(a, b):');
});

t('outline omits bodies', () => {
  const { rows } = outliner.outline(fs.readFileSync(bigFile, 'utf8'), bigFile);
  assert.ok(!rows.some((r) => r.includes('filler')), 'body lines leaked into outline');
});

t('large unbounded read is denied and carries the outline', () => {
  clearState();
  const out = runHook('read-discipline.js', {
    session_id: SID, tool_name: 'Read', tool_input: { file_path: bigFile },
  });
  assert.ok(out, 'expected a denial');
  const reason = out.hookSpecificOutput.permissionDecisionReason;
  assert.strictEqual(out.hookSpecificOutput.permissionDecision, 'deny');
  assert.match(reason, /600 lines/);
  assert.match(reason, /1:def func_0/);
  assert.match(reason, /Read\(file_path, offset, limit\)/);
});

t('escape hatch: the same path is never denied twice', () => {
  const out = runHook('read-discipline.js', {
    session_id: SID, tool_name: 'Read', tool_input: { file_path: bigFile },
  });
  assert.strictEqual(out, null, 'second read of the same path must go through');
});

t('small file is untouched', () => {
  clearState();
  assert.strictEqual(runHook('read-discipline.js', {
    session_id: SID, tool_name: 'Read', tool_input: { file_path: smallFile },
  }), null);
});

t('already-bounded read is untouched', () => {
  clearState();
  assert.strictEqual(runHook('read-discipline.js', {
    session_id: SID, tool_name: 'Read', tool_input: { file_path: bigFile, offset: 1, limit: 50 },
  }), null);
});

t('non-source file is untouched', () => {
  clearState();
  const bin = path.join(tmp, 'data.parquet');
  fs.writeFileSync(bin, 'x\n'.repeat(2000));
  assert.strictEqual(runHook('read-discipline.js', {
    session_id: SID, tool_name: 'Read', tool_input: { file_path: bin },
  }), null);
});

t('missing file does not crash the hook', () => {
  clearState();
  assert.strictEqual(runHook('read-discipline.js', {
    session_id: SID, tool_name: 'Read', tool_input: { file_path: '/nope/gone.py' },
  }), null);
});

// ---------------------------------------------------------------- latency
console.log('\nlatency');

t('lazy-rules stays under 150ms', () => {
  const s = Date.now();
  runHook('lazy-rules.js', {
    session_id: SID + '-perf', cwd: path.join(tmp, 'proj'),
    tool_name: 'Edit', tool_input: { file_path: '/x/a.js', new_string: 'const a = 1;' },
  });
  const ms = Date.now() - s;
  assert.ok(ms < 150, `took ${ms}ms`);
  console.log(`       (${ms}ms)`);
});

t('read-discipline stays under 150ms on a 600-line file', () => {
  const s = Date.now();
  runHook('read-discipline.js', {
    session_id: SID + '-perf2', tool_name: 'Read', tool_input: { file_path: bigFile },
  });
  const ms = Date.now() - s;
  assert.ok(ms < 150, `took ${ms}ms`);
  console.log(`       (${ms}ms)`);
});

t('coverage gate: sparse outline over a long file does not intercept', () => {
  assert.strictEqual(outliner.covers(800, 5), false, '5 decls in 800 lines is not representative');
  assert.strictEqual(outliner.covers(800, 10), true);
  // Requirement is max(4, lines/80): 420 lines needs 5, not the floor of 4.
  assert.strictEqual(outliner.covers(420, 5), true);
  assert.strictEqual(outliner.covers(420, 4), false);
  assert.strictEqual(outliner.covers(100, 4), true, 'the floor of 4 applies to short files');
});

t('a long file with almost no declarations is left alone', () => {
  clearState();
  const sparse = path.join(tmp, 'sparse.py');
  const body2 = ['def only_one():'];
  for (let i = 0; i < 800; i++) body2.push(`    x${i} = ${i}`);
  fs.writeFileSync(sparse, body2.join('\n'));
  assert.strictEqual(runHook('read-discipline.js', {
    session_id: SID, tool_name: 'Read', tool_input: { file_path: sparse },
  }), null, 'should not deny when the outline would teach nothing');
});

t('test-style calls are captured as declarations', () => {
  const spec = path.join(tmp, 'x.test.js');
  fs.writeFileSync(spec, [
    "describe('suite', () => {",
    "  it('does a thing', () => {",
    '    for (const x of ys) {',
    '      doWork(x);',
    '    }',
    '  });',
    "  test.each(cases)('case %s', (c) => {});",
    '});',
  ].join('\n'));
  const { rows } = outliner.outline(fs.readFileSync(spec, 'utf8'), spec);
  const text = rows.join('\n');
  assert.match(text, /describe\('suite'/);
  assert.match(text, /it\('does a thing'/);
  assert.match(text, /test\.each/);
  assert.ok(!text.includes('for (const x'), 'control flow must not be treated as a declaration');
});

// ---------------------------------------------------------- shipped rules
console.log('\nshipped rules (false-positive checks)');

const shippedDir = path.join(tmp, 'shipped', '.claude', 'rules');
fs.mkdirSync(shippedDir, { recursive: true });
for (const f of fs.readdirSync(path.join(ROOT, 'rules'))) {
  fs.copyFileSync(path.join(ROOT, 'rules', f), path.join(shippedDir, f));
}
const shipped = rules.loadRules(path.join(tmp, 'shipped'));
const fire = (tool, input) => rules.evaluate(shipped, tool, input, { calls: 1, fired: {} });
const named = (tool, input) => { const h = fire(tool, input); return h && h.rule.name; };

t('all shipped rules parse and load', () => {
  assert.strictEqual(shipped.length, 3, `loaded ${shipped.map((r) => r.name).join(',')}`);
});

t('no-bare-pip fires on the bad form', () => {
  assert.strictEqual(named('Bash', { command: 'pip install pandas' }), 'no-bare-pip');
  assert.strictEqual(named('Bash', { command: 'pip3 install -r requirements.txt' }), 'no-bare-pip');
});

t('no-bare-pip does NOT fire on uv forms or lookalikes', () => {
  for (const cmd of [
    'uv pip install pandas',
    'uv add pandas',
    'grep -r "pip install" docs/',   // still contains the phrase, but as a search term
  ]) {
    const hit = named('Bash', { command: cmd });
    if (cmd.startsWith('grep')) continue; // documented limitation, asserted separately below
    assert.strictEqual(hit, null, `over-fired on: ${cmd}`);
  }
});

t('no-bare-python fires on a bare interpreter call', () => {
  assert.strictEqual(named('Bash', { command: 'python scripts/load.py' }), 'no-bare-python');
  assert.strictEqual(named('Bash', { command: 'python3 main.py --dry-run' }), 'no-bare-python');
});

t('no-bare-python does NOT fire on uv run or non-.py targets', () => {
  for (const cmd of ['uv run scripts/load.py', 'python --version', 'which python3', './venv/bin/python x.py']) {
    assert.strictEqual(named('Bash', { command: cmd }), null, `over-fired on: ${cmd}`);
  }
});

t('secrets rule fires on real-shaped credentials', () => {
  assert.strictEqual(named('Write', { file_path: '/x/c.py', content: 'KEY = "AKIAIOSFODNN7EXAMPLE"' }), 'no-hardcoded-secrets');
  assert.strictEqual(named('Edit', { file_path: '/x/c.py', new_string: 'password = "hunter2swordfish"' }), 'no-hardcoded-secrets');
});

t('secrets rule does NOT fire on placeholders, env reads, or interpolation', () => {
  for (const s of [
    'api_key = os.environ["API_KEY"]',
    'password = "REPLACE_ME"',
    'api_key = f"{settings.key}"',
    'password = ""',
    'api_key: str',
  ]) {
    assert.strictEqual(named('Edit', { file_path: '/x/c.py', new_string: s }), null, `over-fired on: ${s}`);
  }
});

t('shipped rules ignore tools outside their scope', () => {
  assert.strictEqual(named('Read', { command: 'pip install x' }), null);
  assert.strictEqual(named('Bash', { command: 'echo "AKIAIOSFODNN7EXAMPLE"' }), null,
    'secrets rule is scoped to file writes, not Bash');
});

t('KNOWN LIMITATION: a rule pattern inside a search string still fires', () => {
  // Regex has no way to tell "running pip install" from "grepping for it".
  // Documented in README rather than papered over with a fragile negative lookahead.
  assert.strictEqual(named('Bash', { command: 'grep -r "pip install" docs/' }), 'no-bare-pip');
});

// -------------------------------------------------------------- config dir
console.log('\nconfig resolution');

t('CLAUDE_CONFIG_DIR is honored for rule discovery', () => {
  const alt = path.join(tmp, 'altconfig');
  fs.mkdirSync(path.join(alt, 'rules'), { recursive: true });
  fs.writeFileSync(path.join(alt, 'rules', 'alt.md'),
    '---\ndescription: alt\ncondition: "ZZTOP"\nscope: "tool:Bash"\n---\nalt rule body\n');
  const out = execFileSync(RUNTIME, ['-e',
    `const r=require(${JSON.stringify(path.join(ROOT, 'hooks/lib/rules'))});` +
    `console.log(r.loadRules('/nonexistent').map(x=>x.name).join(','))`,
  ], { env: { ...process.env, CLAUDE_CONFIG_DIR: alt }, encoding: 'utf8' }).trim();
  assert.strictEqual(out, 'alt');
});

t('project rule shadows a user rule of the same filename', () => {
  const alt = path.join(tmp, 'shadowcfg');
  const proj = path.join(tmp, 'shadowproj');
  fs.mkdirSync(path.join(alt, 'rules'), { recursive: true });
  fs.mkdirSync(path.join(proj, '.claude', 'rules'), { recursive: true });
  fs.writeFileSync(path.join(alt, 'rules', 'dup.md'),
    '---\ndescription: user\ncondition: "QQQ"\nscope: "tool:Bash"\n---\nUSER VERSION\n');
  fs.writeFileSync(path.join(proj, '.claude', 'rules', 'dup.md'),
    '---\ndescription: project\ncondition: "QQQ"\nscope: "tool:Bash"\n---\nPROJECT VERSION\n');
  const out = execFileSync(RUNTIME, ['-e',
    `const r=require(${JSON.stringify(path.join(ROOT, 'hooks/lib/rules'))});` +
    `const rs=r.loadRules(${JSON.stringify(proj)});` +
    `console.log(rs.length + '|' + rs[0].body)`,
  ], { env: { ...process.env, CLAUDE_CONFIG_DIR: alt }, encoding: 'utf8' }).trim();
  assert.strictEqual(out, '1|PROJECT VERSION');
});

// ---------------------------------------------------------------- teardown
clearState();
fs.rmSync(ISO, { recursive: true, force: true });
fs.rmSync(tmp, { recursive: true, force: true });

console.log(`\n${pass} passed, ${fail} failed\n`);
process.exit(fail ? 1 : 0);
