//! Binary sniff shared by text search, indexing, and read tool.
//!
//! Heuristic aligned with ripgrep: treat as binary when the first 8 KiB
//! contains a NUL byte (or the file cannot be read).

use std::io::Read;
use std::path::Path;

const BINARY_CHECK_SIZE: usize = 8192;

/// True when the first 8 KiB contains a NUL byte (or the file cannot be read).
pub fn looks_binary(abs: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(abs) else {
        return true;
    };
    let mut buf = [0u8; BINARY_CHECK_SIZE];
    let Ok(n) = file.read(&mut buf) else {
        return true;
    };
    buf[..n].contains(&0)
}
