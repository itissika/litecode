//! `stamps.v1` — persisted mtime/len stamps for the code index (WP-141).
//!
//! Lives next to the index (`.litecode/index/stamps.v1`) and is only trusted
//! while it describes the index on disk. Line 1 is a JSON header; every other
//! line is `<mtime_ns>\t<len>\t<rel_path>` for a stamped path, or
//! `?\t?\t<rel_path>` for a path the index holds without a stamp yet (the
//! keys are the complete set of paths the index has accounted for, so a
//! cooled index can still detect deletions). Paths containing `\n` are never
//! written; they fall back to a content check every pass (safe side).
//!
//! Invalid ⇒ the file is deleted and the caller treats every path as
//! unstamped (full read). Invalid means: unparsable header, `v != 1`,
//! pipeline/embedder/root differ from the index, or `index_created_at` differs
//! from `meta.json` (the index was rewritten after the stamps: rebuild,
//! replacement, or a crash between index commit and stamp write).
//! Write order: index first, stamps second, so a crash only costs extra reads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::FileStamp;
use super::meta::{self, IndexMeta};

pub const STAMPS_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Header {
    v: u32,
    pipeline_version: u32,
    embedder_id: String,
    root: String,
    index_created_at: String,
}

/// Path → stamp (`None` = indexed, not stamped yet).
pub type StampEntries = HashMap<String, Option<FileStamp>>;

pub fn stamps_path(workspace_root: &Path) -> PathBuf {
    meta::index_dir(workspace_root).join("stamps.v1")
}

fn root_key(workspace_root: &Path) -> String {
    crate::config::path::canon_abs_lossy(workspace_root)
        .to_string_lossy()
        .into_owned()
}

fn header_for(workspace_root: &Path, meta: &IndexMeta) -> Header {
    Header {
        v: STAMPS_VERSION,
        pipeline_version: meta.pipeline_version,
        embedder_id: meta.embedder_id.clone(),
        root: root_key(workspace_root),
        index_created_at: meta.created_at.clone(),
    }
}

/// Drop the stamps (rebuild start, invalid file).
pub fn discard(workspace_root: &Path) {
    let _ = std::fs::remove_file(stamps_path(workspace_root));
}

/// Load and validate. `None` when absent or invalid (an invalid file is
/// deleted so the next pass does not re-parse it).
pub fn load(workspace_root: &Path) -> Option<StampEntries> {
    let body = std::fs::read_to_string(stamps_path(workspace_root)).ok()?;
    let parsed = parse(workspace_root, &body);
    if parsed.is_none() {
        tracing::info!("code_search stamps.v1 invalid; discarding (full read)");
        discard(workspace_root);
    }
    parsed
}

fn parse(workspace_root: &Path, body: &str) -> Option<StampEntries> {
    let meta = meta::read_meta(workspace_root).ok().flatten()?;
    if meta::needs_rebuild(&meta) {
        return None;
    }
    let mut lines = body.split('\n');
    let header: Header = serde_json::from_str(lines.next()?).ok()?;
    if header != header_for(workspace_root, &meta) {
        return None;
    }
    let mut out = StampEntries::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        let (m, l, path) = (parts.next()?, parts.next()?, parts.next()?);
        // WP-143: appended lines override earlier ones; `-` drops the path.
        if m == "-" && l == "-" {
            out.remove(path);
            continue;
        }
        let stamp = if m == "?" && l == "?" {
            None
        } else {
            Some(FileStamp {
                mtime_ns: m.parse().ok()?,
                len: l.parse().ok()?,
            })
        };
        out.insert(path.to_string(), stamp);
    }
    Some(out)
}

/// Write after the index commit. Skipped (and any old file dropped) when the
/// index has no `meta.json` to bind to.
pub fn save(workspace_root: &Path, entries: &StampEntries) {
    let Some(meta) = meta::read_meta(workspace_root).ok().flatten() else {
        discard(workspace_root);
        return;
    };
    let Ok(header) = serde_json::to_string(&header_for(workspace_root, &meta)) else {
        return;
    };
    let mut paths: Vec<&String> = entries.keys().filter(|p| !p.contains('\n')).collect();
    paths.sort();
    let mut body = String::with_capacity(64 + paths.len() * 48);
    body.push_str(&header);
    body.push('\n');
    for path in paths {
        match entries[path] {
            Some(s) => body.push_str(&format!("{}\t{}\t{path}\n", s.mtime_ns, s.len)),
            None => body.push_str(&format!("?\t?\t{path}\n")),
        }
    }
    let path = stamps_path(workspace_root);
    let Some(dir) = path.parent() else { return };
    let _ = std::fs::create_dir_all(dir);
    let tmp = dir.join(format!(".stamps.v1.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, body).is_err() || std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// One appended change: `Some(Some(stamp))` stamped, `Some(None)` indexed
/// without a stamp, `None` = path dropped.
pub type StampChange = Option<Option<FileStamp>>;

fn change_line(path: &str, change: &StampChange) -> String {
    match change {
        Some(Some(s)) => format!("{}\t{}\t{path}\n", s.mtime_ns, s.len),
        Some(None) => format!("?\t?\t{path}\n"),
        None => format!("-\t-\t{path}\n"),
    }
}

/// WP-143: append changes to a valid `stamps.v1` (header still matches the
/// index meta). Returns false when the caller must write the full file
/// (absent, other header, or an I/O error). Bytes ≈ the changes.
pub fn append(workspace_root: &Path, changes: &[(String, StampChange)]) -> bool {
    use std::io::{BufRead, Write};
    let Some(meta) = meta::read_meta(workspace_root).ok().flatten() else {
        return false;
    };
    let path = stamps_path(workspace_root);
    let Ok(file) = std::fs::File::open(&path) else {
        return false;
    };
    let mut first = String::new();
    if std::io::BufReader::new(file).read_line(&mut first).is_err() {
        return false;
    }
    let Ok(header) = serde_json::from_str::<Header>(first.trim_end()) else {
        return false;
    };
    if header != header_for(workspace_root, &meta) {
        return false;
    }
    let mut body = String::new();
    for (p, c) in changes {
        if !p.contains('\n') {
            body.push_str(&change_line(p, c));
        }
    }
    let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&path) else {
        return false;
    };
    // A torn tail line only fails the parse ⇒ full read; never wrong data.
    f.write_all(body.as_bytes()).is_ok() && f.sync_data().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn seed_meta(root: &Path) -> IndexMeta {
        let meta = IndexMeta::shell();
        meta::write_meta(root, &meta).unwrap();
        meta
    }

    fn sample() -> StampEntries {
        let mut e = StampEntries::new();
        e.insert(
            "a.rs".into(),
            Some(FileStamp {
                mtime_ns: 7,
                len: 3,
            }),
        );
        e.insert("dir/b c.txt".into(), None);
        e
    }

    #[test]
    fn invalidation_matrix_discards_the_file() {
        type Mutate = fn(&Path);
        let cases: Vec<(&str, Mutate)> = vec![
            ("bad header", |r| {
                std::fs::write(stamps_path(r), "not json\n1\t2\ta.rs\n").unwrap()
            }),
            ("version", |r| {
                let body = std::fs::read_to_string(stamps_path(r)).unwrap();
                std::fs::write(stamps_path(r), body.replacen("\"v\":1", "\"v\":2", 1)).unwrap();
            }),
            ("embedder changed", |r| {
                let mut m = meta::read_meta(r).unwrap().unwrap();
                m.embedder_id = "other".into();
                meta::write_meta(r, &m).unwrap();
            }),
            ("index rewritten (created_at)", |r| {
                let mut m = meta::read_meta(r).unwrap().unwrap();
                m.created_at = "2000-01-01T00:00:00Z".into();
                meta::write_meta(r, &m).unwrap();
            }),
            ("pipeline bump", |r| {
                let mut m = meta::read_meta(r).unwrap().unwrap();
                m.pipeline_version += 1;
                meta::write_meta(r, &m).unwrap();
            }),
            ("meta gone", |r| {
                std::fs::remove_file(meta::meta_path(r)).unwrap();
            }),
            ("bad line", |r| {
                let mut body = std::fs::read_to_string(stamps_path(r)).unwrap();
                body.push_str("x\t1\tz.rs\n");
                std::fs::write(stamps_path(r), body).unwrap();
            }),
        ];
        for (name, mutate) in cases {
            let dir = TempDir::new().unwrap();
            seed_meta(dir.path());
            save(dir.path(), &sample());
            mutate(dir.path());
            assert_eq!(load(dir.path()), None, "{name}");
            assert!(!stamps_path(dir.path()).exists(), "{name}: file discarded");
        }
    }

}
