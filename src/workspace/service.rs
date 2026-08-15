use std::sync::Arc;

use tokio::sync::broadcast;

use super::change::WorkspaceChange;
use super::sandbox::{Sandbox, SandboxError};
use super::tree::{TreeEntry, TreeError, list_tree};

/// Maximum file size for read/write (10 MB), matching the read tool.
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("not a file: {0}")]
    NotFile(String),
    #[error("is a directory: {0}")]
    IsDir(String),
    #[error("file too large (max {MAX_FILE_SIZE} bytes)")]
    TooLarge,
    #[error("content is not valid UTF-8")]
    NotUtf8,
    #[error("invalid move: {0}")]
    InvalidMove(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct WorkspaceService {
    sandbox: Sandbox,
    change_tx: broadcast::Sender<WorkspaceChange>,
}

impl WorkspaceService {
    pub fn new(project_root: std::path::PathBuf) -> Result<Arc<Self>, WorkspaceError> {
        let sandbox = Sandbox::new(project_root)?;
        let (change_tx, _) = broadcast::channel(512);
        Ok(Arc::new(Self { sandbox, change_tx }))
    }

    pub fn sandbox(&self) -> &Sandbox {
        &self.sandbox
    }

    pub fn subscribe_changes(&self) -> broadcast::Receiver<WorkspaceChange> {
        self.change_tx.subscribe()
    }

    pub fn change_sender(&self) -> broadcast::Sender<WorkspaceChange> {
        self.change_tx.clone()
    }

    fn emit_changed(&self, paths: Vec<String>, kind: &str) {
        let _ = self.change_tx.send(WorkspaceChange {
            paths,
            kind: kind.to_string(),
        });
    }

    pub fn tree(&self, path: &str, depth: usize) -> Result<Vec<TreeEntry>, WorkspaceError> {
        let sandbox = self.sandbox();
        Ok(list_tree(&sandbox, path, depth)?)
    }

    pub fn read_file(&self, path: &str) -> Result<(String, String), WorkspaceError> {
        let (rel, bytes) = self.read_file_bytes(path)?;
        let content = String::from_utf8(bytes).map_err(|_| WorkspaceError::NotUtf8)?;
        Ok((rel, content))
    }

    /// Read a workspace file through the shared sandbox and size limits.
    ///
    /// Consumers that support binary formats (such as Agent image reads) can
    /// interpret the bytes themselves without bypassing workspace policy.
    pub fn read_file_bytes(&self, path: &str) -> Result<(String, Vec<u8>), WorkspaceError> {
        let sandbox = self.sandbox();
        let abs = sandbox.resolve(path)?;
        if !abs.exists() {
            return Err(WorkspaceError::NotFound(
                sandbox.rel_path(&abs).unwrap_or_else(|_| path.into()),
            ));
        }
        if !abs.is_file() {
            return Err(WorkspaceError::NotFile(
                sandbox.rel_path(&abs).unwrap_or_else(|_| path.into()),
            ));
        }
        let meta = abs.metadata()?;
        if meta.len() > MAX_FILE_SIZE {
            return Err(WorkspaceError::TooLarge);
        }
        let bytes = std::fs::read(&abs)?;
        let rel = sandbox.rel_path(&abs)?;
        Ok((rel, bytes))
    }

    pub fn write_file(&self, path: &str, content: &str) -> Result<String, WorkspaceError> {
        if content.len() as u64 > MAX_FILE_SIZE {
            return Err(WorkspaceError::TooLarge);
        }
        let sandbox = self.sandbox();
        let abs = sandbox.resolve(path)?;
        if abs.is_dir() {
            return Err(WorkspaceError::IsDir(
                sandbox.rel_path(&abs).unwrap_or_else(|_| path.into()),
            ));
        }
        let existed = abs.exists();
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        super::file_ops::atomic_write(&abs, content)?;
        let rel = sandbox.rel_path(&abs)?;
        let kind = if existed { "modified" } else { "created" };
        self.emit_changed(vec![rel.clone()], kind);
        Ok(rel)
    }

    pub fn create_file(&self, path: &str, content: &str) -> Result<String, WorkspaceError> {
        if content.len() as u64 > MAX_FILE_SIZE {
            return Err(WorkspaceError::TooLarge);
        }
        let sandbox = self.sandbox();
        let abs = sandbox.resolve(path)?;
        if abs.exists() {
            return Err(WorkspaceError::AlreadyExists(
                sandbox.rel_path(&abs).unwrap_or_else(|_| path.into()),
            ));
        }
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        super::file_ops::atomic_write(&abs, content)?;
        let rel = sandbox.rel_path(&abs)?;
        self.emit_changed(vec![rel.clone()], "created");
        Ok(rel)
    }

    pub fn delete_path(&self, path: &str, recursive: bool) -> Result<String, WorkspaceError> {
        let sandbox = self.sandbox();
        let abs = sandbox.resolve(path)?;
        if !abs.exists() {
            return Err(WorkspaceError::NotFound(
                sandbox.rel_path(&abs).unwrap_or_else(|_| path.into()),
            ));
        }
        let rel = sandbox.rel_path(&abs)?;
        super::recycle::remove_path(&abs, recursive)?;
        self.emit_changed(vec![rel.clone()], "deleted");
        Ok(rel)
    }

    pub fn mkdir(&self, path: &str) -> Result<String, WorkspaceError> {
        let sandbox = self.sandbox();
        let abs = sandbox.resolve(path)?;
        let rel = sandbox.rel_path(&abs)?;
        if abs.exists() {
            return Err(WorkspaceError::AlreadyExists(rel));
        }
        std::fs::create_dir_all(&abs)?;
        self.emit_changed(vec![rel.clone()], "created");
        Ok(rel)
    }

    pub fn rename_path(
        &self,
        from: &str,
        to: &str,
        overwrite: bool,
    ) -> Result<(String, String), WorkspaceError> {
        let sandbox = self.sandbox();
        let from_abs = sandbox.resolve(from)?;
        if !from_abs.exists() {
            return Err(WorkspaceError::NotFound(
                sandbox.rel_path(&from_abs).unwrap_or_else(|_| from.into()),
            ));
        }
        let from_rel = sandbox.rel_path(&from_abs)?;
        let to_abs = sandbox.resolve(to)?;
        let to_rel = sandbox.rel_path(&to_abs)?;
        if from_rel == to_rel {
            return Ok((from_rel, to_rel));
        }
        if is_same_or_descendant(&from_rel, &to_rel) {
            return Err(WorkspaceError::InvalidMove(
                "cannot move a path into itself".into(),
            ));
        }
        if from_rel.is_empty() {
            return Err(WorkspaceError::InvalidMove(
                "cannot move the workspace root".into(),
            ));
        }
        refuse_directory_overwrite(&from_abs, &to_abs, overwrite, &to_rel)?;
        if let Some(parent) = to_abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let dest_existed = to_abs.exists();
        match std::fs::rename(&from_abs, &to_abs) {
            Ok(()) => {}
            Err(_) if dest_existed && overwrite && is_file_replace(&from_abs, &to_abs) => {
                super::file_ops::replace_file(&from_abs, &to_abs)?;
            }
            Err(error) if is_cross_device(&error) => {
                move_via_copy(&from_abs, &to_abs)?;
            }
            Err(error) => return Err(error.into()),
        }
        self.emit_changed(vec![from_rel.clone(), to_rel.clone()], "renamed");
        Ok((from_rel, to_rel))
    }

    pub fn copy_path(
        &self,
        from: &str,
        to: &str,
        overwrite: bool,
    ) -> Result<String, WorkspaceError> {
        let sandbox = self.sandbox();
        let from_abs = sandbox.resolve(from)?;
        if !from_abs.exists() {
            return Err(WorkspaceError::NotFound(
                sandbox.rel_path(&from_abs).unwrap_or_else(|_| from.into()),
            ));
        }
        let from_rel = sandbox.rel_path(&from_abs)?;
        if from_rel.is_empty() {
            return Err(WorkspaceError::InvalidMove(
                "cannot copy the workspace root".into(),
            ));
        }
        let to_abs = sandbox.resolve(to)?;
        let to_rel = sandbox.rel_path(&to_abs)?;
        if is_same_or_descendant(&from_rel, &to_rel) {
            return Err(WorkspaceError::InvalidMove(
                "cannot copy a path into itself".into(),
            ));
        }
        refuse_directory_overwrite(&from_abs, &to_abs, overwrite, &to_rel)?;
        if let Some(parent) = to_abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        copy_recursive(&from_abs, &to_abs).map_err(map_copy_error)?;
        self.emit_changed(vec![to_rel.clone()], "created");
        Ok(to_rel)
    }

    /// Write raw bytes (OS drop / binary). `overwrite=false` fails if the path exists.
    pub fn write_file_bytes(
        &self,
        path: &str,
        bytes: &[u8],
        overwrite: bool,
    ) -> Result<String, WorkspaceError> {
        if bytes.len() as u64 > MAX_FILE_SIZE {
            return Err(WorkspaceError::TooLarge);
        }
        let sandbox = self.sandbox();
        let abs = sandbox.resolve(path)?;
        if abs.is_dir() {
            return Err(WorkspaceError::IsDir(
                sandbox.rel_path(&abs).unwrap_or_else(|_| path.into()),
            ));
        }
        let existed = abs.exists();
        if existed && !overwrite {
            return Err(WorkspaceError::AlreadyExists(
                sandbox.rel_path(&abs).unwrap_or_else(|_| path.into()),
            ));
        }
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        super::file_ops::atomic_write_bytes(&abs, bytes)?;
        let rel = sandbox.rel_path(&abs)?;
        let kind = if existed { "modified" } else { "created" };
        self.emit_changed(vec![rel.clone()], kind);
        Ok(rel)
    }
}

fn refuse_directory_overwrite(
    from_abs: &std::path::Path,
    to_abs: &std::path::Path,
    overwrite: bool,
    to_rel: &str,
) -> Result<(), WorkspaceError> {
    if !to_abs.exists() {
        return Ok(());
    }
    let from_dir = from_abs.is_dir();
    let to_dir = to_abs.is_dir();
    if from_dir || to_dir {
        return Err(WorkspaceError::InvalidMove(
            "refusing to overwrite a directory".into(),
        ));
    }
    if !overwrite {
        return Err(WorkspaceError::AlreadyExists(to_rel.into()));
    }
    Ok(())
}

fn is_file_replace(from_abs: &std::path::Path, to_abs: &std::path::Path) -> bool {
    from_abs.is_file() && to_abs.is_file()
}

fn map_copy_error(error: std::io::Error) -> WorkspaceError {
    if error.kind() == std::io::ErrorKind::InvalidInput {
        WorkspaceError::InvalidMove(error.to_string())
    } else {
        WorkspaceError::Io(error)
    }
}

/// Copy then delete source. On copy failure, remove the incomplete destination
/// and leave the source untouched.
fn move_via_copy(
    from_abs: &std::path::Path,
    to_abs: &std::path::Path,
) -> Result<(), WorkspaceError> {
    if let Err(error) = copy_recursive(from_abs, to_abs) {
        cleanup_incomplete_dest(to_abs);
        return Err(map_copy_error(error));
    }
    if from_abs.is_dir() {
        std::fs::remove_dir_all(from_abs)?;
    } else if from_abs.exists() {
        std::fs::remove_file(from_abs)?;
    }
    Ok(())
}

fn cleanup_incomplete_dest(path: &std::path::Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

fn is_same_or_descendant(ancestor: &str, candidate: &str) -> bool {
    if ancestor.is_empty() {
        return true;
    }
    candidate == ancestor || candidate.starts_with(&format!("{ancestor}/"))
}

fn is_cross_device(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(18) // EXDEV on unix
        || error.kind() == std::io::ErrorKind::CrossesDevices
}

fn copy_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to copy symbolic links",
        ));
    }
    if meta.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let child_meta = std::fs::symlink_metadata(entry.path())?;
            if child_meta.file_type().is_symlink() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "refusing to copy symbolic links",
                ));
            }
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if dst.exists() {
            let tmp = dst.with_file_name(format!(
                ".{}.litecode-copy-{}",
                dst.file_name().unwrap_or_default().to_string_lossy(),
                std::process::id()
            ));
            std::fs::copy(src, &tmp)?;
            let persist = super::file_ops::replace_file(&tmp, dst);
            if persist.is_err() || tmp.exists() {
                let _ = std::fs::remove_file(&tmp);
            }
            persist
        } else {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(src, dst)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn change_broadcast_is_bounded_and_lagged_receiver_recovers() {
        // The workspace change broadcast is `broadcast::channel(512)` (bounded).
        // A receiver that never drains must observe Lagged rather than the
        // channel buffering events without limit (REV-8 / FIX-2).
        let dir = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(dir.path().to_path_buf()).unwrap();

        // A slow subscriber: subscribe but never drain until the end.
        let mut slow = service.subscribe_changes();

        // Drive the producer beyond the bounded capacity while slow is idle.
        let tx = service.change_sender();
        let overflow = 512 + 100;
        for i in 0..overflow {
            tx.send(WorkspaceChange {
                paths: vec![format!("f{i}.txt")],
                kind: "modified".into(),
            })
            .unwrap_or_else(|_| {
                panic!("broadcast send must not fail while a receiver is attached")
            });
        }

        // Draining the slow receiver now yields a Lagged error (bounded, not
        // silently dropping or unbounded growth).
        match slow.try_recv() {
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(skipped)) => {
                assert!(skipped >= overflow - 512, "skipped={skipped}");
            }
            other => panic!("expected Lagged on overflow, got {other:?}"),
        }
        // The receiver is still usable: drain the retained window (at most the
        // 512-message broadcast capacity) and confirm it reaches the newest
        // event produced after the overflow. A bounded drain (`try_recv`) keeps
        // this test from hanging forever on a live broadcast sender, unlike an
        // unbounded `recv().await` loop which would never observe `Closed`.
        let mut last: Option<String> = None;
        for _ in 0..512 {
            match slow.try_recv() {
                Ok(change) => last = Some(change.paths[0].clone()),
                Err(_) => break,
            }
        }
        assert_eq!(
            last.as_deref(),
            Some(format!("f{}.txt", overflow - 1).as_str())
        );
    }

    #[test]
    fn mkdir_rename_copy_and_reject_self_copy() {
        let dir = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(dir.path().to_path_buf()).unwrap();

        service.mkdir("sub/nested").unwrap();
        assert!(dir.path().join("sub/nested").is_dir());
        assert!(matches!(
            service.mkdir("sub/nested"),
            Err(WorkspaceError::AlreadyExists(_))
        ));

        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let (from, to) = service.rename_path("a.txt", "b.txt", false).unwrap();
        assert_eq!(from, "a.txt");
        assert_eq!(to, "b.txt");
        assert!(!dir.path().join("a.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("b.txt")).unwrap(),
            "hello"
        );

        service.copy_path("b.txt", "sub/c.txt", false).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("sub/c.txt")).unwrap(),
            "hello"
        );

        std::fs::write(dir.path().join("sub/nested/inner.txt"), "x").unwrap();
        service.copy_path("sub", "sub2", false).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("sub2/nested/inner.txt")).unwrap(),
            "x"
        );

        assert!(matches!(
            service.copy_path("sub", "sub/deeper", false),
            Err(WorkspaceError::InvalidMove(_))
        ));
        assert!(matches!(
            service.rename_path("sub", "sub/deeper", false),
            Err(WorkspaceError::InvalidMove(_))
        ));
        assert!(matches!(
            service.rename_path("b.txt", "sub/c.txt", false),
            Err(WorkspaceError::AlreadyExists(_))
        ));

        service
            .write_file_bytes("pic.bin", &[0, 1, 255], false)
            .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("pic.bin")).unwrap(),
            vec![0, 1, 255]
        );
        assert!(matches!(
            service.write_file_bytes("pic.bin", &[2], false),
            Err(WorkspaceError::AlreadyExists(_))
        ));
        service.write_file_bytes("pic.bin", &[9], true).unwrap();
        assert_eq!(std::fs::read(dir.path().join("pic.bin")).unwrap(), vec![9]);
    }

    #[test]
    fn refuses_directory_overwrite_and_renames_directories() {
        let dir = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(dir.path().to_path_buf()).unwrap();
        std::fs::create_dir(dir.path().join("a")).unwrap();
        std::fs::create_dir(dir.path().join("b")).unwrap();
        std::fs::write(dir.path().join("a/x.txt"), "x").unwrap();
        std::fs::write(dir.path().join("file.txt"), "f").unwrap();

        assert!(matches!(
            service.rename_path("a", "b", true),
            Err(WorkspaceError::InvalidMove(_))
        ));
        assert!(dir.path().join("a/x.txt").exists());
        assert!(dir.path().join("b").exists());

        assert!(matches!(
            service.copy_path("file.txt", "a", true),
            Err(WorkspaceError::InvalidMove(_))
        ));

        let (from, to) = service.rename_path("a", "c", false).unwrap();
        assert_eq!(from, "a");
        assert_eq!(to, "c");
        assert!(dir.path().join("c/x.txt").exists());
        assert!(!dir.path().join("a").exists());
    }

    #[test]
    fn delete_removes_file_from_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(dir.path().to_path_buf()).unwrap();
        std::fs::write(dir.path().join("gone.txt"), "bye").unwrap();
        service.delete_path("gone.txt", false).unwrap();
        assert!(!dir.path().join("gone.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_copy_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(dir.path().to_path_buf()).unwrap();
        std::fs::write(dir.path().join("real.txt"), "x").unwrap();
        std::os::unix::fs::symlink(dir.path().join("real.txt"), dir.path().join("link.txt"))
            .unwrap();
        assert!(matches!(
            service.copy_path("link.txt", "out.txt", false),
            Err(WorkspaceError::InvalidMove(_))
        ));
        assert!(!dir.path().join("out.txt").exists());
    }
}
