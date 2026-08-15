//! Bounded stdout capture for agent foreground bash: tee to a workspace file,
//! keep only a head+tail window in memory.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use super::ansi::strip_ansi;
use super::error::{TerminalError, TerminalResult};

/// Bytes kept at the start of the model-facing window.
pub const INLINE_HEAD: usize = 8 * 1024;
/// Bytes kept at the end of the model-facing window.
pub const INLINE_TAIL: usize = 8 * 1024;
/// Grow the in-memory buffer until this size, then freeze into head+tail.
pub const INLINE_FULL: usize = INLINE_HEAD + INLINE_TAIL;
/// Stop writing the on-disk log after this many bytes.
pub const FILE_MAX: usize = 8 * 1024 * 1024;

pub struct BoundedTee {
    pub path: PathBuf,
    file: File,
    file_written: usize,
    pub total_bytes: usize,
    pub truncated_on_disk: bool,
    frozen: bool,
    buf: String,
    head: String,
    tail: String,
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
            frozen: false,
            buf: String::new(),
            head: String::new(),
            tail: String::new(),
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
        if !self.frozen {
            self.buf.push_str(chunk);
            if self.buf.len() > INLINE_FULL {
                self.freeze();
            }
        } else {
            self.tail.push_str(chunk);
            trim_to_tail(&mut self.tail);
        }
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

    fn freeze(&mut self) {
        self.frozen = true;
        let (head, rest) = split_at_bytes(&self.buf, INLINE_HEAD);
        self.head = head.to_string();
        self.tail = rest.to_string();
        self.buf.clear();
        trim_to_tail(&mut self.tail);
    }

    pub fn flush_file(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }

    pub fn snapshot_capture(&self) -> TeeCapture {
        if self.frozen {
            TeeCapture {
                path: self.path.clone(),
                head: self.head.clone(),
                tail: self.tail.clone(),
                frozen: true,
                total_bytes: self.total_bytes,
                truncated_on_disk: self.truncated_on_disk,
            }
        } else {
            TeeCapture {
                path: self.path.clone(),
                head: self.buf.clone(),
                tail: String::new(),
                frozen: false,
                total_bytes: self.total_bytes,
                truncated_on_disk: self.truncated_on_disk,
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TeeCapture {
    pub path: PathBuf,
    pub head: String,
    pub tail: String,
    pub frozen: bool,
    pub total_bytes: usize,
    pub truncated_on_disk: bool,
}

impl TeeCapture {
    pub fn snapshot(&self) -> String {
        if self.frozen {
            format!("{}{}", self.head, self.tail)
        } else {
            self.head.clone()
        }
    }
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

fn trim_to_tail(tail: &mut String) {
    if tail.len() <= INLINE_TAIL {
        return;
    }
    let start = {
        let mut i = tail.len() - INLINE_TAIL;
        while i < tail.len() && !tail.is_char_boundary(i) {
            i += 1;
        }
        i
    };
    tail.replace_range(..start, "");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tee_in(dir: &std::path::Path) -> BoundedTee {
        BoundedTee::create(dir.join("t.output")).expect("tee")
    }

    #[test]
    fn small_output_stays_unfrozen() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = tee_in(dir.path());
        t.push_clean("hello\n");
        t.push_clean("world\n");
        let _ = t.flush_file();
        let cap = t.snapshot_capture();
        assert!(!cap.frozen);
        assert_eq!(cap.head, "hello\nworld\n");
        assert!(cap.tail.is_empty());
        let on_disk = std::fs::read_to_string(&cap.path).unwrap();
        assert_eq!(on_disk, "hello\nworld\n");
    }

    #[test]
    fn large_output_freezes_head_and_tail() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = tee_in(dir.path());
        let chunk = "a".repeat(INLINE_FULL + 100);
        t.push_clean(&chunk);
        let _ = t.flush_file();
        let cap = t.snapshot_capture();
        assert!(cap.frozen);
        assert!(cap.head.len() <= INLINE_HEAD);
        assert!(cap.tail.len() <= INLINE_TAIL + 4);
        assert_eq!(cap.total_bytes, chunk.len());
        let on_disk = std::fs::read_to_string(&cap.path).unwrap();
        assert_eq!(on_disk.len(), chunk.len());
        assert!(on_disk.starts_with(&cap.head));
        assert!(on_disk.ends_with(&cap.tail));
    }

    #[test]
    fn freeze_keeps_utf8_char_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = tee_in(dir.path());
        let chunk = "你".repeat(INLINE_FULL / 3 + 80);
        t.push_clean(&chunk);
        let cap = t.snapshot_capture();
        assert!(cap.frozen);
        assert!(std::str::from_utf8(cap.head.as_bytes()).is_ok());
        assert!(std::str::from_utf8(cap.tail.as_bytes()).is_ok());
        let on_disk = std::fs::read_to_string(&cap.path).unwrap();
        assert!(on_disk.starts_with(&cap.head));
        assert!(on_disk.ends_with(&cap.tail));
    }

    #[test]
    fn file_stops_at_max_and_flags_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = tee_in(dir.path());
        t.push_clean(&"x".repeat(FILE_MAX + 50));
        let _ = t.flush_file();
        let cap = t.snapshot_capture();
        assert!(cap.truncated_on_disk);
        assert_eq!(cap.total_bytes, FILE_MAX + 50);
        let on_disk = std::fs::read(&cap.path).unwrap();
        assert_eq!(on_disk.len(), FILE_MAX);
    }
}
