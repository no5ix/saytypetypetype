//! ASR model lifecycle guard: the model is a permanent resident.
//!
//! Keeping the local recognizer loaded for the whole application session is a
//! product requirement — the user must be able to dictate the moment the
//! global hotkey fires, without paying a multi-second model load. An earlier
//! design unloaded the cached recognizers after five idle minutes; that is
//! gone, and these tests keep it from creeping back in.
//!
//! The tests read the production sources at test time (no model, no network),
//! so they run in the ordinary `cargo test` pass.

use std::path::Path;

/// Every production (non-`tests/`) Rust source file under `src-tauri/src`.
fn production_sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // The tests directory is where the profiling harness lives; it is
            // allowed to talk about unloading.
            if path.file_name().and_then(|n| n.to_str()) == Some("tests") {
                continue;
            }
            production_sources(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn sources() -> Vec<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    production_sources(&root, &mut files);
    assert!(
        files.len() > 20,
        "expected to scan the whole backend, found {} files",
        files.len()
    );
    files
}

/// Identifiers whose appearance in production code means a timed/idle unload
/// has been reintroduced.
const FORBIDDEN: &[&str] = &[
    "idle_unload",
    "IDLE_UNLOAD",
    "IdleUnload",
    "schedule_idle_model_unload",
    "model_idle_ticket",
    "auto_unload",
    "AUTO_UNLOAD",
    "evict_model",
];

#[test]
fn no_production_code_schedules_a_timed_model_unload() {
    for path in sources() {
        let text = std::fs::read_to_string(&path).expect("source readable");
        for needle in FORBIDDEN {
            assert!(
                !text.contains(needle),
                "{} mentions {needle}: an idle/timed ASR model unload must not \
                 exist — the recognizer has to stay resident for the whole app \
                 session so dictation starts instantly",
                path.display()
            );
        }
    }
}

/// Each `unload` call in production must carry an explicit, user-initiated
/// reason. A reason computed from a timer (a format string mentioning seconds
/// or "idle") is the shape the removed 5-minute policy used.
#[test]
fn every_production_unload_names_an_explicit_reason() {
    let allowed = [
        "app exit",
        "ASR config changed",
        "hotwords changed",
        "model deleted",
        // pass-through of the caller's reason inside SherpaModelCache::unload
        "reason",
        // the cache's own unit tests live inside the production module
        "test",
    ];
    let mut calls = 0usize;
    for path in sources() {
        let text = std::fs::read_to_string(&path).expect("source readable");
        for (idx, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if !trimmed.contains(".unload(") {
                continue;
            }
            calls += 1;
            let ok = allowed
                .iter()
                .any(|reason| trimmed.contains(reason) && !trimmed.contains("idle"));
            assert!(
                ok,
                "{}:{} unloads the ASR cache for a reason that is not an \
                 explicit user action: {trimmed}",
                path.display(),
                idx + 1
            );
        }
    }
    // The scan must actually see the sanctioned call sites.
    assert!(
        calls >= 4,
        "expected the known unload call sites, saw {calls}"
    );
}
