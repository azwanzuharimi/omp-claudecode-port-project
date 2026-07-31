'use strict';

const fs = require('fs');
const path = require('path');
const os = require('os');

const CONFIG_DIR = process.env.CLAUDE_CONFIG_DIR || path.join(os.homedir(), '.claude');
const STATE_DIR = path.join(CONFIG_DIR, 'state', 'omp-claudecode-port-project');

function sanitize(id) {
  return String(id || 'nosession').replace(/[^A-Za-z0-9._-]/g, '_').slice(0, 128);
}

function statePath(sessionId) {
  return path.join(STATE_DIR, sanitize(sessionId) + '.json');
}

function load(sessionId) {
  try {
    return JSON.parse(fs.readFileSync(statePath(sessionId), 'utf8'));
  } catch {
    return { calls: 0, fired: {}, pending: [], reads: {} };
  }
}

function save(sessionId, state) {
  try {
    fs.mkdirSync(STATE_DIR, { recursive: true });
    fs.writeFileSync(statePath(sessionId), JSON.stringify(state), 'utf8');
  } catch {
    // A state write failure must never block a tool call.
  }
}

module.exports = { STATE_DIR, statePath, load, save, sanitize };
