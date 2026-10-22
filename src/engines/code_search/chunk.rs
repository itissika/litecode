//! Line-based chunking (60 lines, 12 overlap).
//!
//! AST / tree-sitter chunking was prototyped (Continue / cAST) but failed the
//! hard-v2 `prod-baseline` gate (−5pp R@10); not shipped. Path-prefix embed
//! text is likewise held back with that experiment.

use serde::{Deserialize, Serialize};

use super::{CHUNK_LINES, CHUNK_OVERLAP};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRecord {
    pub id: u64,
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub text: String,
}

impl ChunkRecord {
    pub fn summary(&self) -> String {
        self.text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .chars()
            .take(200)
            .collect()
    }
}

pub fn chunk_file(path: &str, content: &str, next_id: u64) -> (Vec<ChunkRecord>, u64) {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return (Vec::new(), next_id);
    }

    let step = CHUNK_LINES.saturating_sub(CHUNK_OVERLAP).max(1);
    let mut chunks = Vec::new();
    let mut id = next_id;
    let mut start = 0usize;

    while start < lines.len() {
        let end = (start + CHUNK_LINES).min(lines.len());
        let text = lines[start..end].join("\n");
        let start_line = start as u32 + 1;
        let end_line = end as u32;
        chunks.push(ChunkRecord {
            id,
            path: path.to_string(),
            start_line,
            end_line,
            text,
        });
        id += 1;
        if end >= lines.len() {
            break;
        }
        start += step;
    }

    (chunks, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_overlap_by_twenty_lines() {
        let content: String = (1..=150)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (chunks, _) = chunk_file("a.rs", &content, 1);
        assert!(chunks.len() >= 2);
        let first_end = chunks[0].end_line as usize;
        let second_start = chunks[1].start_line as usize;
        assert!(first_end > second_start);
        assert_eq!(first_end - second_start + 1, CHUNK_OVERLAP as usize);
    }
}
