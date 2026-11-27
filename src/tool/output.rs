use std::fs;
use std::path::{Path, PathBuf};

use crate::authority::responses::FunctionCallOutput;
use crate::session::media::{MAX_MEDIA_BLOB_SIZE, write_media_blob};
use crate::types::{Item, MediaSource, Result, ToolOutputPart};
use crate::types::utf8::{prefix_utf8, suffix_utf8};

pub const RUN_SPILL_DIR: &str = ".litecode/run";
pub const BLOB_PREFIX: &str = "[blob:";
pub const DEFAULT_SPILL_THRESHOLD: usize = 32_768;

pub fn blob_dir(data_root: &Path) -> PathBuf {
    data_root.join("blobs")
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
    let dir = workspace_root.join(RUN_SPILL_DIR);
    fs::create_dir_all(&dir)?;
    let name = format!("{}_{}.txt", sanitize_stem(stem), crate::terminal::bash_nonce());
    fs::write(dir.join(&name), full_text.as_bytes())?;
    Ok(format!("{RUN_SPILL_DIR}/{name}"))
}

/// Result of spilling and clipping a `run_code` model view stream.
///
/// The clipped `text` keeps a minimal inline gap marker between head and tail,
/// while `upfloat_notice` carries the full-output path and exact omitted line
/// range for the upfloat block above the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpillCutResult {
    pub text: String,
    pub spilled_path: Option<String>,
    pub upfloat_notice: Option<String>,
    pub total_lines: usize,
    pub total_bytes: usize,
}

fn line_count_1based(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.bytes().filter(|&b| b == b'\n').count() + 1
    }
}

/// Cut `text` to fit `max_bytes`, spilling the complete unclipped text once
/// under `.litecode/run/` when it exceeds `max_bytes`.
///
/// Prefers line-aligned 50/50 head+tail cuts so the inline gap marker and
/// upfloat notice report exact 1-based omitted line ranges (`lines A-B omitted`).
pub fn cut_and_spill_lines(
    text: &str,
    max_bytes: usize,
    workspace_root: Option<&Path>,
    stem: &str,
) -> SpillCutResult {
    let total_bytes = text.len();
    let total_lines = line_count_1based(text);
    if total_bytes <= max_bytes {
        return SpillCutResult {
            text: text.to_string(),
            spilled_path: None,
            upfloat_notice: None,
            total_lines,
            total_bytes,
        };
    }
    let full_path = workspace_root.and_then(|root| match spill_full_text(root, stem, text) {
        Ok(rel) => Some(rel),
        Err(error) => {
            tracing::warn!(error = %error, stem, "failed to spill full tool output");
            None
        }
    });
    let path_suffix = full_path
        .as_deref()
        .map(|p| format!("; full output: {p}"))
        .unwrap_or_default();

    // Reserve enough space for the minimal inline gap line (`\n… lines X-Y omitted\n`).
    let marker_reserve = 64usize;
    if max_bytes <= marker_reserve {
        let gap = format!("… {total_bytes} bytes omitted");
        let upfloat = format!("output clipped ({total_lines} lines, {total_bytes} bytes){path_suffix}");
        return SpillCutResult {
            text: prefix_utf8(&gap, max_bytes).to_string(),
            spilled_path: full_path,
            upfloat_notice: Some(upfloat),
            total_lines,
            total_bytes,
        };
    }

    let preview_budget = max_bytes.saturating_sub(marker_reserve);
    let head_budget = preview_budget / 2;
    let tail_budget = preview_budget - head_budget;

    let head_raw = prefix_utf8(text, head_budget);
    // Align head end to a newline boundary when possible so kept head ends cleanly on a full line.
    let head_aligned = head_raw
        .rfind('\n')
        .map(|nl| (&text[..nl], nl + 1));

    let tail_search_start = head_aligned.map_or(head_raw.len(), |(_, after_nl)| after_nl);
    let tail_raw = suffix_utf8(&text[tail_search_start..], tail_budget);
    let tail_raw_start = total_bytes - tail_raw.len();
    let tail_aligned = tail_raw
        .find('\n')
        .map(|nl| (tail_raw_start + nl + 1, &tail_raw[nl + 1..]))
        .filter(|(_, slice)| !slice.is_empty());

    if let (Some((head, head_after_nl)), Some((tail_byte_start, tail))) = (head_aligned, tail_aligned)
        && head_after_nl <= tail_byte_start
    {
        let head_lines = line_count_1based(head);
        let tail_start_line = text[..tail_byte_start]
            .bytes()
            .filter(|&b| b == b'\n')
            .count()
            + 1;
        let omit_start = head_lines + 1;
        let omit_end = tail_start_line.saturating_sub(1);
        if omit_start <= omit_end {
            let inline_gap = format!("… lines {omit_start}-{omit_end} omitted");
            let combined = format!("{head}\n{inline_gap}\n{tail}");
            if combined.len() <= max_bytes {
                let upfloat = format!(
                    "output clipped (lines {omit_start}-{omit_end} of {total_lines} omitted, {total_bytes} bytes){path_suffix}"
                );
                return SpillCutResult {
                    text: combined,
                    spilled_path: full_path,
                    upfloat_notice: Some(upfloat),
                    total_lines,
                    total_bytes,
                };
            }
        }
    }

    // Fallback for single very long lines or narrow budgets: UTF-8 byte head+tail cut.
    let head = head_raw;
    let tail = suffix_utf8(&text[head.len()..], tail_budget);
    let omitted_bytes = total_bytes.saturating_sub(head.len() + tail.len());
    let head_line = text[..head.len()].bytes().filter(|&b| b == b'\n').count() + 1;
    let tail_start_byte = total_bytes.saturating_sub(tail.len());
    let tail_line = text[..tail_start_byte]
        .bytes()
        .filter(|&b| b == b'\n')
        .count()
        + 1;
    let (inline_gap, detail) = if tail_line > head_line + 1 {
        let s = head_line + 1;
        let e = tail_line - 1;
        (
            format!("… lines {s}-{e} omitted"),
            format!("lines {s}-{e} of {total_lines} omitted, {total_bytes} bytes"),
        )
    } else {
        (
            format!("… {omitted_bytes} bytes omitted (line {head_line})"),
            format!("{omitted_bytes} bytes omitted on line {head_line}, {total_bytes} bytes total"),
        )
    };
    let mut combined = match (head.is_empty(), tail.is_empty()) {
        (false, false) => format!("{head}\n{inline_gap}\n{tail}"),
        (false, true) => format!("{head}\n{inline_gap}"),
        (true, false) => format!("{inline_gap}\n{tail}"),
        (true, true) => inline_gap,
    };
    if combined.len() > max_bytes {
        combined = prefix_utf8(&combined, max_bytes).to_string();
    }
    let upfloat = format!("output clipped ({detail}){path_suffix}");
    SpillCutResult {
        text: combined,
        spilled_path: full_path,
        upfloat_notice: Some(upfloat),
        total_lines,
        total_bytes,
    }
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

