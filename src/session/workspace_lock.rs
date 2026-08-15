//! Cross-process exclusive lock for a workspace `.litecode/` directory.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::types::{LitecodeError, Result};

/// Held for the lifetime of a serve/CLI process that owns the workspace.
#[derive(Debug)]
pub struct WorkspaceLock {
    _file: File,
    path: PathBuf,
}

#[cfg(unix)]
fn try_lock_exclusive(file: &File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;

    let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if ret == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn unlock(file: &File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;

    let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    if ret == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn try_lock_exclusive(file: &File) -> std::io::Result<()> {
    use std::mem::zeroed;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;

    let handle = file.as_raw_handle() as HANDLE;
    let mut overlapped: OVERLAPPED = unsafe { zeroed() };
    let ok = unsafe {
        LockFileEx(
            handle,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn unlock(file: &File) -> std::io::Result<()> {
    use std::mem::zeroed;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
    use windows_sys::Win32::System::IO::OVERLAPPED;

    let handle = file.as_raw_handle() as HANDLE;
    let mut overlapped: OVERLAPPED = unsafe { zeroed() };
    let ok = unsafe { UnlockFileEx(handle, 0, u32::MAX, u32::MAX, &mut overlapped) };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
fn try_lock_exclusive(_file: &File) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn unlock(_file: &File) -> std::io::Result<()> {
    Ok(())
}

fn is_lock_busy(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    matches!(
        err.raw_os_error(),
        Some(11)  // EAGAIN
            | Some(35) // EWOULDBLOCK on some platforms
            | Some(16) // EBUSY
            | Some(33) // ERROR_LOCK_VIOLATION (Windows)
            | Some(32) // ERROR_SHARING_VIOLATION (Windows)
    )
}

impl WorkspaceLock {
    /// Acquire an exclusive lock on `{litecode_dir}/workspace.lock`.
    ///
    /// Fails with a clear error if another process already holds the lock.
    pub fn acquire(litecode_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(litecode_dir)?;
        let path = litecode_dir.join("workspace.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(LitecodeError::Io)?;

        try_lock_exclusive(&file).map_err(|e| {
            if is_lock_busy(&e) {
                LitecodeError::Config(format!(
                    "workspace already open elsewhere (lock busy: {})",
                    path.display()
                ))
            } else {
                LitecodeError::Io(e)
            }
        })?;

        Ok(Self { _file: file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        let _ = unlock(&self._file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails() {
        let dir = tempfile::tempdir().unwrap();
        let litecode = dir.path().join(".litecode");
        let _first = WorkspaceLock::acquire(&litecode).unwrap();
        let err = WorkspaceLock::acquire(&litecode).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("already open") || msg.contains("lock busy"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn lock_moves_to_new_dir_after_drop() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a").join(".litecode");
        let b = dir.path().join("b").join(".litecode");
        let first = WorkspaceLock::acquire(&a).unwrap();
        let second = WorkspaceLock::acquire(&b).unwrap();
        drop(first);
        let _reacquire_a = WorkspaceLock::acquire(&a).unwrap();
        drop(second);
    }
}
