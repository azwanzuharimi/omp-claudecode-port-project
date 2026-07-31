'use strict';

const fs = require('fs');
const path = require('path');
const os = require('os');

const CONFIG_DIR = process.env.CLAUDE_CONFIG_DIR || path.join(os.homedir(), '.claude');

// Later entries shadow earlier ones by filename, so a project rule overrides a user rule.
const RULE_DIRS = (cwd) => [
  { scope: 'user', dir: path.join(CONFIG_DIR, 'rules') },
  { scope: 'project', dir: path.join(cwd || process.cwd(), '.claude', 'rules') },
];

function parseFrontmatter(text) {
  if (!text.startsWith('---')) return null;
  const end = text.indexOf('\n---', 3);
  if (end === -1) return null;
  const raw = text.slice(text.indexOf('\n', 3) + 1, end);
  const body = text.slice(text.indexOf('\n', end + 1) + 1);
  const meta = {};
  for (const line of raw.split('\n')) {
    const m = line.match(/^([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*)$/);
    if (!m) continue;
    let v = m[2].trim();
    if (
      (v.startsWith('"') && v.endsWith('"') && v.length > 1) ||
      (v.startsWith("'") && v.endsWith("'") && v.length > 1)
    ) {
      v = v.slice(1, -1);
    }
    meta[m[1]] = v;
  }
  return { meta, body: body.trim() };
}

// "tool:Bash, tool:Write(*.sh)" -> [{tool:'Bash', glob:null}, {tool:'Write', glob:'*.sh'}]
function parseScope(scope) {
  if (!scope) return [];
  const out = [];
  for (const part of scope.split(',')) {
    const m = part.trim().match(/^tool:([A-Za-z_][A-Za-z0-9_]*)\s*(?:\(([^)]*)\))?$/);
    if (m) out.push({ tool: m[1], glob: m[2] ? m[2].trim() : null });
  }
  return out;
}

function globToRegExp(glob) {
  let re = '';
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === '*') {
      if (glob[i + 1] === '*') { re += '.*'; i++; } else { re += '[^/]*'; }
    } else if (c === '?') re += '[^/]';
    else re += c.replace(/[.+^${}()|[\]\\]/g, '\\$&');
  }
  return new RegExp('^' + re + '$');
}

function globMatches(glob, filePath) {
  if (!glob) return true;
  if (!filePath) return false;
  const re = globToRegExp(glob);
  return re.test(filePath) || re.test(path.basename(filePath));
}

/**
 * omp calls this the matcherDigest: ONLY the new content the call introduces.
 * Matching pre-existing file content over-fires on every unrelated edit to a file
 * that happens to contain the pattern somewhere.
 */
function matcherDigest(toolName, input) {
  if (!input || typeof input !== 'object') return '';
  switch (toolName) {
    case 'Edit': return String(input.new_string ?? '');
    case 'Write': return String(input.content ?? '');
    case 'MultiEdit':
      return Array.isArray(input.edits)
        ? input.edits.map((e) => String(e && e.new_string ? e.new_string : '')).join('\n')
        : '';
    case 'NotebookEdit': return String(input.new_source ?? '');
    case 'Bash': return String(input.command ?? '');
    default: return '';
  }
}

function targetPath(toolName, input) {
  if (!input || typeof input !== 'object') return null;
  if (toolName === 'NotebookEdit') return input.notebook_path || null;
  return input.file_path || null;
}

function loadRules(cwd) {
  const byName = new Map();
  for (const { dir } of RULE_DIRS(cwd)) {
    let entries;
    try { entries = fs.readdirSync(dir); } catch { continue; }
    for (const f of entries) {
      if (!f.endsWith('.md')) continue;
      let text;
      try { text = fs.readFileSync(path.join(dir, f), 'utf8'); } catch { continue; }
      const parsed = parseFrontmatter(text);
      if (!parsed || !parsed.meta.condition) continue;
      // The condition is taken verbatim - no YAML escape processing - so write
      // single backslashes (\w, \s), not the doubled form.
      let condition;
      try { condition = new RegExp(parsed.meta.condition, parsed.meta.flags || ''); } catch { continue; }
      // Later dirs (project) shadow earlier ones (user) by filename.
      byName.set(f, {
        name: f.replace(/\.md$/, ''),
        description: parsed.meta.description || '',
        condition,
        scope: parseScope(parsed.meta.scope),
        repeat: parsed.meta.repeat || 'once',
        interrupt: String(parsed.meta.interrupt ?? 'true') !== 'false',
        body: parsed.body,
      });
    }
  }
  return [...byName.values()];
}

function scopeAllows(rule, toolName, filePath) {
  if (rule.scope.length === 0) return true;
  return rule.scope.some((s) => s.tool === toolName && globMatches(s.glob, filePath));
}

function isArmed(rule, state) {
  const fired = state.fired && state.fired[rule.name];
  if (fired === undefined) return true;
  const m = /^after-gap\s+(\d+)$/.exec(rule.repeat);
  if (!m) return false; // "once"
  return (state.calls || 0) - fired >= Number(m[1]);
}

function evaluate(rules, toolName, input, state) {
  const digest = matcherDigest(toolName, input);
  if (!digest) return null;
  const filePath = targetPath(toolName, input);
  for (const rule of rules) {
    if (!scopeAllows(rule, toolName, filePath)) continue;
    if (!isArmed(rule, state)) continue;
    if (!rule.condition.test(digest)) continue;
    return { rule, filePath };
  }
  return null;
}

function renderInterrupt(rule, filePath) {
  return (
    `<system-interrupt reason="rule_violation" rule="${rule.name}"` +
    (filePath ? ` path="${filePath}"` : '') +
    `>\n${rule.body}\n</system-interrupt>`
  );
}

module.exports = {
  parseFrontmatter, parseScope, globToRegExp, globMatches,
  matcherDigest, targetPath, loadRules, scopeAllows, isArmed,
  evaluate, renderInterrupt,
};
