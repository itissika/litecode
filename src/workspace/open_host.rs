//! Hand a sandbox-resolved path to the system file manager.
//! Linux and macOS intentionally do nothing here — the file panel only
//! offers these actions when the server itself is Windows.

use std::path::Path;

pub enum OpenHostError {
    /// Only constructed off Windows. Kept in the type so the caller can map it.
    #[allow(dead_code)]
    Unsupported,
    Failed(String),
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
