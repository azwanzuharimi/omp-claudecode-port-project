#!/usr/bin/env node
'use strict';

// PreToolUse: Edit|Write|MultiEdit|NotebookEdit|Bash
//
// Port of oh-my-pi's TTSR ("time-traveling stream rules"). Rules sit dormant and
// cost nothing until the model goes off-script, instead of taxing every request
// the way an always-on CLAUDE.md rule does.

const rulesLib = require('./lib/rules');
const store = require('./lib/state');

function readStdin() {
  try {
    return require('fs').readFileSync(0, 'utf8');
  } catch {
    return '';
  }
}

function main() {
  const raw = readStdin();
  if (!raw.trim()) return;

  let payload;
  try { payload = JSON.parse(raw); } catch { return; }

  const toolName = payload.tool_name;
  const input = payload.tool_input;
  const sessionId = payload.session_id;
  if (!toolName || !input) return;

  const rules = rulesLib.loadRules(payload.cwd);
  if (rules.length === 0) return;

  const state = store.load(sessionId);
  state.calls = (state.calls || 0) + 1;

  const hit = rulesLib.evaluate(rules, toolName, input, state);
  if (!hit) { store.save(sessionId, state); return; }

  const { rule, filePath } = hit;
  state.fired = state.fired || {};
  state.fired[rule.name] = state.calls;

  if (rule.interrupt) {
    store.save(sessionId, state);
    process.stdout.write(JSON.stringify({
      hookSpecificOutput: {
        hookEventName: 'PreToolUse',
        permissionDecision: 'deny',
        permissionDecisionReason: rulesLib.renderInterrupt(rule, filePath),
      },
    }));
    return;
  }

  // Soft mode: omp's reminder_target=tool_result. Let the call run, hand the
  // correction back through PostToolUse so it costs no extra round trip.
  state.pending = state.pending || [];
  state.pending.push(rulesLib.renderInterrupt(rule, filePath));
  store.save(sessionId, state);
}

try { main(); } catch { /* never block a tool call */ }
process.exit(0);
