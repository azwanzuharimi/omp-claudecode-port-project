'use strict';

const path = require('path');

// Declaration patterns per language. Deliberately regex over a spawned parser:
// this hook fires on every Read, so a subprocess round-trip would cost more than
// the tokens it saves. ast-grep is used where it genuinely wins (see skills/codemod).
const LANGS = {
  js: [/^\s*(export\s+)?(default\s+)?(async\s+)?function\s*\*?\s*[\w$]+/, /^\s*(export\s+)?(abstract\s+)?class\s+[\w$]+/, /^\s*(export\s+)?(const|let|var)\s+[\w$]+\s*=\s*(async\s*)?(\([^)]*\)|[\w$]+)\s*=>/, /^\s*(export\s+)?(interface|type|enum)\s+[\w$]+/, /^\s*(describe|it|test|suite|context|bench)(\.\w+)?\s*(\([^)]*\)\s*)?\(\s*[`'"]/],
  py: [/^\s*(async\s+)?def\s+\w+/, /^\s*class\s+\w+/, /^\s*@\w[\w.]*/],
  go: [/^\s*func\s+/, /^\s*type\s+\w+/, /^\s*(var|const)\s*\(/],
  rs: [/^\s*(pub\s+)?(async\s+)?fn\s+\w+/, /^\s*(pub\s+)?(struct|enum|trait|impl|mod)\s/, /^\s*(pub\s+)?type\s+\w+/],
  java: [/^\s*(public|private|protected|static|final|abstract|\s)*(class|interface|enum|record)\s+\w+/, /^\s*(public|private|protected|static|final|abstract|synchronized|\s)+[\w<>\[\],.\s]+\s+\w+\s*\([^;]*\)\s*\{?\s*$/],
  rb: [/^\s*(def|class|module)\s+/],
  php: [/^\s*(abstract\s+|final\s+)?(class|interface|trait)\s+\w+/, /^\s*(public|private|protected|static|\s)*function\s+\w+/],
  sh: [/^\s*(function\s+)?[\w-]+\s*\(\)\s*\{/],
  sql: [/^\s*(CREATE|ALTER|DROP)\s+/i, /^\s*(WITH|SELECT|INSERT|UPDATE|DELETE)\s/i],
  swift: [/^\s*(public\s+|private\s+|internal\s+|open\s+|fileprivate\s+)?(final\s+)?(func|class|struct|enum|protocol|extension|var|let)\s+\w+/],
  c: [/^\s*[\w*\s]+\s+\**\w+\s*\([^;]*\)\s*\{?\s*$/, /^\s*(typedef|struct|enum|union)\s+\w*/, /^\s*#(define|include)\s/],
};

const EXT_MAP = {
  '.js': 'js', '.jsx': 'js', '.mjs': 'js', '.cjs': 'js', '.ts': 'js', '.tsx': 'js',
  '.py': 'py', '.go': 'go', '.rs': 'rs', '.java': 'java', '.kt': 'java', '.scala': 'java',
  '.rb': 'rb', '.php': 'php', '.sh': 'sh', '.bash': 'sh', '.zsh': 'sh',
  '.sql': 'sql', '.swift': 'swift',
  '.c': 'c', '.h': 'c', '.cc': 'c', '.cpp': 'c', '.hpp': 'c', '.cs': 'java',
};

function langFor(filePath) {
  return EXT_MAP[path.extname(String(filePath || '')).toLowerCase()] || null;
}

function isSource(filePath) {
  return langFor(filePath) !== null;
}

/**
 * Returns { lines, rows } where rows are "LINE:signature" strings.
 * Line numbers are 1-indexed so they drop straight into Read(offset, limit).
 */
function outline(text, filePath, maxRows = 200) {
  const lang = langFor(filePath);
  const lines = text.split('\n');
  if (!lang) return { lines: lines.length, rows: [] };
  const pats = LANGS[lang];
  const rows = [];
  for (let i = 0; i < lines.length && rows.length < maxRows; i++) {
    const line = lines[i];
    if (!line.trim()) continue;
    if (line.length > 400) continue;
    if (pats.some((p) => p.test(line))) {
      rows.push(`${i + 1}:${line.replace(/\s+$/, '').slice(0, 160)}`);
    }
  }
  return { lines: lines.length, rows };
}

/**
 * Is this outline dense enough to represent the file?
 *
 * A handful of declarations across a long file means the outline misses the
 * file's real structure - the model would just re-read the whole thing, and the
 * denial cost a round trip for nothing. Better to let the read through.
 */
function covers(lines, rowCount) {
  return rowCount >= Math.max(4, Math.floor(lines / 80));
}

/** omp's recovery footer: teaches the cheap re-read instead of a whole-file retry. */
function render(filePath, lines, rows) {
  const head = `[${filePath}] ${lines} lines - structural outline only, bodies elided.`;
  const body = rows.join('\n');
  const foot =
    `\n\n[Re-read only what you need: Read(file_path, offset, limit) using the line ` +
    `numbers above. If you genuinely need the whole file, just call Read again on ` +
    `this path - it will go through.]`;
  return `${head}\n\n${body}${foot}`;
}

module.exports = { langFor, isSource, outline, render, covers, LANGS, EXT_MAP };
