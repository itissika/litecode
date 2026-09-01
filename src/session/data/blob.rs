//! Content-addressed blob store for spilled transcript bodies and media.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::types::{LitecodeError, Result};

pub fn blob_root(data_root: &Path) -> PathBuf {
    data_root.join("blobs")
}

pub fn put_bytes(data_root: &Path, bytes: &[u8]) -> Result<String> {
    let digest = Sha256::digest(bytes);
    let hex = hex_encode(&digest);
    let rel = rel_path_for(&hex);
    let dest = data_root.join(&rel);
    if dest.exists() {
        return Ok(hex);
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension("tmp");
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, &dest)?;
    Ok(hex)
}

pub fn put_text(data_root: &Path, text: &str) -> Result<String> {
    put_bytes(data_root, text.as_bytes())
}

pub fn read_bytes(data_root: &Path, blob_id: &str) -> Result<Vec<u8>> {
    let path = data_root.join(rel_path_for(blob_id));
    fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            LitecodeError::MediaBlobMissing(blob_id.to_string())
        } else {
            LitecodeError::SessionStorage(format!("read blob {blob_id}: {e}"))
        }
    })
}

pub fn rel_path_for(blob_id: &str) -> PathBuf {
    let prefix = blob_id.get(..2).unwrap_or("00");
    PathBuf::from("blobs").join(prefix).join(blob_id)
}

pub fn gc_unreferenced(data_root: &Path, live_ids: &[String]) -> Result<usize> {
    let root = blob_root(data_root);
    if !root.exists() {
        return Ok(0);
    }
    let live: std::collections::HashSet<&str> = live_ids.iter().map(String::as_str).collect();
    let mut removed = 0usize;
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        for file in fs::read_dir(entry.path())? {
            let file = file?;
            let name = file.file_name().to_string_lossy().into_owned();
            if name.ends_with(".tmp") {
                let _ = fs::remove_file(file.path());
                continue;
            }
            if !live.contains(name.as_str()) {
                fs::remove_file(file.path())?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_is_content_addressed_and_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let a = put_text(dir.path(), "hello blob").unwrap();
        let b = put_text(dir.path(), "hello blob").unwrap();
        assert_eq!(a, b);
        let body = read_bytes(dir.path(), &a).unwrap();
        assert_eq!(body, b"hello blob");
    }
}
