//! Live per-session record of files this session has touched.
//!
//! The map is process memory only. A restart starts empty. Our own tool writes
//! are absorbed after each tool batch so the next seam reports only external
//! changes.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAX_TRACKED: usize = 256;
const HASH_LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Snap {
    mtime: Option<SystemTime>,
    len: u64,
    hash: Option<String>,
    missing: bool,
}

#[derive(Debug, Default)]
pub struct FileTracker {
    order: VecDeque<PathBuf>,
    snaps: HashMap<PathBuf, Snap>,
}

impl FileTracker {
    /// Remember `path` and snapshot it. The newest paths stay when the cap is hit.
    pub fn record(&mut self, path: &Path) {
        let path = path.to_path_buf();
        if let Some(pos) = self.order.iter().position(|existing| existing == &path) {
            self.order.remove(pos);
        }
        self.order.push_back(path.clone());
        self.snaps
            .insert(path, snap_of(self.order.back().expect("pushed")));
        while self.order.len() > MAX_TRACKED {
            if let Some(old) = self.order.pop_front() {
                self.snaps.remove(&old);
            }
        }
    }

    pub fn record_paths<I, P>(&mut self, paths: I)
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        for path in paths {
            self.record(path.as_ref());
        }
    }

    /// Re-snapshot every tracked file so this session's own writes are not reported.
    pub fn absorb(&mut self) {
        let paths: Vec<PathBuf> = self.order.iter().cloned().collect();
        for path in paths {
            self.snaps.insert(path.clone(), snap_of(&path));
        }
    }

    /// Paths whose mtime, length, hash, or presence differ from the last snapshot.
    /// Those snapshots are updated to the observed state.
    pub fn take_changes(&mut self) -> Vec<PathBuf> {
        let paths: Vec<PathBuf> = self.order.iter().cloned().collect();
        let mut changed = Vec::new();
        for path in paths {
            let next = snap_of(&path);
            let differs = self
                .snaps
                .get(&path)
                .is_none_or(|previous| previous != &next);
            if differs {
                changed.push(path.clone());
            }
            self.snaps.insert(path, next);
        }
        changed
    }
}

fn snap_of(path: &Path) -> Snap {
    let Ok(meta) = fs::metadata(path) else {
        return Snap {
            mtime: None,
            len: 0,
            hash: None,
            missing: true,
        };
    };
    let len = meta.len();
    let mtime = meta.modified().ok();
    let hash = if len <= HASH_LIMIT {
        fs::read(path).ok().map(|bytes| {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(bytes))
        })
    } else {
        None
    };
    Snap {
        mtime,
        len,
        hash,
        missing: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absorb_hides_our_write_and_take_changes_reports_external() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "one").unwrap();
        let mut tracker = FileTracker::default();
        tracker.record(&path);
        fs::write(&path, "ours").unwrap();
        tracker.absorb();
        assert!(tracker.take_changes().is_empty());
        fs::write(&path, "theirs").unwrap();
        let changed = tracker.take_changes();
        assert_eq!(changed, vec![path]);
    }

    #[test]
    fn deletion_is_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone.txt");
        fs::write(&path, "x").unwrap();
        let mut tracker = FileTracker::default();
        tracker.record(&path);
        fs::remove_file(&path).unwrap();
        assert_eq!(tracker.take_changes(), vec![path]);
    }
}
