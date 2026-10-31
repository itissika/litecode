//! Pipe output decoding for agent jobs (WP-210 F).
//!
//! Output is UTF-8 until a chunk proves otherwise. A multi-byte character split
//! across reads is held back, not mangled. Once invalid UTF-8 shows up, the
//! stream switches to the platform's ANSI code page (Windows `GetACP`, e.g.
//! CP936 → GBK) for the rest of the stream. Without a known code page the
//! bytes are decoded lossily, as before.

use encoding_rs::{Decoder, Encoding};

pub struct StreamDecoder {
    fallback: Option<&'static Encoding>,
    pending: Vec<u8>,
    legacy: Option<Decoder>,
}

impl StreamDecoder {
    /// Decoder whose fallback is the platform ANSI code page.
    pub fn platform() -> Self {
        Self::with_fallback(platform_fallback())
    }

    pub fn with_fallback(fallback: Option<&'static Encoding>) -> Self {
        Self {
            fallback,
            pending: Vec::new(),
            legacy: None,
        }
    }

    /// Decode one chunk. Bytes of an incomplete trailing character wait for
    /// the next chunk (or [`Self::finish`]).
    pub fn push(&mut self, bytes: &[u8]) -> String {
        if let Some(decoder) = self.legacy.as_mut() {
            return decode_legacy(decoder, bytes, false);
        }
        self.pending.extend_from_slice(bytes);
        match std::str::from_utf8(&self.pending) {
            Ok(text) => {
                let text = text.to_string();
                self.pending.clear();
                text
            }
            Err(error) if error.error_len().is_none() => {
                // Only an incomplete character at the end: keep it.
                let valid = error.valid_up_to();
                let text = String::from_utf8_lossy(&self.pending[..valid]).into_owned();
                self.pending.drain(..valid);
                text
            }
            Err(_) => match self.fallback {
                Some(encoding) => {
                    let mut decoder = encoding.new_decoder_without_bom_handling();
                    let pending = std::mem::take(&mut self.pending);
                    let text = decode_legacy(&mut decoder, &pending, false);
                    self.legacy = Some(decoder);
                    text
                }
                None => {
                    let pending = std::mem::take(&mut self.pending);
                    String::from_utf8_lossy(&pending).into_owned()
                }
            },
        }
    }

    /// Flush anything still held back.
    pub fn finish(&mut self) -> String {
        if let Some(decoder) = self.legacy.as_mut() {
            return decode_legacy(decoder, &[], true);
        }
        let pending = std::mem::take(&mut self.pending);
        if pending.is_empty() {
            return String::new();
        }
        match (std::str::from_utf8(&pending), self.fallback) {
            (Ok(text), _) => text.to_string(),
            (Err(_), Some(encoding)) => encoding
                .decode_without_bom_handling(&pending)
                .0
                .into_owned(),
            (Err(_), None) => String::from_utf8_lossy(&pending).into_owned(),
        }
    }
}

fn decode_legacy(decoder: &mut Decoder, bytes: &[u8], last: bool) -> String {
    let capacity = decoder
        .max_utf8_buffer_length(bytes.len())
        .unwrap_or(bytes.len() * 3 + 16);
    let mut out = String::with_capacity(capacity);
    let _ = decoder.decode_to_string(bytes, &mut out, last);
    out
}

/// Encoding for a Windows ANSI code page number.
pub fn encoding_for_code_page(code_page: u32) -> Option<&'static Encoding> {
    let encoding = match code_page {
        936 => encoding_rs::GBK,
        54936 => encoding_rs::GB18030,
        950 => encoding_rs::BIG5,
        932 => encoding_rs::SHIFT_JIS,
        949 => encoding_rs::EUC_KR,
        1250 => encoding_rs::WINDOWS_1250,
        1251 => encoding_rs::WINDOWS_1251,
        1252 => encoding_rs::WINDOWS_1252,
        1253 => encoding_rs::WINDOWS_1253,
        1254 => encoding_rs::WINDOWS_1254,
        1255 => encoding_rs::WINDOWS_1255,
        1256 => encoding_rs::WINDOWS_1256,
        1257 => encoding_rs::WINDOWS_1257,
        1258 => encoding_rs::WINDOWS_1258,
        874 => encoding_rs::WINDOWS_874,
        _ => return None,
    };
    Some(encoding)
}

#[cfg(windows)]
fn platform_fallback() -> Option<&'static Encoding> {
    let code_page = unsafe { windows_sys::Win32::Globalization::GetACP() };
    encoding_for_code_page(code_page)
}

#[cfg(not(windows))]
fn platform_fallback() -> Option<&'static Encoding> {
    None
}

/// Shell that `run` hands commands to, as shown to the agent.
pub fn shell_identity() -> String {
    #[cfg(windows)]
    {
        if crate::config::git_install::find_git_bash().is_some() {
            "bash (MINGW64)".to_string()
        } else {
            "powershell".to_string()
        }
    }
    #[cfg(not(windows))]
    {
        let spec = super::shell::shell_command("");
        super::shell::shell_label(&spec)
    }
}

/// Git Bash (MSYS) rewrites arguments that look like POSIX paths, so `/c`
/// reaches `cmd` as `C:/`. Returns a hint when such an argument is present.
pub fn msys_path_hint(command: &str) -> Option<String> {
    let suspicious: Vec<&str> = command
        .split_whitespace()
        .filter(|token| {
            let bytes = token.as_bytes();
            bytes.len() == 2 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic()
        })
        .collect();
    if suspicious.is_empty() {
        return None;
    }
    Some(format!(
        "Git Bash (MSYS) converts POSIX-looking arguments such as {} into Windows paths; write them as //x (e.g. `cmd //c`) or set MSYS_NO_PATHCONV=1 for this command",
        suspicious.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_output_is_utf8_or_transcoded_from_the_ansi_code_page() {
        // "中文" in UTF-8, split inside the first character.
        let utf8 = "中文 ok".as_bytes();
        let mut decoder = StreamDecoder::with_fallback(encoding_for_code_page(936));
        let mut text = decoder.push(&utf8[..1]);
        text.push_str(&decoder.push(&utf8[1..]));
        text.push_str(&decoder.finish());
        assert_eq!(text, "中文 ok");

        // "中文" in CP936 (GBK) bytes, split across chunks.
        let gbk = [0xD6, 0xD0, 0xCE, 0xC4, b' ', b'o', b'k'];
        let mut decoder = StreamDecoder::with_fallback(encoding_for_code_page(936));
        let mut text = decoder.push(&gbk[..3]);
        text.push_str(&decoder.push(&gbk[3..]));
        text.push_str(&decoder.finish());
        assert_eq!(text, "中文 ok");

        // No code page known: lossy, never panics.
        let mut decoder = StreamDecoder::with_fallback(None);
        let text = decoder.push(&gbk);
        assert!(text.contains(" ok"), "{text}");
    }
}
