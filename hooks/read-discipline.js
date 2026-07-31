#!/usr/bin/env node
'use strict';

// PreToolUse: Read
//
// Approximates oh-my-pi's structural-summary read. Claude Code cannot rewrite a
// tool result, but permissionDecisionReason IS delivered to the model - so the
// hook builds the outline itself and hands it back as the denial reason. The
// model gets the outline for the price of one denied call, then re-reads only
// the ranges it needs.

const fs = require('fs');
const outliner = require('./lib/outline');
const store = require('./lib/state');

const LINE_THRESHOLD = Number(process.env.OMP_LITE_READ_THRESHOLD || 400);
const MAX_BYTES = 4 * 1024 * 1024;

function main() {
  let raw = '';
  try { raw = fs.readFileSync(0, 'utf8'); } catch { return; }
  if (!raw.trim()) return;

  let payload;
  try { payload = JSON.parse(raw); } catch { return; }
  if (payload.tool_name !== 'Read') return;

  const input = payload.tool_input || {};
  const filePath = input.file_path;
  if (!filePath) return;

  // Already bounded - the model is doing the right thing.
  if (input.offset != null || input.limit != null) return;
  if (!outliner.isSource(filePath)) return;

  let stat;
  try { stat = fs.statSync(filePath); } catch { return; }
  if (!stat.isFile() || stat.size > MAX_BYTES) return;

  // Escape hatch: deny a given path at most once per session. A second request
  // means the model genuinely wants the file, and a denial loop would wedge it.
  const state = store.load(payload.session_id);
  state.reads = state.reads || {};
  if (state.reads[filePath]) return;

  let text;
  try { text = fs.readFileSync(filePath, 'utf8'); } catch { return; }

  const { lines, rows } = outliner.outline(text, filePath);
  if (lines < LINE_THRESHOLD) return;
  // A sparse outline misses the file's structure; the model would re-read anyway
  // and the denial would have cost a round trip for nothing.
  if (!outliner.covers(lines, rows.length)) return;

  state.reads[filePath] = 1;
  store.save(payload.session_id, state);

  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: 'PreToolUse',
      permissionDecision: 'deny',
      permissionDecisionReason: outliner.render(filePath, lines, rows),
    },
  }));
}

try { main(); } catch { /* never block a read */ }
process.exit(0);
