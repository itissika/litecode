//! Disk-space and directory-size helpers.

use std::path::Path;

pub fn available_bytes(path: &Path) -> Option<u64> {
    fs2::available_space(path).ok()
}

pub fn directory_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let mut total: u64 = 0;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        total = total.saturating_add(directory_size(&entry.path()));
    }
    total
}
