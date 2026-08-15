//! LSP binary installation directory resolution.

use std::path::{Path, PathBuf};

use crate::types::{LitecodeError, Result};

pub fn lsp_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("LITECODE_LSP_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Ok(p);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().unwrap_or(Path::new(".")).to_path_buf();

        loop {
            let candidate = dir.join("lsp");
            if candidate.is_dir() {
                return Ok(crate::config::path::os_probe_abs(&candidate)
                    .unwrap_or_else(|_| crate::config::path::canon_abs_lossy(&candidate)));
            }
            match dir.parent() {
                Some(parent) => dir = parent.to_path_buf(),
                None => break,
            }
        }
    }

    let home = dirs_next().ok_or_else(|| {
        LitecodeError::Config("cannot determine home directory for lsp files".into())
    })?;
    let p = home.join(".litecode").join("lsp");
    std::fs::create_dir_all(&p)
        .map_err(|e| LitecodeError::Config(format!("create lsp dir {}: {e}", p.display())))?;
    Ok(p)
}

fn dirs_next() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .or_else(|| {
            std::env::var("USERPROFILE").ok().or_else(|| {
                let home = std::env::var("HOMEDRIVE").unwrap_or_default();
                let path = std::env::var("HOMEPATH").unwrap_or_default();
                let combined = format!("{home}{path}");
                if combined.is_empty() {
                    None
                } else {
                    Some(combined)
                }
            })
        })
        .map(PathBuf::from)
}
