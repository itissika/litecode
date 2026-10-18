use std::sync::Arc;

use tokio::sync::broadcast;

use super::change::WorkspaceChange;
use super::sandbox::{Sandbox, SandboxError};
use super::tree::{GlobListing, TreeEntry, TreeError, list_glob, list_tree, list_tree_reveal};

/// Maximum file size for read/write (10 MB), matching the read tool.
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum bytes the file panel will download for an image, PDF, or media preview.
pub const MAX_PREVIEW_BYTES: u64 = 32 * 1024 * 1024;

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
    #[error("File is too large to preview in the panel.")]
    PreviewTooLarge,
    #[error("Binary file. Can't display it here.")]
    Binary,
    #[error("This file is UTF-16. Opening it as UTF-8 here would corrupt it.")]
    Utf16,
    #[error("content is not valid UTF-8 (if this is a GBK/ANSI file, convert it to UTF-8 first)")]
    NotUtf8,
    #[error("Not a SQLite database. Can't preview it.")]
    NotSqlite,
    #[error("Unknown table.")]
    UnknownTable,
    #[error("offset is too large")]
    PreviewOffset,
    #[error("could not read sqlite database: {0}")]
    Sqlite(String),
    #[error("opening with the system app is not available on this host")]
    OpenUnsupported,
    #[error("{0}")]
    OpenFailed(String),
    #[error("invalid move: {0}")]
    InvalidMove(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn preview_mime(path: &str) -> &'static str {
    let ext = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "opus" => "audio/ogg",
        "weba" => "audio/webm",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

pub struct WorkspaceService {
    sandbox: Sandbox,
    change_tx: broadcast::Sender<WorkspaceChange>,
    /// WP-141: the watcher lost track (notify error, Rescan flag, queue
    /// overflow). Consumers run a full disk reconcile; no paths attached.
    rescan_tx: broadcast::Sender<()>,
}

impl WorkspaceService {
    pub fn new(project_root: std::path::PathBuf) -> Result<Arc<Self>, WorkspaceError> {
        let sandbox = Sandbox::new(project_root)?;
        let (change_tx, _) = broadcast::channel(512);
        let (rescan_tx, _) = broadcast::channel(16);
        Ok(Arc::new(Self {
            sandbox,
            change_tx,
            rescan_tx,
        }))
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

    pub fn subscribe_rescan(&self) -> broadcast::Receiver<()> {
        self.rescan_tx.subscribe()
    }

    pub fn rescan_sender(&self) -> broadcast::Sender<()> {
        self.rescan_tx.clone()
    }

    fn emit_changed(&self, paths: Vec<String>, kind: &str) {
        let _ = self.change_tx.send(WorkspaceChange {
            paths,
            kind: kind.to_string(),
        });
    }

    pub fn tree(&self, path: &str, depth: usize) -> Result<Vec<TreeEntry>, WorkspaceError> {
        let sandbox = self.sandbox();
        Ok(list_tree(sandbox, path, depth)?)
    }

    pub fn tree_reveal(&self, path: &str) -> Result<Vec<(String, Vec<TreeEntry>)>, WorkspaceError> {
        Ok(list_tree_reveal(self.sandbox(), path)?)
    }

    pub fn glob(&self, pattern: &str) -> Result<GlobListing, WorkspaceError> {
        Ok(list_glob(self.sandbox(), pattern)?)
    }

    pub fn read_file(&self, path: &str) -> Result<(String, String), WorkspaceError> {
        let (rel, bytes) = self.read_file_bytes(path)?;
        // `decode_utf8_bytes` strips a BOM. Keep the original UTF-8, including a
        // BOM, so a later save does not rewrite the file's encoding.
        match super::text_codec::decode_utf8_bytes(&bytes) {
            Ok(_) => match String::from_utf8(bytes) {
                Ok(content) => Ok((rel, content)),
                Err(_) => Err(WorkspaceError::NotUtf8),
            },
            Err(super::text_codec::Utf8DecodeError::Utf16) => Err(WorkspaceError::Utf16),
            Err(super::text_codec::Utf8DecodeError::Binary) => Err(WorkspaceError::Binary),
            Err(super::text_codec::Utf8DecodeError::NotUtf8) => Err(WorkspaceError::NotUtf8),
        }
    }

    /// Bytes for an in-panel preview (image, PDF, audio, video). Separate from
    /// [`Self::read_file_bytes`], which stays capped at the text read limit.
    pub fn read_preview_bytes(
        &self,
        path: &str,
    ) -> Result<(String, &'static str, Vec<u8>), WorkspaceError> {
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
        if meta.len() > MAX_PREVIEW_BYTES {
            return Err(WorkspaceError::PreviewTooLarge);
        }
        let bytes = std::fs::read(&abs)?;
        let rel = sandbox.rel_path(&abs)?;
        let mime = preview_mime(&rel);
        Ok((rel, mime, bytes))
    }

    pub fn sqlite_preview(
        &self,
        path: &str,
        table: &str,
        offset: u64,
    ) -> Result<(String, super::sqlite_preview::SqlitePreview), WorkspaceError> {
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
        let rel = sandbox.rel_path(&abs)?;
        let page = super::sqlite_preview::preview(&abs, table, offset)?;
        Ok((rel, page))
    }

    pub fn open_with_default_app(&self, path: &str) -> Result<(), WorkspaceError> {
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
        match super::open_host::open_with_default_app(&abs) {
            Ok(()) => Ok(()),
            Err(super::open_host::OpenHostError::Unsupported) => {
                Err(WorkspaceError::OpenUnsupported)
            }
            Err(super::open_host::OpenHostError::Failed(message)) => {
                Err(WorkspaceError::OpenFailed(message))
            }
        }
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
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Exclusive create (no exists()-then-write TOCTOU). Concurrent creators:
        // at most one Ok; losers get AlreadyExists without clobbering.
        match super::file_ops::atomic_create(&abs, content) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(WorkspaceError::AlreadyExists(
                    sandbox.rel_path(&abs).unwrap_or_else(|_| path.into()),
                ));
            }
            Err(error) => return Err(error.into()),
        }
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
                // Same-device replace; on EXDEV fall through to copy+replace.
                match super::file_ops::replace_file(&from_abs, &to_abs) {
                    Ok(()) => {}
                    Err(error) if is_cross_device(&error) => {
                        move_via_copy(sandbox.root(), &from_abs, &to_abs)?;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) if is_cross_device(&error) => {
                move_via_copy(sandbox.root(), &from_abs, &to_abs)?;
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
        let root = sandbox.root();

        // Top-level symlink: lexical path (do not Sandbox::resolve — it follows).
        if let Some((from_abs, from_rel)) = lexical_relative_path(root, from)? {
            if path_is_symlink(&from_abs) {
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
                refuse_symlink_node_overwrite(&to_abs, overwrite, &to_rel)?;
                if let Some(parent) = to_abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                preserve_symlink_at(root, &from_abs, &to_abs)?;
                self.emit_changed(vec![to_rel.clone()], "created");
                return Ok(to_rel);
            }
        }

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
        copy_recursive(root, &from_abs, &to_abs)?;
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
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let rel = sandbox.rel_path(&abs)?;
        let kind = if overwrite {
            let existed = abs.exists();
            super::file_ops::atomic_write_bytes(&abs, bytes)?;
            if existed { "modified" } else { "created" }
        } else {
            // Exclusive create — same TOCTOU fix as create_file (ISSUE-037).
            match super::file_ops::atomic_create_bytes(&abs, bytes) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(WorkspaceError::AlreadyExists(
                        sandbox.rel_path(&abs).unwrap_or_else(|_| path.into()),
                    ));
                }
                Err(error) => return Err(error.into()),
            }
            "created"
        };
        self.emit_changed(vec![rel.clone()], kind);
        Ok(rel)
    }
}

/// Lexical workspace-relative path without following symlinks.
/// Returns `None` for absolute / unclassifiable inputs so callers can fall
/// through to [`Sandbox::resolve`].
fn lexical_relative_path(
    root: &std::path::Path,
    from: &str,
) -> Result<Option<(std::path::PathBuf, String)>, WorkspaceError> {
    use super::agent_path::{PathCandidate, classify_agent_path};
    match classify_agent_path(from) {
        Ok(PathCandidate::Relative(rel)) => {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            Ok(Some((root.join(rel), rel_str)))
        }
        Ok(PathCandidate::Absolute(_)) => Ok(None),
        Err(super::agent_path::ClassifyError::Empty) => Err(WorkspaceError::Sandbox(
            SandboxError::Invalid("path must be non-empty".into()),
        )),
        Err(super::agent_path::ClassifyError::RelativeTraversal)
        | Err(super::agent_path::ClassifyError::UnixStyleOnWindows { .. }) => {
            Err(WorkspaceError::Sandbox(SandboxError::Escape))
        }
    }
}

fn path_is_symlink(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
}

fn resolve_link_target_path(
    link: &std::path::Path,
    target: &std::path::Path,
) -> std::path::PathBuf {
    if target.is_absolute() {
        target.to_path_buf()
    } else {
        match link.parent() {
            Some(parent) => parent.join(target),
            None => target.to_path_buf(),
        }
    }
}

/// Sandbox gate for a symlink's referent (does not copy content).
fn ensure_link_target_in_workspace(
    root: &std::path::Path,
    link_abs: &std::path::Path,
    target_text: &std::path::Path,
) -> Result<(), WorkspaceError> {
    use crate::config::path::{canon_abs, canon_abs_lossy, canon_join_nonexistent, is_under};
    let root = canon_abs_lossy(root);
    let resolved = resolve_link_target_path(link_abs, target_text);
    if resolved.exists() {
        let canon = canon_abs(&resolved)?;
        if !is_under(&canon, &root) {
            return Err(WorkspaceError::Sandbox(SandboxError::Escape));
        }
        return Ok(());
    }
    match canon_join_nonexistent(&root, &resolved) {
        Ok(joined) if is_under(&joined, &root) => Ok(()),
        Ok(_) => Err(WorkspaceError::Sandbox(SandboxError::Escape)),
        Err(_) => {
            let lossy = canon_abs_lossy(&resolved);
            if is_under(&lossy, &root) {
                Ok(())
            } else {
                Err(WorkspaceError::Sandbox(SandboxError::Escape))
            }
        }
    }
}

fn create_symlink(target_text: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target_text, dest)
    }
    #[cfg(windows)]
    {
        let looks_dir = resolve_link_target_path(dest, target_text).is_dir();
        if looks_dir {
            std::os::windows::fs::symlink_dir(target_text, dest)
        } else {
            std::os::windows::fs::symlink_file(target_text, dest)
        }
    }
}

/// Preserve a symlink node at `dest_abs` (readlink → sandbox gate → recreate).
fn preserve_symlink_at(
    root: &std::path::Path,
    link_abs: &std::path::Path,
    dest_abs: &std::path::Path,
) -> Result<(), WorkspaceError> {
    let target_text = std::fs::read_link(link_abs)?;
    ensure_link_target_in_workspace(root, link_abs, &target_text)?;
    // Overwrite path: replace file/symlink nodes; never clobber a real directory.
    if let Ok(meta) = std::fs::symlink_metadata(dest_abs) {
        if meta.file_type().is_symlink() || meta.is_file() {
            std::fs::remove_file(dest_abs)?;
        } else if meta.is_dir() {
            return Err(WorkspaceError::InvalidMove(
                "refusing to overwrite a directory".into(),
            ));
        }
    }
    if let Some(parent) = dest_abs.parent() {
        std::fs::create_dir_all(parent)?;
    }
    create_symlink(&target_text, dest_abs)?;
    Ok(())
}

/// Symlink (and plain-file) overwrite gate for top-level preserve copies.
fn refuse_symlink_node_overwrite(
    to_abs: &std::path::Path,
    overwrite: bool,
    to_rel: &str,
) -> Result<(), WorkspaceError> {
    let meta = match std::fs::symlink_metadata(to_abs) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if meta.is_dir() && !meta.file_type().is_symlink() {
        return Err(WorkspaceError::InvalidMove(
            "refusing to overwrite a directory".into(),
        ));
    }
    if !overwrite {
        return Err(WorkspaceError::AlreadyExists(to_rel.into()));
    }
    Ok(())
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

/// Copy then delete source. On copy failure, remove an incomplete destination
/// only when it did not already exist — never delete a pre-existing target
/// (ISSUE-038 / WP-040).
fn move_via_copy(
    root: &std::path::Path,
    from_abs: &std::path::Path,
    to_abs: &std::path::Path,
) -> Result<(), WorkspaceError> {
    let dest_existed = to_abs.exists() || path_is_symlink(to_abs);
    if let Err(error) = copy_recursive(root, from_abs, to_abs) {
        if !dest_existed {
            cleanup_incomplete_dest(to_abs);
        }
        return Err(error);
    }
    // Remove source node without following symlinks.
    match std::fs::symlink_metadata(from_abs) {
        Ok(meta) if meta.file_type().is_symlink() || meta.is_file() => {
            std::fs::remove_file(from_abs)?;
        }
        Ok(meta) if meta.is_dir() => {
            std::fs::remove_dir_all(from_abs)?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
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

fn copy_recursive(
    root: &std::path::Path,
    src: &std::path::Path,
    dst: &std::path::Path,
) -> Result<(), WorkspaceError> {
    let meta = std::fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        return preserve_symlink_at(root, src, dst);
    }
    if meta.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(root, &entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else if dst.exists() {
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
        persist.map_err(WorkspaceError::from)
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // WP-141: rescan requests reach every subscriber (router → full reconcile).
    #[tokio::test]
    async fn rescan_signal_reaches_subscribers() {
        let dir = tempfile::TempDir::new().unwrap();
        let service = WorkspaceService::new(dir.path().to_path_buf()).unwrap();
        let mut rx = service.subscribe_rescan();
        service.rescan_sender().send(()).unwrap();
        assert!(rx.recv().await.is_ok());
    }

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
    fn preserves_symlink_top_level() {
        let dir = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(dir.path().to_path_buf()).unwrap();
        std::fs::write(dir.path().join("real.txt"), "payload").unwrap();
        // Relative link text — must be preserved verbatim, not dereferenced.
        std::os::unix::fs::symlink("real.txt", dir.path().join("link.txt")).unwrap();
        let out = service.copy_path("link.txt", "out.txt", false).unwrap();
        assert_eq!(out, "out.txt");
        let out_meta = std::fs::symlink_metadata(dir.path().join("out.txt")).unwrap();
        assert!(
            out_meta.file_type().is_symlink(),
            "dest must remain a symlink, not a copied file"
        );
        assert_eq!(
            std::fs::read_link(dir.path().join("out.txt")).unwrap(),
            std::fs::read_link(dir.path().join("link.txt")).unwrap()
        );
        // Following the new link still reaches the original payload.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("out.txt")).unwrap(),
            "payload"
        );
        // Source link node unchanged.
        assert!(
            std::fs::symlink_metadata(dir.path().join("link.txt"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn preserves_symlink_nested() {
        let dir = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(dir.path().to_path_buf()).unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/real.txt"), "inner").unwrap();
        std::os::unix::fs::symlink("real.txt", dir.path().join("sub/link.txt")).unwrap();
        service.copy_path("sub", "sub2", false).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("sub2/real.txt")).unwrap(),
            "inner"
        );
        let nested = std::fs::symlink_metadata(dir.path().join("sub2/link.txt")).unwrap();
        assert!(nested.file_type().is_symlink());
        assert_eq!(
            std::fs::read_link(dir.path().join("sub2/link.txt")).unwrap(),
            std::fs::read_link(dir.path().join("sub/link.txt")).unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("sub2/link.txt")).unwrap(),
            "inner"
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_copy_escaping_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let service = WorkspaceService::new(dir.path().to_path_buf()).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "nope").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            dir.path().join("evil.txt"),
        )
        .unwrap();
        assert!(matches!(
            service.copy_path("evil.txt", "out.txt", false),
            Err(WorkspaceError::Sandbox(SandboxError::Escape))
        ));
        assert!(!dir.path().join("out.txt").exists());
        assert!(
            std::fs::symlink_metadata(dir.path().join("out.txt")).is_err(),
            "must not leave a half-written escape link"
        );
    }
}
