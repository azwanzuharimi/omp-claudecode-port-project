#!/usr/bin/env node
'use strict';

// PostToolUse: drains soft-mode reminders queued by lazy-rules.js and delivers
// them as additionalContext, so a nudge costs no extra round trip.

const store = require('./lib/state');

function main() {
  let raw = '';
  try { raw = require('fs').readFileSync(0, 'utf8'); } catch { return; }
  if (!raw.trim()) return;

  let payload;
  try { payload = JSON.parse(raw); } catch { return; }

  const state = store.load(payload.session_id);
  const pending = state.pending || [];
  if (pending.length === 0) return;

  state.pending = [];
  store.save(payload.session_id, state);

  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: 'PostToolUse',
      additionalContext: pending.join('\n\n'),
    },
  }));
}

try { main(); } catch { /* never disrupt the hook chain */ }
process.exit(0);
