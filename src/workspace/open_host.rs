//! Hand a sandbox-resolved path to the OS: open it with its default app, or
//! reveal it in the system file manager.
//! Linux and macOS intentionally do nothing here — the file panel only
//! offers these actions when the server itself is Windows.

use std::path::Path;

pub enum OpenHostError {
    /// Only constructed off Windows. Kept in the type so the caller can map it.
    #[allow(dead_code)]
    Unsupported,
    Failed(String),
}

pub fn open_with_default_app(path: &Path) -> Result<(), OpenHostError> {
    #[cfg(windows)]
    {
        shell_execute(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(OpenHostError::Unsupported)
    }
}

/// Show a file or folder selected in the OS file manager.
pub fn reveal_in_file_manager(path: &Path) -> Result<(), OpenHostError> {
    #[cfg(windows)]
    {
        explorer_select(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(OpenHostError::Unsupported)
    }
}

#[cfg(windows)]
fn explorer_select(path: &Path) -> Result<(), OpenHostError> {
    use std::os::windows::process::CommandExt;

    // Explorer reports a non-zero exit code even when it worked, and the
    // process stays resident, so a successful spawn is the only signal we can
    // read. The switch goes in raw so the path keeps its own quotes instead of
    // being re-escaped into something Explorer cannot parse.
    let mut cmd = std::process::Command::new("explorer.exe");
    cmd.raw_arg(format!("/select,\"{}\"", path.display()));
    cmd.spawn()
        .map(|_| ())
        .map_err(|err| OpenHostError::Failed(format!("could not open the file manager: {err}")))
}

#[cfg(windows)]
fn shell_execute(path: &Path) -> Result<(), OpenHostError> {
    use std::os::windows::ffi::OsStrExt;

    // Declared locally so we don't enable the large Win32_UI_WindowsAndMessaging
    // feature just for this call. Success is any return greater than 32.
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut std::ffi::c_void,
            lp_operation: *const u16,
            lp_file: *const u16,
            lp_parameters: *const u16,
            lp_directory: *const u16,
            n_show_cmd: i32,
        ) -> *mut std::ffi::c_void;
    }

    let file: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    // SW_SHOWNORMAL
    const SHOW: i32 = 1;
    let rc = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SHOW,
        )
    };
    let code = rc as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(OpenHostError::Failed(format!(
            "could not open with the default app ({code})"
        )))
    }
}
