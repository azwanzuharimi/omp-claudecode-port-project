//! Per-session hook state: fire-once bookkeeping for rules, deny-once for reads.
//! Mirrors hooks/lib/state.js exactly, including the JSON field names, because
//! the JS and Rust hooks must be able to share a state file.

use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Default, Clone)]
pub struct State {
    pub calls: u64,
    pub fired: BTreeMap<String, u64>,
    pub pending: Vec<String>,
    pub reads: BTreeMap<String, u64>,
}

pub fn config_dir() -> PathBuf {
    if let Ok(d) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".claude")
}

pub fn state_dir() -> PathBuf {
    config_dir()
        .join("state")
        .join("omp-claudecode-port-project")
}

/// Same rule as state.js: keep [A-Za-z0-9._-], replace everything else with '_',
/// then truncate to 128 chars.
pub fn sanitize(id: &str) -> String {
    let id = if id.is_empty() { "nosession" } else { id };
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(128)
        .collect()
}

pub fn state_path(session_id: &str) -> PathBuf {
    state_dir().join(format!("{}.json", sanitize(session_id)))
}

fn u64_of(v: Option<&Value>) -> u64 {
    v.and_then(Value::as_u64).unwrap_or(0)
}

pub fn load(session_id: &str) -> State {
    let raw = match std::fs::read_to_string(state_path(session_id)) {
        Ok(s) => s,
        Err(_) => return State::default(),
    };
    let v: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return State::default(),
    };
    let mut st = State {
        calls: u64_of(v.get("calls")),
        ..Default::default()
    };
    if let Some(o) = v.get("fired").and_then(Value::as_object) {
        for (k, val) in o {
            st.fired.insert(k.clone(), u64_of(Some(val)));
        }
    }
    if let Some(a) = v.get("pending").and_then(Value::as_array) {
        st.pending = a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect();
    }
    if let Some(o) = v.get("reads").and_then(Value::as_object) {
        for (k, val) in o {
            st.reads.insert(k.clone(), u64_of(Some(val)));
        }
    }
    st
}

/// A state-write failure must never block a tool call, so every error is dropped.
pub fn save(session_id: &str, st: &State) {
    let mut fired = Map::new();
    for (k, v) in &st.fired {
        fired.insert(k.clone(), Value::from(*v));
    }
    let mut reads = Map::new();
    for (k, v) in &st.reads {
        reads.insert(k.clone(), Value::from(*v));
    }
    let mut root = Map::new();
    root.insert("calls".into(), Value::from(st.calls));
    root.insert("fired".into(), Value::Object(fired));
    root.insert(
        "pending".into(),
        Value::Array(st.pending.iter().cloned().map(Value::from).collect()),
    );
    root.insert("reads".into(), Value::Object(reads));

    let dir = state_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::write(state_path(session_id), Value::Object(root).to_string());
}
