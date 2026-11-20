use std::fs;
use std::path::{Path, PathBuf};

use crate::authority::responses::FunctionCallOutput;
use crate::session::media::{MAX_MEDIA_BLOB_SIZE, write_media_blob};
use crate::tool::trait_::Tool;
use crate::types::{Item, MediaSource, Result, ToolCallResult, ToolOutputPart};
use crate::types::utf8::{prefix_utf8, suffix_utf8};

pub const RUN_SPILL_DIR: &str = ".litecode/run";
pub const BLOB_PREFIX: &str = "[blob:";
pub const DEFAULT_SPILL_THRESHOLD: usize = 32_768;
pub const DIRECT_VIEW_MAX_BYTES: usize = 65_536;

pub fn blob_dir(data_root: &Path) -> PathBuf {
    data_root.join("blobs")
}

pub fn direct_output_limit(tool: &dyn Tool) -> usize {
    tool.max_result_size()
}

pub fn direct_output_max_bytes_from_limit(tokens: usize) -> usize {
    if tokens == usize::MAX {
        usize::MAX
    } else {
        tokens.saturating_mul(4).min(DIRECT_VIEW_MAX_BYTES)
    }
}

/// Byte ceiling for a direct-surface tool result.
pub fn direct_output_max_bytes(tool: &dyn Tool) -> usize {
    direct_output_max_bytes_from_limit(direct_output_limit(tool))
}

fn sanitize_stem(stem: &str) -> String {
    let cleaned: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if cleaned.is_empty() {
        "output".to_string()
    } else {
        cleaned
    }
}

/// Write the complete unclipped output once to `.litecode/run/{stem}_{nonce}.txt`.
pub fn spill_full_text(
    workspace_root: &Path,
    stem: &str,
    full_text: &str,
) -> std::io::Result<String> {
    let dir = workspace_root.join(".litecode").join("run");
    fs::create_dir_all(&dir)?;
    let name = format!("{}_{}.txt", sanitize_stem(stem), crate::terminal::bash_nonce());
    fs::write(dir.join(&name), full_text.as_bytes())?;
    Ok(format!("{RUN_SPILL_DIR}/{name}"))
}

/// Unified omission notice pointing at the complete on-disk output file.
pub fn full_output_notice(total_bytes: usize, full_path: Option<&str>) -> String {
    match full_path {
        Some(path) => format!("… {total_bytes} bytes total; full output: {path}"),
        None => format!("… {total_bytes} bytes total"),
    }
}

/// Cut `text` with a simple 50% head + 50% tail UTF-8-safe split and the
/// single unified notice marker.
pub fn cut_head_tail_with_notice(text: &str, max_bytes: usize, full_path: Option<&str>) -> String {
    cut_head_tail_with_total(text, max_bytes, text.len(), full_path)
}

fn cut_head_tail_with_total(
    text: &str,
    max_bytes: usize,
    total_bytes: usize,
    full_path: Option<&str>,
) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let notice = full_output_notice(total_bytes, full_path);
    if max_bytes <= notice.len() {
        return notice;
    }
    let preview_budget = max_bytes.saturating_sub(notice.len() + 2);
    let head_budget = preview_budget / 2;
    let tail_budget = preview_budget - head_budget;
    let head = prefix_utf8(text, head_budget);
    let tail = suffix_utf8(&text[head.len()..], tail_budget);
    match (head.is_empty(), tail.is_empty()) {
        (false, false) => format!("{head}\n{notice}\n{tail}"),
        (false, true) => format!("{head}\n{notice}"),
        (true, false) => format!("{notice}\n{tail}"),
        (true, true) => notice,
    }
}

/// Unified spill helper: return `text` untouched if it fits `max_bytes`,
/// otherwise persist the full text under `.litecode/run/` and keep 50/50 head+tail.
pub fn cut_and_spill_full(
    text: &str,
    max_bytes: usize,
    workspace_root: Option<&Path>,
    stem: &str,
) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let full_path = workspace_root.and_then(|root| match spill_full_text(root, stem, text) {
        Ok(rel) => Some(rel),
        Err(error) => {
            tracing::warn!(error = %error, stem, "failed to spill full tool output");
            None
        }
    });
    cut_head_tail_with_notice(text, max_bytes, full_path.as_deref())
}

/// Normalize a single Direct-surface tool result: materialize media parts first,
/// then apply the single unified full-output spill + 50/50 cut if over budget.
pub fn finalize_direct_tool_call_result(
    mut result: ToolCallResult,
    data_root: &Path,
    workspace_root: &Path,
    tool_name: &str,
    max_bytes: usize,
    existing_output_file: Option<&str>,
) -> Result<ToolCallResult> {
    result.parts = materialize_media_parts(result.parts, data_root)?;
    if max_bytes < usize::MAX && result.content.len() > max_bytes {
        if let Some(rel_path) = existing_output_file.filter(|p| !p.is_empty()) {
            let total_bytes = fs::metadata(workspace_root.join(rel_path))
                .map(|m| m.len() as usize)
                .unwrap_or(result.content.len());
            result.content =
                cut_head_tail_with_total(&result.content, max_bytes, total_bytes, Some(rel_path));
        } else {
            result.content =
                cut_and_spill_full(&result.content, max_bytes, Some(workspace_root), tool_name);
        }
    }
    Ok(result)
}

pub fn materialize_media_parts(
    parts: Vec<ToolOutputPart>,
    data_root: &Path,
) -> Result<Vec<ToolOutputPart>> {
    parts
        .into_iter()
        .map(|part| {
            let ToolOutputPart::Media { mut artifact } = part else {
                return Ok(part);
            };
            if let MediaSource::Url { url } = &artifact.source
                && let Some(rest) = url.strip_prefix("data:")
                && let Some((meta, b64)) = rest.split_once(";base64,")
            {
                let mime = meta.split(';').next().unwrap_or("").trim();
                if !mime.is_empty() {
                    artifact.mime_type = mime.to_string();
                }
                let decoded = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    b64.trim(),
                )
                .map_err(|e| crate::types::LitecodeError::ToolExecution(format!("invalid base64 media data URL: {e}")))?;
                if decoded.len() as u64 > MAX_MEDIA_BLOB_SIZE {
                    return Err(crate::types::LitecodeError::ToolExecution(format!(
                        "media payload exceeds max size of {MAX_MEDIA_BLOB_SIZE} bytes"
                    )));
                }
                let blob_id = write_media_blob(data_root, &decoded)?;
                artifact.source = MediaSource::BlobRef { blob_id };
            } else if let MediaSource::LocalFile { path } = &artifact.source {
                let meta = fs::metadata(path).map_err(|e| {
                    crate::types::LitecodeError::ToolExecution(format!(
                        "failed to stat tool media '{path}': {e}"
                    ))
                })?;
                if meta.len() > MAX_MEDIA_BLOB_SIZE {
                    return Err(crate::types::LitecodeError::ToolExecution(format!(
                        "tool media '{path}' exceeds the {MAX_MEDIA_BLOB_SIZE} byte limit"
                    )));
                }
                let bytes = fs::read(path).map_err(|e| {
                    crate::types::LitecodeError::ToolExecution(format!(
                        "failed to read tool media '{path}': {e}"
                    ))
                })?;
                if (artifact.width.is_none() || artifact.height.is_none())
                    && let Some((width, height)) = image_dimensions(&bytes, &artifact.mime_type)
                {
                    artifact.width = Some(width);
                    artifact.height = Some(height);
                }
                let blob_id = write_media_blob(data_root, &bytes)?;
                artifact.source = MediaSource::BlobRef { blob_id };
            }
            Ok(ToolOutputPart::Media { artifact })
        })
        .collect()
}

pub(crate) fn image_dimensions(bytes: &[u8], mime_type: &str) -> Option<(u32, u32)> {
    match mime_type {
        "image/png" => (bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n")).then(|| {
            (
                u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
                u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
            )
        }),
        "image/gif" => (bytes.len() >= 10
            && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")))
        .then(|| {
            (
                u16::from_le_bytes(bytes[6..8].try_into().unwrap()) as u32,
                u16::from_le_bytes(bytes[8..10].try_into().unwrap()) as u32,
            )
        }),
        "image/webp" => webp_dimensions(bytes),
        "image/jpeg" | "image/jpg" => jpeg_dimensions(bytes),
        _ => None,
    }
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 30 || !bytes.starts_with(b"RIFF") || &bytes[8..12] != b"WEBP" {
        return None;
    }
    match &bytes[12..16] {
        b"VP8X" if bytes.len() >= 30 => Some((
            1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]),
            1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]),
        )),
        b"VP8L" if bytes.len() >= 25 && bytes[20] == 0x2f => {
            let bits = u32::from_le_bytes(bytes[21..25].try_into().unwrap());
            Some(((bits & 0x3fff) + 1, (((bits >> 14) & 0x3fff) + 1)))
        }
        _ => None,
    }
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[..2] != [0xff, 0xd8] {
        return None;
    }
    let mut pos = 2;
    while pos + 9 < bytes.len() {
        if bytes[pos] != 0xff {
            pos += 1;
            continue;
        }
        while pos < bytes.len() && bytes[pos] == 0xff {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let marker = bytes[pos];
        pos += 1;
        if marker == 0xd8 || marker == 0xd9 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        if pos + 2 > bytes.len() {
            break;
        }
        let segment_len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        if segment_len < 2 || pos + segment_len > bytes.len() {
            break;
        }
        if (0xc0..=0xc3).contains(&marker)
            || (0xc5..=0xc7).contains(&marker)
            || (0xc9..=0xcb).contains(&marker)
            || (0xcd..=0xcf).contains(&marker)
        {
            return Some((
                u16::from_be_bytes([bytes[pos + 5], bytes[pos + 6]]) as u32,
                u16::from_be_bytes([bytes[pos + 3], bytes[pos + 4]]) as u32,
            ));
        }
        pos += segment_len;
    }
    None
}

/// Resolve `[blob:{id}]` references back to full content for historical client hydration.
pub fn resolve_body_refs(items: &mut [Item], data_root: &Path) -> Result<()> {
    let blob_dir = blob_dir(data_root);
    for item in items.iter_mut() {
        if let Item::FunctionCallOutput(out) = item
            && let FunctionCallOutput::Text(body) = &mut out.output
            && let Some(resolved) = resolve_single_blob(body, &blob_dir)
        {
            *body = resolved;
        }
    }
    Ok(())
}

pub fn resolve_single_blob(body: &str, blob_dir: &Path) -> Option<String> {
    let rest = body.strip_prefix(BLOB_PREFIX)?;
    let (id, _) = rest.split_once(']')?;
    let blob_path = blob_dir.join(format!("{id}.txt"));
    fs::read_to_string(blob_path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_and_spill_full_writes_complete_text_to_litecode_run_and_keeps_50_50_ends() {
        let dir = tempfile::tempdir().unwrap();
        let body = format!("HEAD_X\n{}MID_X\n{}TAIL_X", "a".repeat(2_000), "b".repeat(2_000));
        let rendered = cut_and_spill_full(&body, 400, Some(dir.path()), "probe");
        assert!(rendered.len() <= 400, "rendered len={}", rendered.len());
        assert!(rendered.contains("HEAD_X"), "{rendered}");
        assert!(rendered.contains("TAIL_X"), "{rendered}");
        assert!(rendered.contains(&format!("{} bytes total; full output: .litecode/run/probe_", body.len())), "{rendered}");

        let rel = rendered
            .split_once("full output: ")
            .unwrap()
            .1
            .lines()
            .next()
            .unwrap();
        let disk = fs::read_to_string(dir.path().join(rel)).unwrap();
        assert_eq!(disk, body);
    }

    #[test]
    fn cut_and_spill_respects_utf8_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let content = format!("头{}🚀{}尾", "中".repeat(5_000), "🙂".repeat(5_000));
        let rendered = cut_and_spill_full(&content, 600, Some(dir.path()), "utf8");
        assert!(rendered.len() <= 600, "{}", rendered.len());
        assert!(rendered.contains('头') && rendered.contains('尾'), "{rendered}");
        assert!(!rendered.contains('\u{FFFD}'));
    }
}
