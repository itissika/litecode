//! Architecture gate: sessions.db SQL and rusqlite stay inside SessionData sqlite.
//!
//! The writer owns ordering, transactions, and receipts; it must not learn the
//! shape of the tables. This scans production code only. Test modules and the
//! test-only fixture file are excluded because they intentionally open the DB
//! to assert what production wrote.

use std::fs;
use std::path::{Path, PathBuf};

const SQL_NEEDLES: &[&str] = &[
    "use rusqlite",
    "rusqlite::",
    "\"SELECT ",
    "\"INSERT INTO",
    "\"UPDATE ",
    "\"DELETE FROM",
    "\"CREATE TABLE",
    "\"BEGIN IMMEDIATE",
    "\"COMMIT",
    "\"ROLLBACK",
];

fn walk_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn is_comment_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") || t.starts_with("//!") || t.starts_with("///")
}

fn is_test_only_file(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized.ends_with("/src/session/data/sqlite/test_support.rs")
        || normalized.ends_with("/src/session/data/final_immutability_tests.rs")
}

/// Drop the body of a module guarded by `#[cfg(test)]`. Inline test-only fields
/// and methods are left in place; they cannot contain the raw SQL shapes this
/// gate watches without being reported, which is the safer direction.
fn strip_cfg_test_modules(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < lines.len() {
        if lines[i].trim() == "#[cfg(test)]" {
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim().is_empty() {
                j += 1;
            }
            if j < lines.len() && lines[j].trim_start().starts_with("mod ") {
                if lines[j].trim_end().ends_with(';') {
                    i = j + 1;
                    continue;
                }
                let mut depth = 0i32;
                let mut opened = false;
                let mut end = j;
                for (k, line) in lines.iter().enumerate().skip(j) {
                    let trimmed = line.trim();
                    depth += trimmed.matches('{').count() as i32;
                    depth -= trimmed.matches('}').count() as i32;
                    opened |= trimmed.contains('{');
                    end = k;
                    if opened && depth <= 0 {
                        break;
                    }
                }
                i = end + 1;
                continue;
            }
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

#[test]
fn gate_needles_recognize_a_raw_sql_line() {
    let probe = "let sql = \"SELECT 1\";";
    assert!(SQL_NEEDLES.iter().any(|needle| probe.contains(needle)));
}

#[test]
fn gate_scanner_keeps_production_after_a_test_module() {
    let source = "#[cfg(test)]\nmod tests {\n    const SQL: &str = \"SELECT 1\";\n}\nfn production() { let _ = 1; }\n";
    let production = strip_cfg_test_modules(source);
    assert!(production.contains("fn production"));
    assert!(!production.contains("const SQL"));
}

#[test]
fn sessions_db_sql_and_rusqlite_stay_in_the_sqlite_module() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/session");
    let mut files = Vec::new();
    walk_rs_files(&root, &mut files);
    files.sort();
    assert!(files.len() > 10, "scanner walked {} files", files.len());

    let mut scanned = 0usize;
    let mut saw_writer = false;
    let mut hits = Vec::new();

    for path in files {
        let normalized = path.to_string_lossy().replace('\\', "/");
        if is_test_only_file(&path) || normalized.contains("/src/session/data/sqlite/") {
            continue;
        }
        if normalized.ends_with("/src/session/data/writer.rs") {
            saw_writer = true;
        }
        scanned += 1;
        let source = fs::read_to_string(&path).unwrap_or_default();
        let production = strip_cfg_test_modules(&source);
        for (index, line) in production.lines().enumerate() {
            if is_comment_line(line) {
                continue;
            }
            for needle in SQL_NEEDLES {
                if line.contains(needle) {
                    hits.push(format!("{normalized}:{}: {needle}", index + 1));
                }
            }
        }
    }

    assert!(scanned > 0, "no production session files were scanned");
    assert!(saw_writer, "writer.rs was not part of the scanned set");
    assert!(
        hits.is_empty(),
        "raw sessions.db SQL escaped the sqlite module:\n{}",
        hits.join("\n")
    );
}
