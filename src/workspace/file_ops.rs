//! Shared filesystem primitives for the human workspace API and Agent tools.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

const SYMLINK_FOLLOW_LIMIT: u32 = 40;

/// Write content through a sibling temporary file, then persist it into place.
///
/// New files get standard permissions (0644 on unix). Overwriting a regular file
/// copies the original `permissions()` onto the replacement. Symlinks are written
/// through (the link node stays). Hard-linked files (`nlink > 1`) keep their inode.
pub fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    atomic_write_follow(path, content.as_bytes(), 0)
}

/// Same as [`atomic_write`] for raw bytes (OS drop / binary files).
pub fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    atomic_write_follow(path, bytes, 0)
}

fn atomic_write_follow(path: &Path, content: &[u8], depth: u32) -> std::io::Result<()> {
    if depth > SYMLINK_FOLLOW_LIMIT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "too many levels of symbolic links",
        ));
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let target = std::fs::read_link(path)?;
            let dest = resolve_link_target(path, &target);
            atomic_write_follow(&dest, content, depth + 1)
        }
        Ok(meta) => persist_regular(path, content, Some(meta)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            persist_regular(path, content, None)
        }
        Err(error) => Err(error),
    }
}

fn resolve_link_target(link: &Path, target: &Path) -> PathBuf {
    if target.is_absolute() {
        target.to_path_buf()
    } else {
        match link.parent() {
            Some(parent) => parent.join(target),
            None => target.to_path_buf(),
        }
    }
}

fn persist_regular(
    path: &Path,
    content: &[u8],
    existing: Option<std::fs::Metadata>,
) -> std::io::Result<()> {
    let tmp = unique_tmp_path(path);
    {
        #[cfg(unix)]
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        opts.mode(0o644);
        let mut file = opts.open(&tmp)?;
        file.write_all(content)?;
        file.sync_all()?;
    }

    let persist_result = match existing.as_ref() {
        Some(meta) if nlink(meta) > 1 => copy_into_existing(&tmp, path),
        Some(meta) => {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
            match std::fs::rename(&tmp, path) {
                Ok(()) => Ok(()),
                Err(rename_error) => copy_into_existing(&tmp, path).or(Err(rename_error)),
            }
        }
        None => match std::fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(rename_error) => {
                if std::fs::copy(&tmp, path).is_err() {
                    Err(rename_error)
                } else {
                    Ok(())
                }
            }
        },
    };

    if persist_result.is_err() || tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
    }
    persist_result?;

    #[cfg(unix)]
    if let Some(parent) = path.parent()
        && let Ok(dir) = std::fs::File::open(parent)
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

fn unique_tmp_path(path: &Path) -> PathBuf {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(
        ".{file_name}.litecode-tmp-{}-{seq}",
        std::process::id()
    ))
}

fn copy_into_existing(tmp: &Path, path: &Path) -> std::io::Result<()> {
    let mut src = std::fs::File::open(tmp)?;
    let mut dest = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)?;
    let mut buf = Vec::new();
    src.read_to_end(&mut buf)?;
    dest.write_all(&buf)?;
    dest.sync_all()?;
    Ok(())
}

fn nlink(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.nlink()
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        1
    }
}

/// Rename `from` onto `to`, replacing an existing regular file. Directories
/// are never replaced here — callers must refuse that case first.
pub fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MoveFileExW};
        let from_w: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to_w: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        let ok = unsafe { MoveFileExW(from_w.as_ptr(), to_w.as_ptr(), MOVEFILE_REPLACE_EXISTING) };
        if ok == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_file_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("sub").join("note.txt");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        atomic_write(&target, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("sub"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("litecode-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files must be cleaned up");
    }

    #[test]
    fn atomic_write_overwrites_existing_content() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("note.txt");
        atomic_write(&target, "first").unwrap();
        atomic_write(&target, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "second");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_sets_standard_permissions_on_create() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("note.txt");
        atomic_write(&target, "body").unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o644,
            "new file must be world-readable, owner-writable (0644), got {mode:o}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_execute_bit_on_overwrite() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("script.sh");
        std::fs::write(&target, "#!/bin/sh\n").unwrap();
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms).unwrap();
        atomic_write(&target, "#!/bin/sh\necho hi\n").unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "overwrite must keep +x, got {mode:o}");
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "#!/bin/sh\necho hi\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_writes_through_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.txt");
        let link = dir.path().join("alias.txt");
        std::fs::write(&target, "old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        atomic_write(&link, "new").unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "link node must remain a symlink"
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "new");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_hardlink_inode() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "old").unwrap();
        std::fs::hard_link(&a, &b).unwrap();
        atomic_write(&a, "shared").unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "shared");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "shared");
    }
}
