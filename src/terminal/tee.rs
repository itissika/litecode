//! Bounded stdout capture for agent commands: tee clean UTF-8 output to a workspace
//! `.litecode/run/` file up to `FILE_MAX`, and read tail/delta slices back cleanly.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::ansi::strip_ansi;
use super::error::{TerminalError, TerminalResult};

/// Stop writing the on-disk log after this many bytes (8 MiB).
pub const FILE_MAX: usize = 8 * 1024 * 1024;

pub struct BoundedTee {
    pub path: PathBuf,
    file: File,
    file_written: usize,
    pub total_bytes: usize,
    pub truncated_on_disk: bool,
}

impl BoundedTee {
    pub fn create(path: PathBuf) -> TerminalResult<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| TerminalError::Io(e.to_string()))?;
        Ok(Self {
            path,
            file,
            file_written: 0,
            total_bytes: 0,
            truncated_on_disk: false,
        })
    }

    pub fn push_raw(&mut self, raw: &str) {
        if raw.is_empty() {
            return;
        }
        self.push_clean(&strip_ansi(raw));
    }

    fn push_clean(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        self.total_bytes += chunk.len();
        self.write_file(chunk);
    }

    fn write_file(&mut self, chunk: &str) {
        if self.truncated_on_disk {
            return;
        }
        let room = FILE_MAX.saturating_sub(self.file_written);
        if room == 0 {
            self.truncated_on_disk = true;
            return;
        }
        let (keep, rest) = split_at_bytes(chunk, room);
        if !keep.is_empty() {
            let _ = self.file.write_all(keep.as_bytes());
            let _ = self.file.flush();
            self.file_written += keep.len();
        }
        if !rest.is_empty() {
            self.truncated_on_disk = true;
        }
    }

    pub fn flush_file(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Read up to `max_bytes` from the tail of `path` starting no earlier than
/// `start_offset`. Skips leading UTF-8 continuation bytes after a non-zero
/// seek so multi-byte characters split by a byte offset never produce `\u{FFFD}`.
/// Returns `(text, new_cursor, new_bytes_since_cursor, truncated)`.
pub(crate) fn read_tail_utf8_clean(
    path: &Path,
    start_offset: u64,
    max_bytes: usize,
) -> (String, u64, usize, bool) {
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if len <= start_offset {
        return (String::new(), start_offset, 0, false);
    }
    let total_new = usize::try_from(len - start_offset).unwrap_or(usize::MAX);
    if max_bytes == 0 {
        return (String::new(), len, total_new, true);
    }
    let read_start = len.saturating_sub(max_bytes as u64).max(start_offset);
    let mut bytes = Vec::new();
    if let Ok(mut file) = File::open(path) {
        if file.seek(SeekFrom::Start(read_start)).is_ok() {
            let _ = file.read_to_end(&mut bytes);
        }
    }
    let mut start_idx = 0usize;
    if read_start > 0 {
        while start_idx < bytes.len()
            && start_idx < 3
            && (bytes[start_idx] & 0b1100_0000) == 0b1000_0000
        {
            start_idx += 1;
        }
    }
    let slice = &bytes[start_idx..];
    let text = match std::str::from_utf8(slice) {
        Ok(s) => s.to_string(),
        Err(e) if e.error_len().is_none() => std::str::from_utf8(&slice[..e.valid_up_to()])
            .unwrap_or_default()
            .to_string(),
        Err(_) => String::from_utf8_lossy(slice).into_owned(),
    };
    let truncated = (len - start_offset) > max_bytes as u64;
    (text, len, total_new, truncated)
}

fn split_at_bytes(s: &str, max: usize) -> (&str, &str) {
    if s.len() <= max {
        return (s, "");
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    s.split_at(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tee_in(dir: &std::path::Path) -> BoundedTee {
        BoundedTee::create(dir.join("t.output")).expect("tee")
    }

    #[test]
    fn file_stops_at_max_and_flags_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = tee_in(dir.path());
        t.push_clean(&"x".repeat(FILE_MAX + 50));
        let _ = t.flush_file();
        assert!(t.truncated_on_disk);
        assert_eq!(t.total_bytes, FILE_MAX + 50);
        let on_disk = std::fs::read(&t.path).unwrap();
        assert_eq!(on_disk.len(), FILE_MAX);
    }

    #[test]
    fn read_tail_utf8_clean_handles_multibyte_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = tee_in(dir.path());
        let body = format!("HEAD_{}\nTAIL_END", "中".repeat(50));
        t.push_clean(&body);
        t.flush_file().unwrap();

        // Cut size not divisible by 3 so `len - 17` lands mid-character.
        let (tail, cursor, total_new, truncated) = read_tail_utf8_clean(&t.path, 0, 17);
        assert!(truncated);
        assert_eq!(cursor, body.len() as u64);
        assert_eq!(total_new, body.len());
        assert!(!tail.contains('\u{FFFD}'), "replacement char in: {tail:?}");
        assert!(tail.ends_with("TAIL_END"), "{tail:?}");

        let (empty, cursor2, new2, trunc2) = read_tail_utf8_clean(&t.path, cursor, 17);
        assert!(empty.is_empty());
        assert_eq!(cursor2, cursor);
        assert_eq!(new2, 0);
        assert!(!trunc2);
    }
}
