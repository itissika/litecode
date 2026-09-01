//! Architecture gate: sessions.db SQL and rusqlite stay inside SessionData sqlite.

use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "node_modules" | "target" | "dist" | ".git") {
                continue;
            }
            walk(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn norm(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_sqlite_ok(path: &Path) -> bool {
    let s = norm(path);
    s.contains("/src/session/data/sqlite/")
}

fn is_global_db_ok(path: &Path) -> bool {
    let s = norm(path);
    s.contains("/src/config/global_db/")
}

fn is_shared_error_type(path: &Path) -> bool {
    norm(path).ends_with("/src/types/error.rs")
}

fn is_path_construct_ok(path: &Path) -> bool {
    let s = norm(path);
    s.ends_with("/src/config/resolved.rs")
        || s.ends_with("/src/config/workspace.rs")
        || s.contains("/src/session/data/")
        || s.ends_with("/src/serve/state.rs")
}

/// Skip `#[cfg(test)]` modules when scanning production dialect.
fn production_lines(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut test_depth: i32 = 0;
    let mut pending_test_mod = false;
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[cfg(test)]") {
            pending_test_mod = true;
        }
        if pending_test_mod && trimmed.starts_with("mod ") && trimmed.contains('{') {
            test_depth = 1;
            pending_test_mod = false;
            continue;
        }
        if pending_test_mod && trimmed.starts_with("mod ") && trimmed.ends_with(';') {
            pending_test_mod = false;
            continue;
        }
        if test_depth > 0 {
            test_depth += trimmed.matches('{').count() as i32;
            test_depth -= trimmed.matches('}').count() as i32;
            if test_depth <= 0 {
                test_depth = 0;
            }
            continue;
        }
        out.push((idx + 1, line.to_string()));
    }
    out
}

#[test]
fn sessions_db_sql_and_rusqlite_stay_in_sqlite_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);

    let mut hits = Vec::new();
    for path in &files {
        if is_sqlite_ok(path) || is_global_db_ok(path) || is_shared_error_type(path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for (line_no, line) in production_lines(&text) {
            if line.contains("use rusqlite") || line.contains("rusqlite::") {
                hits.push(format!("{}:{line_no}: rusqlite", norm(path)));
            }
            if line.contains("CREATE TABLE") && line.contains("transcript_items") {
                hits.push(format!("{}:{line_no}: transcript SQL", norm(path)));
            }
            if line.contains("SessionGate")
                || line.contains("with_entry_store")
                || line.contains("sessions_db_under")
                || line.contains("transcript_fts::ensure_ready")
            {
                hits.push(format!("{}:{line_no}: leftover symbol", norm(path)));
            }
            if line.contains("Session::open(")
                || line.contains("Session::resume(")
                || line.contains("Session::delete(")
                || line.contains("Session::list_sessions(")
                || line.contains("Session::list_child_session_ids(")
            {
                hits.push(format!("{}:{line_no}: leftover Session CRUD", norm(path)));
            }
            if line.contains(r#"join("sessions.db")"#) && !is_path_construct_ok(path) {
                hits.push(format!("{}:{line_no}: sessions.db join", norm(path)));
            }
            if line.contains("SessionDataReader::open(") {
                hits.push(format!(
                    "{}:{line_no}: reader must be injected or reconstructed inside SessionData",
                    norm(path)
                ));
            }
            if line.contains("for_legacy_root(") && line.contains("sessions_db") {
                hits.push(format!(
                    "{}:{line_no}: sessions.db path reconstruction",
                    norm(path)
                ));
            }
        }
    }

    assert!(
        hits.is_empty(),
        "sessions.db architecture violations:\n{}",
        hits.join("\n")
    );
}

#[test]
fn leftover_symbols_absent_from_repo() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);
    walk(&root.join("tests"), &mut files);
    let needles = [
        "SessionGate",
        "with_entry_store",
        "sessions_db_under",
        "transcript_fts::ensure_ready",
    ];
    let mut hits = Vec::new();
    for path in &files {
        let s = norm(path);
        if s.contains("/session_data_sql_death_list.rs") || s.contains("/f2_lock_scope.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for needle in needles {
                if line.contains(needle) {
                    hits.push(format!("{}:{}: {needle}", s, i + 1));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "forbidden leftover symbols:\n{}",
        hits.join("\n")
    );
}
