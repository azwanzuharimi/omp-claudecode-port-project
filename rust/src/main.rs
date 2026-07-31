//! Claude Code hooks, Rust build.
//!
//! One binary, three subcommands, mirroring the JS entry points:
//!   omp-hooks lazy-rules        PreToolUse  (Edit|Write|MultiEdit|NotebookEdit|Bash)
//!   omp-hooks lazy-rules-post   PostToolUse (same matcher)
//!   omp-hooks read-discipline   PreToolUse  (Read)
//!
//! Output must be byte-identical to the JS hooks; test/differential.js asserts it.

mod outline;
mod rules;
mod state;

use serde_json::Value;
use std::io::Read;

/// JSON is assembled by hand rather than through a serde map: JS emits object keys
/// in insertion order, while serde_json's default map is a BTreeMap and would emit
/// `additionalContext` before `hookEventName`. That alone would break byte-identity.
fn jstr(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

fn emit_deny(reason: &str) {
    print!(
        "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"deny\",\"permissionDecisionReason\":{}}}}}",
        jstr(reason)
    );
}

fn emit_additional_context(ctx: &str) {
    print!(
        "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PostToolUse\",\"additionalContext\":{}}}}}",
        jstr(ctx)
    );
}

fn read_stdin() -> String {
    let mut buf = Vec::new();
    if std::io::stdin().read_to_end(&mut buf).is_err() {
        return String::new();
    }
    // Lossy like JS readFileSync(0,'utf8'), which substitutes replacement chars
    // rather than throwing on invalid UTF-8.
    String::from_utf8_lossy(&buf).into_owned()
}

fn payload() -> Option<Value> {
    let raw = read_stdin();
    if raw.trim().is_empty() {
        return None;
    }
    serde_json::from_str(&raw).ok()
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

// ------------------------------------------------------------------ lazy-rules

fn lazy_rules() {
    let p = match payload() {
        Some(p) => p,
        None => return,
    };
    let tool_name = match str_field(&p, "tool_name") {
        Some(t) => t,
        None => return,
    };
    let input = match p.get("tool_input") {
        Some(i) => i,
        None => return,
    };
    if !input.is_object() {
        return;
    }
    let session_id = str_field(&p, "session_id").unwrap_or("");

    let loaded = rules::load_rules(str_field(&p, "cwd"));
    if loaded.is_empty() {
        return;
    }

    let mut st = state::load(session_id);
    st.calls += 1;

    let (rule, file_path) = match rules::evaluate(&loaded, tool_name, input, &st) {
        Some(hit) => hit,
        None => {
            state::save(session_id, &st);
            return;
        }
    };

    st.fired.insert(rule.name.clone(), st.calls);
    let rendered = rules::render_interrupt(rule, file_path.as_deref());

    if rule.interrupt {
        state::save(session_id, &st);
        emit_deny(&rendered);
    } else {
        // Soft mode: let the call run, hand the correction back through
        // PostToolUse so it costs no extra round trip.
        st.pending.push(rendered);
        state::save(session_id, &st);
    }
}

// ------------------------------------------------------------- lazy-rules-post

fn lazy_rules_post() {
    let p = match payload() {
        Some(p) => p,
        None => return,
    };
    let session_id = str_field(&p, "session_id").unwrap_or("");

    let mut st = state::load(session_id);
    if st.pending.is_empty() {
        return;
    }
    let joined = st.pending.join("\n\n");
    st.pending.clear();
    state::save(session_id, &st);

    emit_additional_context(&joined);
}

// ------------------------------------------------------------ read-discipline

const MAX_BYTES: u64 = 4 * 1024 * 1024;

fn line_threshold() -> usize {
    std::env::var("OMP_PORT_READ_THRESHOLD")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(400)
}

fn read_discipline() {
    let p = match payload() {
        Some(p) => p,
        None => return,
    };
    if str_field(&p, "tool_name") != Some("Read") {
        return;
    }
    let input = match p.get("tool_input") {
        Some(i) => i,
        None => return,
    };
    let file_path = match str_field(input, "file_path") {
        Some(f) => f,
        None => return,
    };

    // Already bounded - the model is doing the right thing.
    let bounded = |k: &str| matches!(input.get(k), Some(v) if !v.is_null());
    if bounded("offset") || bounded("limit") {
        return;
    }
    if !outline::is_source(file_path) {
        return;
    }

    let meta = match std::fs::metadata(file_path) {
        Ok(m) => m,
        Err(_) => return,
    };
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return;
    }

    // Deny a given path at most once per session; a second request means the model
    // genuinely wants the file, and a denial loop would wedge it.
    let session_id = str_field(&p, "session_id").unwrap_or("");
    let mut st = state::load(session_id);
    if st.reads.contains_key(file_path) {
        return;
    }

    let bytes = match std::fs::read(file_path) {
        Ok(b) => b,
        Err(_) => return,
    };
    let text = String::from_utf8_lossy(&bytes);

    let o = outline::outline(&text, file_path, 200);
    if o.lines < line_threshold() {
        return;
    }
    // A sparse outline misses the file's structure; the model would re-read anyway
    // and the denial would have cost a round trip for nothing.
    if !outline::covers(o.lines, o.rows.len()) {
        return;
    }

    st.reads.insert(file_path.to_string(), 1);
    state::save(session_id, &st);

    emit_deny(&outline::render(file_path, o.lines, &o.rows));
}

// ------------------------------------------------------------------------ main

fn main() {
    // Never let a bug block a tool call: swallow panics, always exit 0.
    let _ = std::panic::catch_unwind(|| {
        let cmd = std::env::args().nth(1).unwrap_or_default();
        match cmd.as_str() {
            "lazy-rules" => lazy_rules(),
            "lazy-rules-post" => lazy_rules_post(),
            "read-discipline" => read_discipline(),
            _ => {
                eprintln!(
                    "usage: omp-hooks <lazy-rules|lazy-rules-post|read-discipline>\n\
                     reads a Claude Code hook payload as JSON on stdin"
                );
            }
        }
    });
    std::process::exit(0);
}
