//! WP-143: crash-safe multi-file writes shared by the Code and Session stores.
//!
//! Every part is written to a temp file in its target's directory, flushed
//! and fsynced. Only then are the temps renamed into place, in slice order,
//! and the directories fsynced. Callers pass their commit marker (meta) last,
//! so a crash during the renames leaves a renamed prefix that the loader's
//! consistency check can repair.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::types::{LitecodeError, Result};

/// One target file in an atomic set. `write` streams the full new content.
pub struct AtomicPart<'a> {
    pub target: &'a Path,
    pub write: &'a dyn Fn(&mut dyn Write) -> Result<()>,
}

const TMP_MARK: &str = ".tmp-";

static SEQ: AtomicU64 = AtomicU64::new(0);

fn err(e: impl std::fmt::Display) -> LitecodeError {
    LitecodeError::Config(format!("atomic write: {e}"))
}

fn tmp_path(target: &Path) -> Result<PathBuf> {
    let name = target
        .file_name()
        .ok_or_else(|| err(format!("no file name: {}", target.display())))?
        .to_string_lossy();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = format!(".{name}{TMP_MARK}{}-{seq}", std::process::id());
    Ok(target.with_file_name(tmp))
}

#[cfg(test)]
thread_local! {
    /// Crash injection: `Some(k)` fails the set after `k` renames.
    pub(crate) static FAIL_AFTER_RENAMES: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
    /// Crash injection: fail while writing the temps (no rename happens).
    /// Test counter: bytes written by append_durable / append_fsync /
    /// atomic_write_bytes (shared with Session WP-144 acceptance 3).
    pub static WRITTEN_BYTES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// WP-143 test counter: bytes written by `append_durable`.
    pub(crate) static APPEND_BYTES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    pub(crate) static FAIL_MID_TMP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test counter: parent-directory fsyncs issued.
    pub(crate) static DIR_SYNCS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Write each part to a temp file in the target's own directory, flush and
/// fsync each one, then rename them into place in slice order (commit marker
/// last), then fsync the directories. Any failure before the renames removes
/// every temp and leaves the old files untouched.
pub fn atomic_write_set(parts: &[AtomicPart<'_>]) -> Result<()> {
    let mut tmps: Vec<PathBuf> = Vec::with_capacity(parts.len());
    let staged = (|| -> Result<()> {
        for (i, part) in parts.iter().enumerate() {
            if let Some(dir) = part.target.parent() {
                std::fs::create_dir_all(dir).map_err(err)?;
            }
            let tmp = tmp_path(part.target)?;
            tmps.push(tmp.clone());
            let file = File::create(&tmp).map_err(err)?;
            let mut w = BufWriter::new(file);
            (part.write)(&mut w)?;
            #[cfg(test)]
            if i + 1 == parts.len() && FAIL_MID_TMP.with(|f| f.get()) {
                return Err(err("injected failure while writing temps"));
            }
            let _ = i;
            let file = w.into_inner().map_err(|e| err(e.error()))?;
            file.sync_all().map_err(err)?;
        }
        Ok(())
    })();
    if let Err(e) = staged {
        for tmp in &tmps {
            let _ = std::fs::remove_file(tmp);
        }
        return Err(e);
    }
    for (i, (part, tmp)) in parts.iter().zip(&tmps).enumerate() {
        #[cfg(test)]
        if FAIL_AFTER_RENAMES.with(|f| f.get()) == Some(i) {
            return Err(err(format!("injected crash after {i} renames")));
        }
        let _ = i;
        std::fs::rename(tmp, part.target).map_err(err)?;
    }
    let mut dirs: Vec<&Path> = parts.iter().filter_map(|p| p.target.parent()).collect();
    dirs.dedup();
    for dir in dirs {
        sync_dir(dir);
    }
    Ok(())
}

/// Single-file convenience: [`atomic_write_set`] with one part.
pub fn atomic_write_bytes(target: &Path, bytes: &[u8]) -> Result<()> {
    let write = |w: &mut dyn Write| w.write_all(bytes).map_err(err);
    atomic_write_set(&[AtomicPart {
        target,
        write: &write,
    }])?;
    count_written(bytes.len());
    Ok(())
}

/// Append `bytes` to `data` and fsync it. A tail beyond `committed_len`
/// (a crashed, uncommitted append) is truncated first. Returns the new
/// length for the caller's checkpoint. A file shorter than `committed_len`
/// is an error: the caller repairs.
pub fn append_durable(data: &Path, committed_len: u64, bytes: &[u8]) -> Result<u64> {
    if let Some(dir) = data.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    let created = !data.exists();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(data)
        .map_err(err)?;
    let len = file.metadata().map_err(err)?.len();
    if len < committed_len {
        return Err(err(format!(
            "{} shorter ({len}) than committed ({committed_len})",
            data.display()
        )));
    }
    if len > committed_len {
        file.set_len(committed_len).map_err(err)?;
    }
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(committed_len)).map_err(err)?;
    file.write_all(bytes).map_err(err)?;
    file.sync_data().map_err(err)?;
    if created && let Some(dir) = data.parent() {
        sync_dir(dir); // the new directory entry survives a crash too
    }
    #[cfg(test)]
    APPEND_BYTES.with(|c| c.set(c.get() + bytes.len() as u64));
    count_written(bytes.len());
    Ok(committed_len + bytes.len() as u64)
}

/// Plain append + fsync (no tail check); returns the file's new length.
/// For callers whose recovery already ran [`truncate_to_committed`] on load
/// (Session WP-144 shape). Prefer [`append_durable`] otherwise.
pub fn append_fsync(data: &Path, bytes: &[u8]) -> Result<u64> {
    if let Some(dir) = data.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    let created = !data.exists();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data)
        .map_err(err)?;
    file.write_all(bytes).map_err(err)?;
    file.sync_data().map_err(err)?;
    if created && let Some(dir) = data.parent() {
        sync_dir(dir);
    }
    count_written(bytes.len());
    file.metadata().map(|m| m.len()).map_err(err)
}

fn count_written(_n: usize) {
    #[cfg(test)]
    WRITTEN_BYTES.with(|b| b.set(b.get() + _n as u64));
}

/// Cut `data` back to `committed_len` (drop an uncommitted tail). A missing
/// file with `committed_len == 0` is fine; a short file is an error.
pub fn truncate_to_committed(data: &Path, committed_len: u64) -> Result<()> {
    let file = match std::fs::OpenOptions::new().write(true).open(data) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && committed_len == 0 => return Ok(()),
        Err(e) => return Err(err(e)),
    };
    let len = file.metadata().map_err(err)?.len();
    if len < committed_len {
        return Err(err(format!(
            "{} shorter ({len}) than committed ({committed_len})",
            data.display()
        )));
    }
    if len > committed_len {
        file.set_len(committed_len).map_err(err)?;
        file.sync_data().map_err(err)?;
    }
    Ok(())
}

/// Remove leftover temp files in `dir` (call on load, before reading).
pub fn sweep_atomic_tmp(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') && name.contains(TMP_MARK) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    #[cfg(test)]
    DIR_SYNCS.with(|c| c.set(c.get() + 1));
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_set(a: &Path, b: &Path, va: &str, vb: &str) -> Result<()> {
        let wa = |w: &mut dyn Write| w.write_all(va.as_bytes()).map_err(err);
        let wb = |w: &mut dyn Write| w.write_all(vb.as_bytes()).map_err(err);
        atomic_write_set(&[
            AtomicPart {
                target: a,
                write: &wa,
            },
            AtomicPart {
                target: b,
                write: &wb,
            },
        ])
    }

    #[test]
    fn crash_between_renames_leaves_prefix_and_sweep_clears_tmp() {
        let dir = TempDir::new().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        write_set(&a, &b, "a1", "b1").unwrap();
        FAIL_AFTER_RENAMES.with(|f| f.set(Some(1)));
        assert!(write_set(&a, &b, "a2", "b2").is_err());
        FAIL_AFTER_RENAMES.with(|f| f.set(None));
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a2");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b1");
        sweep_atomic_tmp(dir.path());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

}
