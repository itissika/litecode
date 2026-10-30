//! Read-only filesystem probes inside `run_code` (WP-210 I).
//!
//! `fs.ls(path)`, `fs.stat(path)` and `fs.exists(path)` are synchronous,
//! workspace-confined, and never write. There is deliberately no `rm`, `mv`,
//! or write: several agents share one workspace and `libs`/`fs` stay
//! read-only by invariant. Writes go through `tools.file.write` / `tools.file.edit`.

use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value, json};

/// Entries returned by one `fs.ls` (Q12).
pub const LS_CAP: usize = 500;

/// Resolve `raw` under `root`. `..` may not climb out; absolute paths must
/// already be inside `root`. Symlinks are followed only when the target is
/// still inside `root`.
fn confine(root: &Path, raw: &str) -> Result<PathBuf, Value> {
    let candidate = Path::new(raw);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let mut normal = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                if !normal.pop() {
                    return Err(failure("permission", format!("{raw} is outside the workspace")));
                }
            }
            Component::CurDir => {}
            other => normal.push(other.as_os_str()),
        }
    }
    let root_real = crate::config::path::canon_abs(root).unwrap_or_else(|_| root.to_path_buf());
    let inside = |path: &Path| path.starts_with(root) || path.starts_with(&root_real);
    if !inside(&normal) {
        return Err(failure("permission", format!("{raw} is outside the workspace")));
    }
    if let Ok(real) = crate::config::path::canon_abs(&normal)
        && !inside(&real)
    {
        return Err(failure("permission", format!("{raw} resolves outside the workspace")));
    }
    Ok(normal)
}

fn failure(reason: &str, message: String) -> Value {
    json!({ "ok": false, "reason": reason, "message": message })
}

fn kind(meta: &std::fs::Metadata) -> &'static str {
    if meta.is_dir() {
        "dir"
    } else if meta.is_file() {
        "file"
    } else if meta.file_type().is_symlink() {
        "symlink"
    } else {
        "other"
    }
}

fn io_failure(raw: &str, error: std::io::Error) -> Value {
    if error.kind() == std::io::ErrorKind::NotFound {
        failure("not_found", format!("{raw} does not exist"))
    } else {
        failure("io", format!("{raw}: {error}"))
    }
}

/// `{ok, entries:[{name,type,size}], message?}` — one level, sorted by name.
pub fn ls(root: &Path, raw: &str) -> Value {
    let path = match confine(root, raw) {
        Ok(path) => path,
        Err(value) => return value,
    };
    let reader = match std::fs::read_dir(&path) {
        Ok(reader) => reader,
        Err(error) => return io_failure(raw, error),
    };
    let mut entries: Vec<(String, &'static str, u64)> = Vec::new();
    for entry in reader.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let (kind, size) = match entry.metadata() {
            Ok(meta) => (kind(&meta), if meta.is_file() { meta.len() } else { 0 }),
            Err(_) => ("other", 0),
        };
        entries.push((name, kind, size));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let total = entries.len();
    entries.truncate(LS_CAP);
    let mut out = Map::new();
    out.insert("ok".into(), Value::Bool(true));
    out.insert(
        "entries".into(),
        Value::Array(
            entries
                .into_iter()
                .map(|(name, kind, size)| json!({ "name": name, "type": kind, "size": size }))
                .collect(),
        ),
    );
    if total > LS_CAP {
        out.insert(
            "message".into(),
            Value::String(format!("showing {LS_CAP} of {total} entries; use tools.file.glob to filter")),
        );
    }
    Value::Object(out)
}

/// `{ok, type, size, mtime}` with `mtime` in ms since the Unix epoch.
pub fn stat(root: &Path, raw: &str) -> Value {
    let path = match confine(root, raw) {
        Ok(path) => path,
        Err(value) => return value,
    };
    match std::fs::metadata(&path) {
        Ok(meta) => {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|age| age.as_millis() as u64)
                .unwrap_or(0);
            json!({ "ok": true, "type": kind(&meta), "size": meta.len(), "mtime": mtime })
        }
        Err(error) => io_failure(raw, error),
    }
}

/// `true` only for an existing path inside the workspace.
pub fn exists(root: &Path, raw: &str) -> bool {
    confine(root, raw).is_ok_and(|path| path.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_probes_are_read_only_and_workspace_confined() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn a() {}").unwrap();
        std::fs::write(root.join("b.txt"), "hi").unwrap();

        let listed = ls(root, ".");
        assert_eq!(listed["ok"], true);
        let names: Vec<&str> = listed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["b.txt", "src"]);
        assert_eq!(listed["entries"][0]["type"], "file");
        assert_eq!(listed["entries"][0]["size"], 2);

        let info = stat(root, "src/a.rs");
        assert_eq!((info["ok"].clone(), info["type"].clone(), info["size"].clone()), (json!(true), json!("file"), json!(9)));
        assert!(info["mtime"].as_u64().unwrap() > 0);
        assert!(exists(root, "src"));
        assert!(!exists(root, "nope"));

        assert_eq!(stat(root, "nope")["reason"], "not_found");
        assert_eq!(ls(root, "../")["reason"], "permission");
        assert!(!exists(root, "../"));

        // Nothing was created or removed by the probes.
        assert_eq!(std::fs::read_dir(root).unwrap().count(), 2);
    }
}
