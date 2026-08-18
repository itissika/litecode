//! Destructive deletes for the human workspace API.
//!
//! Windows sends files and folders to the Recycle Bin (`FOF_ALLOWUNDO`).
//! Other platforms keep a direct unlink (no trash spec wiring yet).

use std::io;
use std::path::Path;

pub fn remove_path(path: &Path, recursive: bool) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() && !recursive {
        return std::fs::remove_dir(path);
    }
    #[cfg(windows)]
    {
        recycle_windows(path)
    }
    #[cfg(not(windows))]
    {
        if meta.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        }
    }
}

#[cfg(windows)]
fn recycle_windows(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };

    let display = crate::config::path::strip_verbatim(path);
    let mut from: Vec<u16> = display.as_os_str().encode_wide().collect();
    if from.len() >= 260 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is too long for Recycle Bin; refusing a permanent delete",
        ));
    }
    // SHFileOperationW requires a double-null-terminated list of paths.
    from.push(0);
    from.push(0);

    let mut op = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        wFunc: FO_DELETE as u32,
        pFrom: from.as_ptr(),
        pTo: std::ptr::null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };

    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 {
        return Err(io::Error::other(format!(
            "Recycle Bin failed (code {code}); file was not permanently deleted"
        )));
    }
    if op.fAnyOperationsAborted != 0 {
        return Err(io::Error::other(
            "Recycle Bin operation was aborted; file was not permanently deleted",
        ));
    }
    Ok(())
}
