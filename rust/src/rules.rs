//! Rule loading, scoping and evaluation. A semantics-for-semantics port of
//! hooks/lib/rules.js: the JS and Rust hooks are fed identical payloads by a
//! differential test harness and must produce byte-identical stdout, so the
//! quirks of the JS are reproduced deliberately rather than tidied up.

use crate::state::State;
use regress::Regex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ScopeEntry {
    pub tool: String,
    pub glob: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub name: String,
    pub description: String,
    /// Raw pattern, taken verbatim from the frontmatter. Compiled lazily in
    /// evaluate() so the cheap scope/arming checks can reject first.
    pub condition: String,
    pub flags: String,
    pub scope: Vec<ScopeEntry>,
    pub repeat: String,
    pub interrupt: bool,
    pub body: String,
}

pub struct Frontmatter {
    pub meta: BTreeMap<String, String>,
    pub body: String,
}

// --- JavaScript primitives -------------------------------------------------

/// String.prototype.trim: WhiteSpace + LineTerminator per ECMA-262. Rust's
/// char::is_whitespace is the Unicode White_Space property, which differs
/// (it includes U+0085, and excludes U+FEFF).
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}'
            | '\u{b}'
            | '\u{c}'
            | '\u{20}'
            | '\u{a0}'
            | '\u{feff}'
            | '\u{a}'
            | '\u{d}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// JS truthiness of a JSON value: null, false, 0, "" are falsy; [] and {} are not.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(true),
        Some(Value::String(s)) => !s.is_empty(),
        _ => true,
    }
}

fn js_num_to_string(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if f == 0.0 {
        return "0".into();
    }
    if f == f.trunc() && f.abs() < 1e21 {
        return format!("{:.0}", f);
    }
    format!("{}", f)
}

/// String(value) for a JSON value.
fn js_to_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => if *b { "true".into() } else { "false".into() },
        Value::Number(n) => n.as_f64().map(js_num_to_string).unwrap_or_else(|| n.to_string()),
        Value::String(s) => s.clone(),
        // Array.prototype.toString joins with ',' and renders null/undefined as ''.
        Value::Array(a) => a
            .iter()
            .map(|x| match x {
                Value::Null => String::new(),
                other => js_to_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// `String(obj.key ?? '')`: null/undefined collapse to the empty string.
fn str_prop(input: &Value, key: &str) -> String {
    match input.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(v) => js_to_string(v),
    }
}

/// Node's path.posix.basename, which differs from Rust's Path::file_name for
/// "/", "." and "..".
fn basename(p: &str) -> &str {
    let b = p.as_bytes();
    let mut start = 0usize;
    let mut end: Option<usize> = None;
    let mut matched_slash = true;
    let mut i = b.len();
    while i > 0 {
        i -= 1;
        if b[i] == b'/' {
            if !matched_slash {
                start = i + 1;
                break;
            }
        } else if end.is_none() {
            matched_slash = false;
            end = Some(i + 1);
        }
    }
    match end {
        None => "",
        Some(e) => &p[start..e],
    }
}

// --- Frontmatter -----------------------------------------------------------

pub fn parse_frontmatter(text: &str) -> Option<Frontmatter> {
    if !text.starts_with("---") {
        return None;
    }
    let end = text.get(3..)?.find("\n---").map(|i| i + 3)?;
    // JS: text.slice(indexOf('\n', 3) + 1, end). The '\n' of the closing "\n---"
    // guarantees a hit, and slice() clamps a start past end to an empty string.
    let raw_start = text[3..].find('\n').map(|i| i + 3 + 1).unwrap_or(0);
    let raw = if raw_start >= end { "" } else { &text[raw_start..end] };
    // JS: text.slice(indexOf('\n', end + 1) + 1). With no newline after the
    // closing "---" that is slice(0) — the whole document becomes the body.
    let body = match text[end + 1..].find('\n') {
        Some(i) => &text[end + 1 + i + 1..],
        None => text,
    };

    let kv = Regex::new(r"^([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*)$").ok()?;
    let mut meta = BTreeMap::new();
    for line in raw.split('\n') {
        let m = match kv.find(line) {
            Some(m) => m,
            None => continue,
        };
        let key = line[m.group(1)?].to_string();
        let mut v = js_trim(&line[m.group(2)?]);
        let bytes = v.as_bytes();
        if bytes.len() > 1
            && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
                || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
        {
            v = &v[1..v.len() - 1];
        }
        meta.insert(key, v.to_string());
    }
    Some(Frontmatter {
        meta,
        body: js_trim(body).to_string(),
    })
}

// --- Scope -----------------------------------------------------------------

/// "tool:Bash, tool:Write(*.sh)" -> [Bash/None, Write/Some("*.sh")]
pub fn parse_scope(scope: &str) -> Vec<ScopeEntry> {
    if scope.is_empty() {
        return Vec::new();
    }
    let re = match Regex::new(r"^tool:([A-Za-z_][A-Za-z0-9_]*)\s*(?:\(([^)]*)\))?$") {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for part in scope.split(',') {
        let part = js_trim(part);
        if let Some(m) = re.find(part) {
            let tool = match m.group(1) {
                Some(r) => part[r].to_string(),
                None => continue,
            };
            // JS `m[2] ? m[2].trim() : null`: an absent *or empty* group is null.
            let glob = match m.group(2) {
                Some(r) if !r.is_empty() => Some(js_trim(&part[r]).to_string()),
                _ => None,
            };
            out.push(ScopeEntry { tool, glob });
        }
    }
    out
}

fn glob_to_regexp(glob: &str) -> Option<Regex> {
    let chars: Vec<char> = glob.chars().collect();
    let mut re = String::from("^");
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '*' {
            if chars.get(i + 1) == Some(&'*') {
                re.push_str(".*");
                i += 1;
            } else {
                re.push_str("[^/]*");
            }
        } else if c == '?' {
            re.push_str("[^/]");
        } else {
            if matches!(c, '.' | '+' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '\\') {
                re.push('\\');
            }
            re.push(c);
        }
        i += 1;
    }
    re.push('$');
    Regex::new(&re).ok()
}

pub fn glob_matches(glob: &str, file_path: Option<&str>) -> bool {
    if glob.is_empty() {
        return true;
    }
    let p = match file_path {
        Some(p) if !p.is_empty() => p,
        _ => return false,
    };
    let re = match glob_to_regexp(glob) {
        Some(r) => r,
        None => return false,
    };
    re.find(p).is_some() || re.find(basename(p)).is_some()
}

// --- Tool payload extraction ----------------------------------------------

/// omp calls this the matcherDigest: ONLY the new content the call introduces.
/// Matching pre-existing file content over-fires on every unrelated edit to a
/// file that happens to contain the pattern somewhere.
pub fn matcher_digest(tool_name: &str, input: &Value) -> String {
    if !(input.is_object() || input.is_array()) {
        return String::new();
    }
    match tool_name {
        "Edit" => str_prop(input, "new_string"),
        "Write" => str_prop(input, "content"),
        "MultiEdit" => match input.get("edits").and_then(Value::as_array) {
            Some(edits) => edits
                .iter()
                .map(|e| {
                    // JS uses truthiness here, not ??: an empty new_string
                    // contributes an empty segment.
                    if truthy(Some(e)) && truthy(e.get("new_string")) {
                        js_to_string(e.get("new_string").unwrap())
                    } else {
                        String::new()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            None => String::new(),
        },
        "NotebookEdit" => str_prop(input, "new_source"),
        "Bash" => str_prop(input, "command"),
        _ => String::new(),
    }
}

pub fn target_path(tool_name: &str, input: &Value) -> Option<String> {
    if !(input.is_object() || input.is_array()) {
        return None;
    }
    let key = if tool_name == "NotebookEdit" {
        "notebook_path"
    } else {
        "file_path"
    };
    let v = input.get(key);
    if truthy(v) {
        Some(js_to_string(v.unwrap()))
    } else {
        None
    }
}

// --- Loading ---------------------------------------------------------------

/// Later entries shadow earlier ones by filename, so a project rule overrides
/// a user rule.
fn rule_dirs(cwd: Option<&str>) -> Vec<PathBuf> {
    let project = match cwd {
        Some(c) if !c.is_empty() => PathBuf::from(c),
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    vec![
        crate::state::config_dir().join("rules"),
        project.join(".claude").join("rules"),
    ]
}

fn read_dir_sorted(dir: &Path) -> Option<Vec<String>> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    // Sort explicitly: readdir order is filesystem- and runtime-dependent (node
    // and bun disagree), and evaluate() fires the FIRST matching rule. Without
    // this, which rule wins a two-rule match would vary by interpreter. JS
    // Array.sort compares UTF-16 code units, not UTF-8 bytes.
    names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    Some(names)
}

pub fn load_rules(cwd: Option<&str>) -> Vec<Rule> {
    // A JS Map keeps the insertion position of a key that is overwritten, so a
    // shadowed rule keeps the *user* directory's slot in the output order.
    let mut order: Vec<String> = Vec::new();
    let mut by_name: BTreeMap<String, Rule> = BTreeMap::new();

    for dir in rule_dirs(cwd) {
        let entries = match read_dir_sorted(&dir) {
            Some(e) => e,
            None => continue,
        };
        for f in entries {
            if !f.ends_with(".md") {
                continue;
            }
            let text = match std::fs::read_to_string(dir.join(&f)) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let parsed = match parse_frontmatter(&text) {
                Some(p) => p,
                None => continue,
            };
            let condition = match parsed.meta.get("condition") {
                Some(c) if !c.is_empty() => c.clone(),
                _ => continue,
            };
            let flags = parsed.meta.get("flags").cloned().unwrap_or_default();
            // The condition is taken verbatim - no YAML escape processing - so
            // rules are written with single backslashes (\w, \s). Compiled here
            // only to reject a rule the JS `new RegExp` would have thrown on.
            if Regex::with_flags(&condition, flags.as_str()).is_err() {
                continue;
            }
            let rule = Rule {
                name: f.strip_suffix(".md").unwrap_or(&f).to_string(),
                description: nonempty(parsed.meta.get("description")).unwrap_or_default(),
                condition,
                flags,
                scope: parse_scope(parsed.meta.get("scope").map(String::as_str).unwrap_or("")),
                repeat: nonempty(parsed.meta.get("repeat")).unwrap_or_else(|| "once".into()),
                interrupt: parsed.meta.get("interrupt").map(String::as_str) != Some("false"),
                body: parsed.body,
            };
            if !by_name.contains_key(&f) {
                order.push(f.clone());
            }
            by_name.insert(f, rule);
        }
    }

    order
        .iter()
        .filter_map(|k| by_name.remove(k))
        .collect()
}

/// `meta.x || fallback`: an empty string is falsy in JS.
fn nonempty(v: Option<&String>) -> Option<String> {
    v.filter(|s| !s.is_empty()).cloned()
}

// --- Evaluation ------------------------------------------------------------

pub fn scope_allows(rule: &Rule, tool_name: &str, file_path: Option<&str>) -> bool {
    if rule.scope.is_empty() {
        return true;
    }
    rule.scope.iter().any(|s| {
        s.tool == tool_name && glob_matches(s.glob.as_deref().unwrap_or(""), file_path)
    })
}

pub fn is_armed(rule: &Rule, state: &State) -> bool {
    let fired = match state.fired.get(&rule.name) {
        Some(f) => *f,
        None => return true,
    };
    let re = match Regex::new(r"^after-gap\s+(\d+)$") {
        Ok(r) => r,
        Err(_) => return false,
    };
    let m = match re.find(&rule.repeat) {
        Some(m) => m,
        None => return false, // "once"
    };
    let n: f64 = match m.group(1) {
        Some(r) => rule.repeat[r].parse().unwrap_or(f64::INFINITY),
        None => return false,
    };
    // The JS subtracts in f64 and can go negative; u64 arithmetic would wrap.
    (state.calls as f64) - (fired as f64) >= n
}

pub fn evaluate<'a>(
    rules: &'a [Rule],
    tool_name: &str,
    input: &Value,
    state: &State,
) -> Option<(&'a Rule, Option<String>)> {
    let digest = matcher_digest(tool_name, input);
    if digest.is_empty() {
        return None;
    }
    let file_path = target_path(tool_name, input);
    for rule in rules {
        if !scope_allows(rule, tool_name, file_path.as_deref()) {
            continue;
        }
        if !is_armed(rule, state) {
            continue;
        }
        // Compiled last: the cheap checks reject most calls before paying for it.
        let re = match Regex::with_flags(&rule.condition, rule.flags.as_str()) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if re.find(&digest).is_none() {
            continue;
        }
        return Some((rule, file_path));
    }
    None
}

pub fn render_interrupt(rule: &Rule, file_path: Option<&str>) -> String {
    let path_attr = match file_path {
        Some(p) => format!(" path=\"{}\"", p),
        None => String::new(),
    };
    format!(
        "<system-interrupt reason=\"rule_violation\" rule=\"{}\"{}>\n{}\n</system-interrupt>",
        rule.name, path_attr, rule.body
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(name: &str, repeat: &str) -> Rule {
        Rule {
            name: name.into(),
            description: String::new(),
            condition: "x".into(),
            flags: String::new(),
            scope: Vec::new(),
            repeat: repeat.into(),
            interrupt: true,
            body: "body".into(),
        }
    }

    #[test]
    fn frontmatter_basic() {
        let fm = parse_frontmatter("---\ncondition: \\bTODO\\b\nflags: i\n---\nBody text\n").unwrap();
        assert_eq!(fm.meta.get("condition").unwrap(), "\\bTODO\\b");
        assert_eq!(fm.meta.get("flags").unwrap(), "i");
        assert_eq!(fm.body, "Body text");
    }

    #[test]
    fn frontmatter_strips_one_quote_layer_only() {
        let fm = parse_frontmatter("---\na: \"x\"\nb: '\"y\"'\nc: \"\nd: \n---\nz").unwrap();
        assert_eq!(fm.meta.get("a").unwrap(), "x");
        assert_eq!(fm.meta.get("b").unwrap(), "\"y\"");
        assert_eq!(fm.meta.get("c").unwrap(), "\"");
        assert_eq!(fm.meta.get("d").unwrap(), "");
    }

    #[test]
    fn frontmatter_rejects_and_quirks() {
        assert!(parse_frontmatter("no marker").is_none());
        assert!(parse_frontmatter("---\ncondition: x\n").is_none());
        // Empty frontmatter block.
        let fm = parse_frontmatter("---\n---\nbody").unwrap();
        assert!(fm.meta.is_empty());
        assert_eq!(fm.body, "body");
        // No newline after the closing --- : JS slice(0) makes the whole doc the body.
        let fm = parse_frontmatter("---\ncondition: x\n---").unwrap();
        assert_eq!(fm.meta.get("condition").unwrap(), "x");
        assert_eq!(fm.body, "---\ncondition: x\n---");
        // Non key: value lines are ignored, not fatal.
        let fm = parse_frontmatter("---\njust text\n9bad: v\nok_key: v\n---\nb").unwrap();
        assert_eq!(fm.meta.len(), 1);
        assert_eq!(fm.meta.get("ok_key").unwrap(), "v");
    }

    #[test]
    fn scope_parsing() {
        let s = parse_scope("tool:Bash, tool:Write(*.sh)");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].tool, "Bash");
        assert!(s[0].glob.is_none());
        assert_eq!(s[1].tool, "Write");
        assert_eq!(s[1].glob.as_deref(), Some("*.sh"));

        assert!(parse_scope("").is_empty());
        assert!(parse_scope("Bash").is_empty());
        assert!(parse_scope("tool:9x").is_empty());
        // Junk parts are dropped, valid ones survive.
        let s = parse_scope("garbage, tool:Edit ( src/*.rs )");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].glob.as_deref(), Some("src/*.rs"));
        // Empty parens -> null glob, matching the JS truthiness check.
        assert!(parse_scope("tool:Edit()")[0].glob.is_none());
    }

    #[test]
    fn globs() {
        assert!(glob_matches("", Some("/a/b.rs")));
        assert!(glob_matches("", None));
        assert!(!glob_matches("*.rs", None));
        // basename fallback
        assert!(glob_matches("*.rs", Some("/a/b/c.rs")));
        assert!(!glob_matches("*.rs", Some("/a/b/c.py")));
        // * does not cross a separator, ** does
        assert!(!glob_matches("src/*.rs", Some("src/a/b.rs")));
        assert!(glob_matches("src/**.rs", Some("src/a/b.rs")));
        assert!(glob_matches("src/*.rs", Some("src/a.rs")));
        // ? is a single non-separator char
        assert!(glob_matches("a?c.txt", Some("abc.txt")));
        assert!(!glob_matches("a?c.txt", Some("a/c.txt")));
        // regex metacharacters in the glob are literal
        assert!(glob_matches("a+b.txt", Some("a+b.txt")));
        assert!(!glob_matches("a+b.txt", Some("aab.txt")));
        assert!(glob_matches("(x).md", Some("(x).md")));
        // full-path match without a basename match
        assert!(glob_matches("/etc/**", Some("/etc/passwd")));
    }

    #[test]
    fn digests() {
        assert_eq!(matcher_digest("Edit", &json!({"new_string": "abc"})), "abc");
        assert_eq!(matcher_digest("Edit", &json!({"old_string": "abc"})), "");
        assert_eq!(matcher_digest("Write", &json!({"content": "hi"})), "hi");
        assert_eq!(matcher_digest("Bash", &json!({"command": "rm -rf /"})), "rm -rf /");
        assert_eq!(matcher_digest("NotebookEdit", &json!({"new_source": "src"})), "src");
        assert_eq!(matcher_digest("Read", &json!({"content": "hi"})), "");
        assert_eq!(matcher_digest("Edit", &json!("not an object")), "");
        assert_eq!(matcher_digest("Edit", &Value::Null), "");
        // MultiEdit joins with \n, missing/empty new_string contributes "".
        assert_eq!(
            matcher_digest(
                "MultiEdit",
                &json!({"edits": [{"new_string": "a"}, {"old_string": "z"}, {"new_string": "b"}]})
            ),
            "a\n\nb"
        );
        assert_eq!(matcher_digest("MultiEdit", &json!({"edits": "nope"})), "");
        assert_eq!(matcher_digest("MultiEdit", &json!({})), "");
    }

    #[test]
    fn paths() {
        assert_eq!(
            target_path("Edit", &json!({"file_path": "/a.rs"})).as_deref(),
            Some("/a.rs")
        );
        assert_eq!(
            target_path("NotebookEdit", &json!({"notebook_path": "/n.ipynb", "file_path": "/x"}))
                .as_deref(),
            Some("/n.ipynb")
        );
        assert_eq!(target_path("Edit", &json!({"file_path": ""})), None);
        assert_eq!(target_path("Bash", &json!({"command": "ls"})), None);
        assert_eq!(target_path("Edit", &Value::Null), None);
    }

    #[test]
    fn basename_matches_node() {
        assert_eq!(basename("/a/b/c.rs"), "c.rs");
        assert_eq!(basename("c.rs"), "c.rs");
        assert_eq!(basename("/a/b/"), "b");
        assert_eq!(basename("/"), "");
        assert_eq!(basename(""), "");
        assert_eq!(basename(".."), "..");
    }

    #[test]
    fn armed_when_never_fired() {
        let st = State::default();
        assert!(is_armed(&rule("r", "once"), &st));
        assert!(is_armed(&rule("r", "after-gap 5"), &st));
    }

    #[test]
    fn once_never_rearms() {
        let mut st = State::default();
        st.calls = 9999;
        st.fired.insert("r".into(), 1);
        assert!(!is_armed(&rule("r", "once"), &st));
        // An unparseable repeat behaves like "once".
        assert!(!is_armed(&rule("r", "after-gap"), &st));
        assert!(!is_armed(&rule("r", "after-gap x"), &st));
        assert!(!is_armed(&rule("r", "AFTER-GAP 1"), &st));
    }

    #[test]
    fn after_gap_boundary() {
        let mut st = State::default();
        st.fired.insert("r".into(), 10);
        let r = rule("r", "after-gap 3");
        st.calls = 12;
        assert!(!is_armed(&r, &st));
        st.calls = 13;
        assert!(is_armed(&r, &st));
        st.calls = 100;
        assert!(is_armed(&r, &st));
        // after-gap 0 is always armed.
        st.calls = 10;
        assert!(is_armed(&rule("r", "after-gap 0"), &st));
    }

    #[test]
    fn after_gap_survives_calls_below_fired_at() {
        // A truncated or shared state file can leave calls < fired_at; u64
        // subtraction would panic here, JS just goes negative.
        let mut st = State::default();
        st.calls = 2;
        st.fired.insert("r".into(), 500);
        assert!(!is_armed(&rule("r", "after-gap 3"), &st));
        assert!(!is_armed(&rule("r", "after-gap 0"), &st));
        st.calls = 0;
        st.fired.insert("r".into(), u64::MAX);
        assert!(!is_armed(&rule("r", "after-gap 1"), &st));
    }

    #[test]
    fn scope_allows_rules() {
        let mut r = rule("r", "once");
        assert!(scope_allows(&r, "Anything", None));
        r.scope = parse_scope("tool:Write(*.sh), tool:Bash");
        assert!(scope_allows(&r, "Bash", None));
        assert!(scope_allows(&r, "Write", Some("/x/y.sh")));
        assert!(!scope_allows(&r, "Write", Some("/x/y.rs")));
        assert!(!scope_allows(&r, "Write", None));
        assert!(!scope_allows(&r, "Edit", Some("/x/y.sh")));
    }

    #[test]
    fn evaluate_picks_first_match() {
        let mut a = rule("a", "once");
        a.condition = "nope".into();
        let mut b = rule("b", "once");
        b.condition = "TODO".into();
        let mut c = rule("c", "once");
        c.condition = "TO.O".into();
        let rules = vec![a, b, c];
        let st = State::default();
        let input = json!({"new_string": "// TODO fix", "file_path": "/a.rs"});
        let (hit, p) = evaluate(&rules, "Edit", &input, &st).unwrap();
        assert_eq!(hit.name, "b");
        assert_eq!(p.as_deref(), Some("/a.rs"));

        // Empty digest short-circuits.
        assert!(evaluate(&rules, "Edit", &json!({"new_string": ""}), &st).is_none());
        assert!(evaluate(&rules, "Read", &json!({"file_path": "/a.rs"}), &st).is_none());
        // Disarmed rules are skipped, so the next match wins.
        let mut st2 = State::default();
        st2.fired.insert("b".into(), 1);
        st2.calls = 1;
        assert_eq!(evaluate(&rules, "Edit", &input, &st2).unwrap().0.name, "c");
    }

    #[test]
    fn evaluate_honours_flags() {
        let mut r = rule("r", "once");
        r.condition = "todo".into();
        r.flags = "i".into();
        let rules = vec![r];
        let st = State::default();
        assert!(evaluate(&rules, "Bash", &json!({"command": "TODO"}), &st).is_some());
    }

    #[test]
    fn render_with_and_without_path() {
        let r = rule("no-secrets", "once");
        assert_eq!(
            render_interrupt(&r, Some("/etc/x.env")),
            "<system-interrupt reason=\"rule_violation\" rule=\"no-secrets\" path=\"/etc/x.env\">\nbody\n</system-interrupt>"
        );
        assert_eq!(
            render_interrupt(&r, None),
            "<system-interrupt reason=\"rule_violation\" rule=\"no-secrets\">\nbody\n</system-interrupt>"
        );
    }

    /// Point the user-level rules dir at a path that cannot exist, so the
    /// loader tests only ever see the project dir they create. Idempotent, so
    /// it is safe under the test harness's thread parallelism.
    fn isolate_user_rules() {
        std::env::set_var(
            "CLAUDE_CONFIG_DIR",
            std::env::temp_dir().join("omp-no-such-config-dir"),
        );
    }

    #[test]
    fn load_rules_is_sorted_and_filters() {
        isolate_user_rules();
        let dir = std::env::temp_dir().join(format!("omp-rules-{}", std::process::id()));
        let rules_dir = dir.join(".claude").join("rules");
        std::fs::create_dir_all(&rules_dir).unwrap();
        std::fs::write(rules_dir.join("b.md"), "---\ncondition: b\n---\nB").unwrap();
        std::fs::write(rules_dir.join("a.md"), "---\ncondition: a\nrepeat: after-gap 2\n---\nA")
            .unwrap();
        std::fs::write(rules_dir.join("c.md"), "---\ndescription: no condition\n---\nC").unwrap();
        std::fs::write(rules_dir.join("d.md"), "---\ncondition: (unclosed\n---\nD").unwrap();
        std::fs::write(rules_dir.join("e.txt"), "---\ncondition: e\n---\nE").unwrap();

        let loaded = load_rules(Some(dir.to_str().unwrap()));
        let names: Vec<&str> = loaded.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
        assert_eq!(loaded[0].repeat, "after-gap 2");
        assert_eq!(loaded[1].repeat, "once");
        assert!(loaded[0].interrupt);
        assert_eq!(loaded[0].body, "A");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_rules_missing_dir_is_silent() {
        isolate_user_rules();
        let missing = std::env::temp_dir().join("omp-rules-definitely-missing-xyz");
        assert!(load_rules(Some(missing.to_str().unwrap())).is_empty());
    }
}
