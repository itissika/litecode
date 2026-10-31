//! One read policy for every knowledge entry point.
//!
//! The turn note, `knowledge check`, `/symbol-refs`, `/api/workspace/knowledge`
//! and user-message expansion all decide "does this cited path exist, and can
//! it be read as text" here, so they cannot disagree (KR-INV-2, B4/B5).
//!
//! - Names match the on-disk directory entry exactly, segment by segment,
//!   case-sensitive on every OS, with no Win32 trailing-dot/space trimming
//!   (KR-PATH-4).
//! - Symlinks are followed; the resolved target must stay inside the
//!   workspace (KR-LINK-1).
//! - Text = a regular file of at most [`MAX_TEXT_BYTES`] that decodes as UTF-8
//!   (KR-READ-1).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::mentions::is_workspace_file_ref;

/// Same cap as the workspace file API.
pub const MAX_TEXT_BYTES: u64 = crate::workspace::MAX_FILE_SIZE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Not a legal workspace path, or no exact on-disk entry (or a broken link).
    Missing,
    /// A symlink on the way resolves outside the workspace.
    Escape,
    Dir,
    File,
}

/// What a stat saw. Equal stamps mean the cached parse is still valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stamp {
    pub kind: Kind,
    pub len: u64,
    pub mtime: Option<SystemTime>,
}

impl Stamp {
    pub const MISSING: Stamp = Stamp {
        kind: Kind::Missing,
        len: 0,
        mtime: None,
    };
}

/// Why an existing path is not readable text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unreadable {
    Directory,
    TooLarge,
    NotText,
}

impl Unreadable {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::TooLarge => "larger than 10 MiB",
            Self::NotText => "not UTF-8 text",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextRead {
    Missing,
    Escape,
    Unreadable(Unreadable),
    Text(String),
}

/// Directory listings for one pass. Exact-name checks list each parent once.
#[derive(Default)]
pub struct Listings {
    dirs: HashMap<PathBuf, Option<HashMap<String, bool>>>,
    root_canon: Option<Option<PathBuf>>,
}

impl Listings {
    /// `Some(is_symlink)` when `dir` holds an entry named exactly `name`.
    fn entry(&mut self, dir: &Path, name: &str) -> Option<bool> {
        let listing = self
            .dirs
            .entry(dir.to_path_buf())
            .or_insert_with(|| list_dir(dir));
        listing.as_ref()?.get(name).copied()
    }

    fn root_canon(&mut self, workspace: &Path) -> Option<PathBuf> {
        self.root_canon
            .get_or_insert_with(|| std::fs::canonicalize(workspace).ok())
            .clone()
    }
}

fn list_dir(dir: &Path) -> Option<HashMap<String, bool>> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut names = HashMap::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let link = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        names.insert(name, link);
    }
    Some(names)
}

/// Exact-name stat of a cited workspace path.
pub fn lookup(workspace: &Path, rel: &str, listings: &mut Listings) -> Stamp {
    if !is_workspace_file_ref(rel) {
        return Stamp::MISSING;
    }
    let rel = rel.replace('\\', "/");
    let mut cur = workspace.to_path_buf();
    let mut linked = false;
    for seg in rel.split('/') {
        match listings.entry(&cur, seg) {
            Some(is_link) => linked |= is_link,
            None => return Stamp::MISSING,
        }
        cur.push(seg);
    }
    let Ok(meta) = std::fs::metadata(&cur) else {
        return Stamp::MISSING;
    };
    if linked {
        let inside = match (listings.root_canon(workspace), std::fs::canonicalize(&cur)) {
            (Some(root), Ok(target)) => target.starts_with(&root),
            _ => false,
        };
        if !inside {
            return Stamp {
                kind: Kind::Escape,
                len: 0,
                mtime: None,
            };
        }
    }
    Stamp {
        kind: if meta.is_dir() { Kind::Dir } else { Kind::File },
        len: meta.len(),
        mtime: meta.modified().ok(),
    }
}

/// Read a cited path under the single policy. Returns the stamp it read under.
pub fn read_text(workspace: &Path, rel: &str, listings: &mut Listings) -> (Stamp, TextRead) {
    let stamp = lookup(workspace, rel, listings);
    let read = match stamp.kind {
        Kind::Missing => TextRead::Missing,
        Kind::Escape => TextRead::Escape,
        Kind::Dir => TextRead::Unreadable(Unreadable::Directory),
        Kind::File if stamp.len > MAX_TEXT_BYTES => TextRead::Unreadable(Unreadable::TooLarge),
        Kind::File => match std::fs::read(workspace.join(rel.replace('\\', "/"))) {
            Err(_) => TextRead::Missing,
            Ok(bytes) if bytes.len() as u64 > MAX_TEXT_BYTES => {
                TextRead::Unreadable(Unreadable::TooLarge)
            }
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => TextRead::Text(text),
                Err(_) => TextRead::Unreadable(Unreadable::NotText),
            },
        },
    };
    (stamp, read)
}

/// One-shot [`read_text`] for callers outside a pass.
pub fn read_text_once(workspace: &Path, rel: &str) -> TextRead {
    read_text(workspace, rel, &mut Listings::default()).1
}

/// Lines in a text as the line-range rules count them (KR-RANGE-2):
/// `str::lines`, so a final newline does not add an empty line.
pub fn line_count(text: &str) -> u32 {
    u32::try_from(text.lines().count()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {}
