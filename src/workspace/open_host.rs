//! Open a sandbox-resolved file with the OS default application.
//! Linux and macOS intentionally do nothing here — the file panel only
//! offers this action when the server itself is Windows.

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
