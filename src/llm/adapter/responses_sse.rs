//! Private SSE framing for Responses HTTP streams.
//!
//! Only splits lines / `data:` payloads. Does **not** define stream dialects.

use crate::types::{LitecodeError, Result};

/// Bounded, stateful SSE line reader (1.3).
///
/// Buffers raw bytes across HTTP chunks and only decodes a line once its
/// terminating `\n` has arrived, so a multi-byte UTF-8 character split across
/// chunk boundaries is never corrupted (per-chunk `from_utf8_lossy` used to
/// emit U+FFFD). Strips a leading UTF-8 BOM and a trailing `\r`. A line longer
/// than [`MAX_LINE_BYTES`] is a hard error, keeping memory bounded.
pub(super) struct SseLineReader {
    buf: Vec<u8>,
}

impl SseLineReader {
    pub(super) const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

    pub(super) fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feed a raw chunk; returns the complete `\n`-terminated lines (newline and
    /// trailing `\r` stripped) in order.
    pub(super) fn feed(&mut self, chunk: &[u8]) -> Result<Vec<String>> {
        if self.buf.is_empty() && chunk.starts_with(&[0xEF, 0xBB, 0xBF]) {
            // Leading UTF-8 BOM on the first chunk — drop it once.
            self.buf.extend_from_slice(&chunk[3..]);
        } else {
            self.buf.extend_from_slice(chunk);
        }
        let mut out = Vec::new();
        loop {
            let Some(nl) = self.buf.iter().position(|&b| b == b'\n') else {
                if self.buf.len() > Self::MAX_LINE_BYTES {
                    return Err(LitecodeError::Llm("SSE line exceeds size limit".into()));
                }
                break;
            };
            if nl > Self::MAX_LINE_BYTES {
                return Err(LitecodeError::Llm("SSE line exceeds size limit".into()));
            }
            // drain(..=nl) includes the `\n`; pop it, then strip a trailing `\r`.
            let mut line_bytes: Vec<u8> = self.buf.drain(..=nl).collect();
            line_bytes.pop();
            if line_bytes.last() == Some(&b'\r') {
                line_bytes.pop();
            }
            out.push(String::from_utf8_lossy(&line_bytes).into_owned());
        }
        Ok(out)
    }

    /// Finalize: process the trailing bytes with no terminating newline, if any.
    pub(super) fn finish(self) -> Result<Option<String>> {
        if self.buf.is_empty() {
            return Ok(None);
        }
        if self.buf.len() > Self::MAX_LINE_BYTES {
            return Err(LitecodeError::Llm("SSE line exceeds size limit".into()));
        }
        let line_bytes = self.buf.as_slice();
        let line_bytes = line_bytes.strip_suffix(b"\r").unwrap_or(line_bytes);
        Ok(Some(String::from_utf8_lossy(line_bytes).into_owned()))
    }
}

/// Reject a 2xx response whose `Content-Type` is not an SSE stream.
///
/// A proxy that returns a JSON error body with a 200 status used to be silently
/// consumed as an empty stream ("stream ended without response.completed").
/// Surface the body explicitly instead. Absent header stays lenient (some
/// servers omit it).
pub(super) async fn check_event_stream_content_type(
    resp: reqwest::Response,
) -> Result<reqwest::Response> {
    if let Some(ct) = resp.headers().get(reqwest::header::CONTENT_TYPE) {
        let ct = ct.to_str().unwrap_or_default().to_ascii_lowercase();
        if !ct.contains("text/event-stream") {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LitecodeError::Llm(format!(
                "expected text/event-stream, got Content-Type '{ct}' (HTTP {status}): {text}"
            )));
        }
    }
    Ok(resp)
}

/// Extract a Responses SSE `data:` payload from one line, if any.
///
/// Returns `None` for empty lines, comment lines (`:`), `event:` lines, and
/// the optional `[DONE]` sentinel. Returns `Some(payload)` for `data: …`
/// (payload may be empty).
pub(super) fn sse_data_payload(line: &str) -> Option<&str> {
    let line = line.trim_end_matches('\r');
    if line.is_empty() {
        return None;
    }
    if line.starts_with(':') {
        return None;
    }
    let Some(rest) = line.strip_prefix("data:") else {
        return None;
    };
    let data = rest.strip_prefix(' ').unwrap_or(rest);
    if data == "[DONE]" {
        return None;
    }
    Some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_data_payload() {
        assert_eq!(
            sse_data_payload(r#"data: {"type":"response.completed"}"#),
            Some(r#"{"type":"response.completed"}"#)
        );
        assert_eq!(sse_data_payload("data:{}"), Some("{}"));
    }

    #[test]
    fn skips_noise() {
        assert_eq!(sse_data_payload(""), None);
        assert_eq!(sse_data_payload(": keep-alive"), None);
        assert_eq!(sse_data_payload("event: response.completed"), None);
        assert_eq!(sse_data_payload("data: [DONE]"), None);
    }

    #[test]
    fn multibyte_char_split_across_chunks_is_preserved() {
        // "数据" = E6 95 B0 E6 8D AE — split across three feeds, plus an emoji
        // (F0 9F 98 80) split across two. Per-chunk lossy decode would corrupt
        // these into U+FFFD; the stateful byte buffer must not.
        let mut reader = SseLineReader::new();
        assert_eq!(
            reader.feed(&b"data: \xE6\x95"[..]).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            reader.feed(&b"\xB0\xE6\x8D"[..]).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            reader.feed(&b"\xAE\xF0\x9F"[..]).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            reader.feed(&b"\x98\x80\n"[..]).unwrap(),
            vec!["data: 数据😀"]
        );
    }

    #[test]
    fn leading_bom_is_stripped_once() {
        let mut reader = SseLineReader::new();
        assert_eq!(
            reader.feed(b"\xEF\xBB\xBFdata: {\"a\":1}\n").unwrap(),
            vec![r#"data: {"a":1}"#]
        );
    }

    #[test]
    fn crlf_lines_strip_cr() {
        let mut reader = SseLineReader::new();
        assert_eq!(
            reader.feed(b"data: x\r\ndata: y\r\n").unwrap(),
            vec!["data: x", "data: y"]
        );
    }

    #[test]
    fn trailing_line_without_newline_is_kept_for_finish() {
        let mut reader = SseLineReader::new();
        assert_eq!(reader.feed(b"data: partial").unwrap(), Vec::<String>::new());
        assert_eq!(reader.finish().unwrap(), Some("data: partial".to_string()));
    }

    #[test]
    fn oversized_line_is_a_hard_error() {
        let mut reader = SseLineReader::new();
        let big = vec![b'a'; SseLineReader::MAX_LINE_BYTES + 1];
        assert!(reader.feed(&big).is_err());
    }
}
