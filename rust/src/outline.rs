//! Structural outline of a source file: "LINE:signature" rows.
//! Faithful port of hooks/lib/outline.js. The JS and Rust hooks must produce
//! byte-identical stdout, so every observable detail here mirrors the JS:
//! ECMAScript regex semantics (via `regress`), JS whitespace definition,
//! `path.extname` semantics, and UTF-16 length/slice semantics.

use regress::Regex;
use std::sync::OnceLock;

pub struct Outline {
    pub lines: usize,
    pub rows: Vec<String>,
}

/// Declaration patterns per language, verbatim from LANGS in outline.js.
const P_JS: &[&str] = &[
    r"^\s*(export\s+)?(default\s+)?(async\s+)?function\s*\*?\s*[\w$]+",
    r"^\s*(export\s+)?(abstract\s+)?class\s+[\w$]+",
    r"^\s*(export\s+)?(const|let|var)\s+[\w$]+\s*=\s*(async\s*)?(\([^)]*\)|[\w$]+)\s*=>",
    r"^\s*(export\s+)?(interface|type|enum)\s+[\w$]+",
    "^\\s*(describe|it|test|suite|context|bench)(\\.\\w+)?\\s*(\\([^)]*\\)\\s*)?\\(\\s*[`'\"]",
];
const P_PY: &[&str] = &[r"^\s*(async\s+)?def\s+\w+", r"^\s*class\s+\w+", r"^\s*@\w[\w.]*"];
const P_GO: &[&str] = &[r"^\s*func\s+", r"^\s*type\s+\w+", r"^\s*(var|const)\s*\("];
const P_RS: &[&str] = &[
    r"^\s*(pub\s+)?(async\s+)?fn\s+\w+",
    r"^\s*(pub\s+)?(struct|enum|trait|impl|mod)\s",
    r"^\s*(pub\s+)?type\s+\w+",
];
const P_JAVA: &[&str] = &[
    r"^\s*(public|private|protected|static|final|abstract|\s)*(class|interface|enum|record)\s+\w+",
    r"^\s*(public|private|protected|static|final|abstract|synchronized|\s)+[\w<>\[\],.\s]+\s+\w+\s*\([^;]*\)\s*\{?\s*$",
];
const P_RB: &[&str] = &[r"^\s*(def|class|module)\s+"];
const P_PHP: &[&str] = &[
    r"^\s*(abstract\s+|final\s+)?(class|interface|trait)\s+\w+",
    r"^\s*(public|private|protected|static|\s)*function\s+\w+",
];
const P_SH: &[&str] = &[r"^\s*(function\s+)?[\w-]+\s*\(\)\s*\{"];
const P_SQL: &[&str] = &[r"^\s*(CREATE|ALTER|DROP)\s+", r"^\s*(WITH|SELECT|INSERT|UPDATE|DELETE)\s"];
const P_SWIFT: &[&str] = &[
    r"^\s*(public\s+|private\s+|internal\s+|open\s+|fileprivate\s+)?(final\s+)?(func|class|struct|enum|protocol|extension|var|let)\s+\w+",
];
const P_C: &[&str] = &[
    r"^\s*[\w*\s]+\s+\**\w+\s*\([^;]*\)\s*\{?\s*$",
    r"^\s*(typedef|struct|enum|union)\s+\w*",
    r"^\s*#(define|include)\s",
];

static C_JS: OnceLock<Vec<Regex>> = OnceLock::new();
static C_PY: OnceLock<Vec<Regex>> = OnceLock::new();
static C_GO: OnceLock<Vec<Regex>> = OnceLock::new();
static C_RS: OnceLock<Vec<Regex>> = OnceLock::new();
static C_JAVA: OnceLock<Vec<Regex>> = OnceLock::new();
static C_RB: OnceLock<Vec<Regex>> = OnceLock::new();
static C_PHP: OnceLock<Vec<Regex>> = OnceLock::new();
static C_SH: OnceLock<Vec<Regex>> = OnceLock::new();
static C_SQL: OnceLock<Vec<Regex>> = OnceLock::new();
static C_SWIFT: OnceLock<Vec<Regex>> = OnceLock::new();
static C_C: OnceLock<Vec<Regex>> = OnceLock::new();

/// Only the bucket for the language at hand is ever compiled; the hook fires on
/// every Read and eagerly building ~30 patterns would dominate its time budget.
fn patterns(lang: &str) -> Option<&'static [Regex]> {
    let (cell, src, flags): (&'static OnceLock<Vec<Regex>>, &'static [&'static str], &str) =
        match lang {
            "js" => (&C_JS, P_JS, ""),
            "py" => (&C_PY, P_PY, ""),
            "go" => (&C_GO, P_GO, ""),
            "rs" => (&C_RS, P_RS, ""),
            "java" => (&C_JAVA, P_JAVA, ""),
            "rb" => (&C_RB, P_RB, ""),
            "php" => (&C_PHP, P_PHP, ""),
            "sh" => (&C_SH, P_SH, ""),
            "sql" => (&C_SQL, P_SQL, "i"),
            "swift" => (&C_SWIFT, P_SWIFT, ""),
            "c" => (&C_C, P_C, ""),
            _ => return None,
        };
    let compiled = cell.get_or_init(|| {
        src.iter()
            .map(|p| Regex::with_flags(p, flags).expect("outline pattern must compile"))
            .collect()
    });
    Some(compiled.as_slice())
}

/// EXT_MAP from outline.js, verbatim. Keys are already lowercase.
const EXT_MAP: &[(&str, &str)] = &[
    (".js", "js"),
    (".jsx", "js"),
    (".mjs", "js"),
    (".cjs", "js"),
    (".ts", "js"),
    (".tsx", "js"),
    (".py", "py"),
    (".go", "go"),
    (".rs", "rs"),
    (".java", "java"),
    (".kt", "java"),
    (".scala", "java"),
    (".rb", "rb"),
    (".php", "php"),
    (".sh", "sh"),
    (".bash", "sh"),
    (".zsh", "sh"),
    (".sql", "sql"),
    (".swift", "swift"),
    (".c", "c"),
    (".h", "c"),
    (".cc", "c"),
    (".cpp", "c"),
    (".hpp", "c"),
    (".cs", "java"),
];

/// Port of Node's posix `path.extname`. Notably returns "" for a dotfile
/// (".bashrc"), for a name with no dot, and for a trailing component of "..".
/// A naive rsplit('.') would get all three wrong.
fn extname(p: &str) -> &str {
    let b = p.as_bytes();
    let mut start_dot: isize = -1;
    let mut start_part: isize = 0;
    let mut end: isize = -1;
    let mut matched_slash = true;
    // 0 = nothing but dots seen so far, 1 = saw a second dot, -1 = saw a non-dot
    let mut pre_dot_state: i32 = 0;

    let mut i = b.len() as isize - 1;
    while i >= 0 {
        let c = b[i as usize];
        if c == b'/' {
            if !matched_slash {
                start_part = i + 1;
                break;
            }
            i -= 1;
            continue;
        }
        if end == -1 {
            matched_slash = false;
            end = i + 1;
        }
        if c == b'.' {
            if start_dot == -1 {
                start_dot = i;
            } else if pre_dot_state != 1 {
                pre_dot_state = 1;
            }
        } else if start_dot != -1 {
            pre_dot_state = -1;
        }
        i -= 1;
    }

    if start_dot == -1
        || end == -1
        || pre_dot_state == 0
        || (pre_dot_state == 1 && start_dot == end - 1 && start_dot == start_part + 1)
    {
        return "";
    }
    &p[start_dot as usize..end as usize]
}

pub fn lang_for(file_path: &str) -> Option<&'static str> {
    let ext = extname(file_path).to_lowercase();
    if ext.is_empty() {
        return None;
    }
    EXT_MAP
        .iter()
        .find(|(k, _)| *k == ext)
        .map(|(_, lang)| *lang)
}

pub fn is_source(file_path: &str) -> bool {
    lang_for(file_path).is_some()
}

/// ECMAScript WhiteSpace + LineTerminator, which is what JS `String.trim` and
/// the regex `\s` class both use. Differs from Rust's `char::is_whitespace`:
/// U+0085 is whitespace to Rust but not to JS, U+FEFF is the reverse.
fn is_js_ws(c: char) -> bool {
    matches!(c,
        '\u{0009}' | '\u{000A}' | '\u{000B}' | '\u{000C}' | '\u{000D}'
        | '\u{0020}' | '\u{00A0}' | '\u{1680}'
        | '\u{2000}'..='\u{200A}'
        | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
        | '\u{FEFF}')
}

/// JS `String.length` counts UTF-16 code units, not chars.
fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `utf16_len(s) > limit`, without walking `s` when the byte length already
/// settles it. A UTF-8 sequence is never shorter than its UTF-16 form, so
/// `utf16_len(s) <= s.len()` always holds.
fn utf16_len_gt(s: &str, limit: usize) -> bool {
    s.len() > limit && utf16_len(s) > limit
}

/// JS `.slice(0, max)` counts UTF-16 code units.
///
/// KNOWN DIVERGENCE, the only one found against the JS: if the cut lands inside
/// a surrogate pair, JS keeps the lone high surrogate and we drop the whole
/// character. A Rust `String` cannot hold a lone surrogate, so there is no
/// faithful option - JSON.stringify would emit the escape `\ud83d`, a raw stdout
/// write would emit U+FFFD, and neither is reachable from a `String`. Requires an
/// astral character starting at exactly UTF-16 offset 159 of a declaration line
/// in a file whose outline is dense enough to trigger a denial.
fn slice_utf16(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s; // utf16_len(s) <= s.len(), so nothing can be cut
    }
    let mut units = 0usize;
    for (i, c) in s.char_indices() {
        let n = c.len_utf16();
        if units + n > max {
            return &s[..i];
        }
        units += n;
    }
    s
}

/// Returns { lines, rows } where rows are "LINE:signature" strings.
/// Line numbers are 1-indexed so they drop straight into Read(offset, limit).
/// JS default for `max_rows` is 200.
pub fn outline(text: &str, file_path: &str, max_rows: usize) -> Outline {
    // Plain split on '\n', matching JS. Not `str::lines()`, which folds a
    // trailing newline and strips '\r'. `split('\n').count()` is always the
    // number of newlines plus one.
    let lines = bytecount_nl(text) + 1;
    let pats = match lang_for(file_path).and_then(patterns) {
        Some(p) => p,
        None => {
            return Outline {
                lines,
                rows: Vec::new(),
            }
        }
    };

    let mut rows: Vec<String> = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        // JS checks `rows.length < maxRows` in the loop condition, so it stops
        // before ever adding row max_rows + 1.
        if rows.len() >= max_rows {
            break;
        }
        if line.chars().all(is_js_ws) {
            continue;
        }
        if utf16_len_gt(line, 400) {
            continue;
        }
        if pats.iter().any(|p| p.find(line).is_some()) {
            let trimmed = line.trim_end_matches(is_js_ws);
            rows.push(format!("{}:{}", i + 1, slice_utf16(trimmed, 160)));
        }
    }
    Outline { lines, rows }
}

fn bytecount_nl(text: &str) -> usize {
    text.as_bytes().iter().filter(|b| **b == b'\n').count()
}

/// Is this outline dense enough to represent the file?
///
/// A handful of declarations across a long file means the outline misses the
/// file's real structure - the model would just re-read the whole thing, and the
/// denial cost a round trip for nothing. Better to let the read through.
pub fn covers(lines: usize, row_count: usize) -> bool {
    row_count >= std::cmp::max(4, lines / 80)
}

/// omp's recovery footer: teaches the cheap re-read instead of a whole-file retry.
pub fn render(file_path: &str, lines: usize, rows: &[String]) -> String {
    let head = format!(
        "[{}] {} lines - structural outline only, bodies elided.",
        file_path, lines
    );
    let body = rows.join("\n");
    let foot = "\n\n[Re-read only what you need: Read(file_path, offset, limit) using the line \
                numbers above. If you genuinely need the whole file, just call Read again on \
                this path - it will go through.]";
    format!("{}\n\n{}{}", head, body, foot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        for lang in [
            "js", "py", "go", "rs", "java", "rb", "php", "sh", "sql", "swift", "c",
        ] {
            assert!(patterns(lang).is_some(), "{lang}");
        }
        assert!(patterns("nope").is_none());
    }

    #[test]
    fn lang_for_each_extension() {
        let cases: &[(&str, Option<&str>)] = &[
            ("a.js", Some("js")),
            ("a.jsx", Some("js")),
            ("a.mjs", Some("js")),
            ("a.cjs", Some("js")),
            ("a.ts", Some("js")),
            ("a.tsx", Some("js")),
            ("a.py", Some("py")),
            ("a.go", Some("go")),
            ("a.rs", Some("rs")),
            ("a.java", Some("java")),
            ("a.kt", Some("java")),
            ("a.scala", Some("java")),
            ("a.cs", Some("java")),
            ("a.rb", Some("rb")),
            ("a.php", Some("php")),
            ("a.sh", Some("sh")),
            ("a.bash", Some("sh")),
            ("a.zsh", Some("sh")),
            ("a.sql", Some("sql")),
            ("a.swift", Some("swift")),
            ("a.c", Some("c")),
            ("a.h", Some("c")),
            ("a.cc", Some("c")),
            ("a.cpp", Some("c")),
            ("a.hpp", Some("c")),
        ];
        assert_eq!(cases.len(), 25);
        for (p, want) in cases {
            assert_eq!(lang_for(p), *want, "{p}");
            assert_eq!(lang_for(&p.to_uppercase()), *want, "{p} uppercased");
        }
    }

    #[test]
    fn lang_for_non_source() {
        for p in [
            "README.md",
            "Makefile",
            "/etc/hosts",
            "",
            "a.txt",
            "weird.jsx.bak",
        ] {
            assert_eq!(lang_for(p), None, "{p}");
            assert!(!is_source(p), "{p}");
        }
    }

    #[test]
    fn extname_matches_node_semantics() {
        // Dotfiles and extensionless names have no extension in Node.
        assert_eq!(extname(".bashrc"), "");
        assert_eq!(extname("/home/me/.bashrc"), "");
        assert_eq!(extname("Makefile"), "");
        assert_eq!(extname(".."), "");
        assert_eq!(extname("."), "");
        assert_eq!(extname("/a/b/"), "");
        assert_eq!(extname("index."), ".");
        assert_eq!(extname("a.b.c.py"), ".py");
        assert_eq!(extname("/a.dir/file"), "");
        assert_eq!(extname("/a.dir/file.rs"), ".rs");
        assert_eq!(extname("a.tar.gz"), ".gz");
        // Dotfiles must not resolve through a naive rsplit.
        assert_eq!(lang_for(".rs"), None);
        assert_eq!(lang_for("/x/.py"), None);
    }

    #[test]
    fn line_count_uses_split_not_lines() {
        assert_eq!(outline("", "a.txt", 200).lines, 1);
        assert_eq!(outline("a", "a.txt", 200).lines, 1);
        assert_eq!(outline("a\n", "a.txt", 200).lines, 2);
        assert_eq!(outline("a\nb\n", "a.txt", 200).lines, 3);
        assert_eq!(outline("\n\n", "a.txt", 200).lines, 3);
        // CRLF: split('\n') keeps the '\r' inside the segment, so the count is
        // the same as for LF-only text.
        assert_eq!(outline("a\r\nb\r\n", "a.txt", 200).lines, 3);
    }

    #[test]
    fn non_source_yields_no_rows() {
        let o = outline("def foo():\n    pass\n", "notes.txt", 200);
        assert_eq!(o.lines, 3);
        assert!(o.rows.is_empty());
    }

    #[test]
    fn body_lines_are_excluded() {
        let src = "\
import os


def alpha(x):
    total = x + 1
    if total > 3:
        return total
    return 0


class Beta:
    @property
    def gamma(self):
        return 1
";
        let o = outline(src, "m.py", 200);
        assert_eq!(
            o.rows,
            vec![
                "4:def alpha(x):".to_string(),
                "11:class Beta:".to_string(),
                "12:    @property".to_string(),
                "13:    def gamma(self):".to_string(),
            ]
        );
    }

    #[test]
    fn rust_and_js_and_sql_families_match() {
        let o = outline("pub fn a() {}\n  impl Foo {\nlet x = 1;\n", "m.rs", 200);
        assert_eq!(o.rows, vec!["1:pub fn a() {}", "2:  impl Foo {"]);

        let o = outline(
            "export const f = (a) => a;\nexport class K {}\nconst z = 3;\n",
            "m.ts",
            200,
        );
        assert_eq!(o.rows, vec!["1:export const f = (a) => a;", "2:export class K {}"]);

        // The sql patterns carry the /i flag.
        let o = outline("select 1\n  create table t (a int);\n-- x\n", "q.sql", 200);
        assert_eq!(o.rows, vec!["1:select 1", "2:  create table t (a int);"]);
    }

    #[test]
    fn trailing_whitespace_is_stripped_and_blank_lines_skipped() {
        let o = outline("def a():   \t\n   \n\t\n", "m.py", 200);
        assert_eq!(o.rows, vec!["1:def a():"]);
        assert_eq!(o.lines, 4);
        // A '\r' from CRLF input is JS whitespace and gets stripped too.
        let o = outline("def a():\r\n", "m.py", 200);
        assert_eq!(o.rows, vec!["1:def a():"]);
    }

    #[test]
    fn truncates_to_160_units() {
        let name = "a".repeat(300);
        let src = format!("def {name}():\n");
        let o = outline(&src, "m.py", 200);
        assert_eq!(o.rows.len(), 1);
        let sig = &o.rows[0]["2:".len()..];
        assert_eq!(sig.chars().count(), 160);
        assert_eq!(sig, &format!("def {name}")[..160]);
    }

    /// Astral characters count as 2 UTF-16 units, like JS `String.length`.
    #[test]
    fn astral_characters_count_as_two_units() {
        // "def " + 154 'a' = 158 units, emoji lands on 158..160: kept whole.
        let src = format!("def {}\u{1F600}{}():\n", "a".repeat(154), "b".repeat(50));
        let sig = &outline(&src, "m.py", 200).rows[0]["1:".len()..];
        assert_eq!(utf16_len(sig), 160);
        assert!(sig.ends_with('\u{1F600}'));

        // 400-unit skip threshold also counts the emoji as 2.
        let at_400 = format!("def {}\u{1F600}", "a".repeat(394));
        assert_eq!(utf16_len(&at_400), 400);
        assert_eq!(outline(&at_400, "m.py", 200).rows.len(), 1);
        let at_401 = format!("def {}\u{1F600}", "a".repeat(395));
        assert_eq!(outline(&at_401, "m.py", 200).rows.len(), 0);
    }

    /// See `slice_utf16`: the one place this port cannot match the JS byte for
    /// byte. JS keeps a lone high surrogate at the cut; we drop the character.
    #[test]
    fn surrogate_straddle_drops_the_character() {
        // "def " + 155 'a' = 159 units, emoji straddles 159..161.
        let src = format!("def {}\u{1F600}bbb():\n", "a".repeat(155));
        let sig = &outline(&src, "m.py", 200).rows[0]["1:".len()..];
        assert_eq!(utf16_len(sig), 159);
        assert!(sig.ends_with('a'));
    }

    #[test]
    fn lines_over_400_units_are_skipped() {
        let ok = format!("def {}():", "a".repeat(393)); // exactly 400
        assert_eq!(utf16_len(&ok), 400);
        let too_long = format!("def {}():", "a".repeat(394)); // 401
        assert_eq!(utf16_len(&too_long), 401);
        assert_eq!(outline(&ok, "m.py", 200).rows.len(), 1);
        assert_eq!(outline(&too_long, "m.py", 200).rows.len(), 0);
    }

    #[test]
    fn max_rows_caps_output() {
        let src = (0..500)
            .map(|i| format!("def f{i}():\n    pass\n"))
            .collect::<String>();
        assert_eq!(outline(&src, "m.py", 200).rows.len(), 200);
        assert_eq!(outline(&src, "m.py", 3).rows.len(), 3);
        assert_eq!(outline(&src, "m.py", 0).rows.len(), 0);
        let capped = outline(&src, "m.py", 200);
        assert_eq!(capped.rows[199], "399:def f199():");
    }

    #[test]
    fn covers_boundaries() {
        assert!(!covers(800, 5));
        assert!(covers(800, 10));
        assert!(covers(420, 5));
        assert!(!covers(420, 4));
        assert!(covers(100, 4));
        assert!(!covers(0, 3));
        assert!(covers(0, 4));
        // Floor division, matching Math.floor: 799/80 -> 9, not 10.
        assert!(covers(799, 9));
        assert!(!covers(800, 9));
    }

    #[test]
    fn render_is_byte_exact() {
        let rows = vec!["1:def a():".to_string(), "9:class B:".to_string()];
        let got = render("/x/m.py", 42, &rows);
        assert_eq!(
            got,
            "[/x/m.py] 42 lines - structural outline only, bodies elided.\n\n\
             1:def a():\n9:class B:\n\n\
             [Re-read only what you need: Read(file_path, offset, limit) using the line numbers above. If you genuinely need the whole file, just call Read again on this path - it will go through.]"
        );
    }

    #[test]
    fn render_with_no_rows() {
        let got = render("m.py", 1, &[]);
        assert!(got.starts_with("[m.py] 1 lines - structural outline only, bodies elided.\n\n\n\n["));
    }
}
