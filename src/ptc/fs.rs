//! Read-only filesystem probes inside `run_code` (WP-210 I).
//!
//! `fs.ls(path)`, `fs.stat(path)` and `fs.exists(path)` are synchronous and
//! never write. There is deliberately no `rm`, `mv`, or write: several agents
//! share one workspace and `libs`/`fs` stay read-only by invariant. Path scope
//! follows the caller's `read` permission mode (`SAFE` stays confined to the
//! workspace; `ALL` permits read-only probes outside the workspace).

use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value, json};

/// Entries returned by one `fs.ls` (Q12).
pub const LS_CAP: usize = 500;

/// Resolve `raw` under `root`.
///
/// When `allow_outside_workspace` is `false` (`SAFE`), `..` may not climb out,
/// absolute paths must sit inside `root`, and symlinks must target inside `root`.
/// When `allow_outside_workspace` is `true` (`ALL`), read-only paths outside
/// `root` are allowed, matching `tools.read`.
fn resolve_path(
    root: &Path,
    raw: &str,
    allow_outside_workspace: bool,
) -> Result<PathBuf, (&'static str, String)> {
    if raw.trim().is_empty() {
        return Err(("invalid_input", "path must be non-empty".into()));
    }
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
                if !normal.pop() && !allow_outside_workspace {
                    return Err(("permission", format!("{raw} is outside the workspace")));
                }
            }
            Component::CurDir => {}
            other => normal.push(other.as_os_str()),
        }
    }
    if allow_outside_workspace {
        return Ok(normal);
    }
    let root_real = crate::config::path::canon_abs(root).unwrap_or_else(|_| root.to_path_buf());
    let inside = |path: &Path| path.starts_with(root) || path.starts_with(&root_real);
    if !inside(&normal) {
        return Err(("permission", format!("{raw} is outside the workspace")));
    }
    if let Ok(real) = crate::config::path::canon_abs(&normal)
        && !inside(&real)
    {
        return Err((
            "permission",
            format!("{raw} resolves outside the workspace"),
        ));
    }
    Ok(normal)
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

fn io_failure(raw: &str, error: std::io::Error) -> (&'static str, String) {
    if error.kind() == std::io::ErrorKind::NotFound {
        ("not_found", format!("{raw} does not exist"))
    } else {
        ("io", format!("{raw}: {error}"))
    }
}

/// `{entries:[{name,type,size}]}` — one level, sorted by name, capped at `LS_CAP`.
/// Returns `(value, Some(total))` when more than `LS_CAP` entries existed so the
/// host can surface a run warning instead of cluttering normal output.
pub fn ls(
    root: &Path,
    raw: &str,
    allow_outside_workspace: bool,
) -> Result<(Value, Option<usize>), (&'static str, String)> {
    let path = resolve_path(root, raw, allow_outside_workspace)?;
    let reader = std::fs::read_dir(&path).map_err(|error| io_failure(raw, error))?;
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
    out.insert(
        "entries".into(),
        Value::Array(
            entries
                .into_iter()
                .map(|(name, kind, size)| json!({ "name": name, "type": kind, "size": size }))
                .collect(),
        ),
    );
    let truncated_total = (total > LS_CAP).then_some(total);
    Ok((Value::Object(out), truncated_total))
}

/// `{type, size, mtime}` with `mtime` in ms since the Unix epoch.
pub fn stat(
    root: &Path,
    raw: &str,
    allow_outside_workspace: bool,
) -> Result<Value, (&'static str, String)> {
    let path = resolve_path(root, raw, allow_outside_workspace)?;
    match std::fs::metadata(&path) {
        Ok(meta) => {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|age| age.as_millis() as u64)
                .unwrap_or(0);
            Ok(json!({ "type": kind(&meta), "size": meta.len(), "mtime": mtime }))
        }
        Err(error) => Err(io_failure(raw, error)),
    }
}

/// `true` for an existing path permitted under the active workspace scope.
pub fn exists(root: &Path, raw: &str, allow_outside_workspace: bool) -> bool {
    resolve_path(root, raw, allow_outside_workspace).is_ok_and(|path| path.exists())
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

        let (listed, truncated) = ls(root, ".", false).expect("ls root");
        assert_eq!(truncated, None);
        assert!(listed.get("truncated").is_none());
        let names: Vec<&str> = listed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["b.txt", "src"]);
        assert_eq!(listed["entries"][0]["type"], "file");
        assert_eq!(listed["entries"][0]["size"], 2);

        let info = stat(root, "src/a.rs", false).expect("stat file");
        assert_eq!(
            (info["type"].clone(), info["size"].clone()),
            (json!("file"), json!(9))
        );
        assert!(info["mtime"].as_u64().unwrap() > 0);
        assert!(exists(root, "src", false));
        assert!(!exists(root, "nope", false));

        assert_eq!(stat(root, "nope", false).unwrap_err().0, "not_found");
        assert_eq!(ls(root, "../", false).unwrap_err().0, "permission");
        assert!(!exists(root, "../", false));

        // Nothing was created or removed by the probes.
        assert_eq!(std::fs::read_dir(root).unwrap().count(), 2);

        // ALL mode permits read-only probes outside the workspace.
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("ext.txt"), "ok").unwrap();
        let outside_dir = outside.path().to_string_lossy().into_owned();
        let outside_file = outside.path().join("ext.txt").to_string_lossy().into_owned();
        assert_eq!(ls(root, &outside_dir, false).unwrap_err().0, "permission");
        assert!(ls(root, &outside_dir, true).is_ok());
        assert_eq!(stat(root, &outside_file, true).expect("stat ext")["size"], 2);
        assert!(exists(root, &outside_file, true));
    }

    /// Top-level key names declared in the first object literal after `marker`.
    fn declared_keys(marker: &str, libs: &str) -> std::collections::BTreeSet<String> {
        let start = libs
            .find(marker)
            .unwrap_or_else(|| panic!("marker not found in sdk/src/libs.d.ts: {marker}"));
        let body = &libs[start + marker.len()..];
        let open = body.find('{').expect("object type");
        let mut depth = 0usize;
        let mut segment = String::new();
        let mut keys = std::collections::BTreeSet::new();
        for ch in body[open..].chars() {
            match ch {
                '{' => {
                    depth += 1;
                    if depth == 1 {
                        segment.clear();
                    } else {
                        segment.push(ch);
                    }
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        record_key(&segment, &mut keys);
                        break;
                    }
                    segment.push(ch);
                }
                ';' if depth == 1 => {
                    record_key(&segment, &mut keys);
                    segment.clear();
                }
                _ => segment.push(ch),
            }
        }
        keys
    }

    fn record_key(segment: &str, keys: &mut std::collections::BTreeSet<String>) {
        let name = segment
            .split(':')
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches('?');
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            keys.insert(name.to_string());
        }
    }

    fn runtime_keys(value: &Value) -> std::collections::BTreeSet<String> {
        value
            .as_object()
            .expect("object result")
            .keys()
            .cloned()
            .collect()
    }

    /// The SDK is the model-facing contract for `fs`; every key it declares must
    /// exist at runtime and vice versa, so the declaration cannot drift silently.
    #[test]
    fn sdk_declaration_matches_runtime_keys() {
        let libs = include_str!("../../sdk/src/libs.d.ts");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("b.txt"), "hi").unwrap();

        let (listed, truncated) = ls(root, ".", false).expect("ls");
        assert_eq!(truncated, None);
        assert_eq!(declared_keys("  ls(path: string): ", libs), runtime_keys(&listed));
        assert_eq!(
            declared_keys("entries: Array<", libs),
            runtime_keys(&listed["entries"][0]),
        );

        let statted = stat(root, "b.txt", false).expect("stat");
        assert_eq!(declared_keys("  stat(path: string): ", libs), runtime_keys(&statted));

        assert!(libs.contains("  exists(path: string): boolean;"));
    }
}
