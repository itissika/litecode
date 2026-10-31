use std::fs;
use std::path::{Path, PathBuf};

use crate::authority::responses::FunctionCallOutput;
use crate::session::media::{MAX_MEDIA_BLOB_SIZE, write_media_blob};
use crate::session::store::Session;
use crate::tool::trait_::Tool;
use crate::types::{Item, MediaSource, Result, ToolCallResult, ToolOutputPart};

/// Direct callers still cap model-facing text. Code callers do not consult this.
pub fn direct_output_limit(tool: &dyn Tool) -> usize {
    tool.max_result_size()
}

pub fn cap_direct_content(tool: &dyn Tool, content: &str) -> String {
    let max = direct_output_limit(tool);
    if max < usize::MAX {
        Session::truncated_tool_result(content, max)
    } else {
        content.to_string()
    }
}

pub const BLOB_PREFIX: &str = "[blob:";

pub const EMERGENCY_HARD_CAP_CHARS: usize = 400_000;
pub const EMERGENCY_PREVIEW_CHARS: usize = 2_000;

pub const DEFAULT_SPILL_THRESHOLD: usize = 32_768;

pub fn blob_dir(data_root: &Path) -> PathBuf {
    data_root.join("blobs")
}

/// Circuit-breaker for runaway tool output before spill/persist.
pub fn apply_emergency_cap(content: String) -> String {
    if content.len() <= EMERGENCY_HARD_CAP_CHARS {
        return content;
    }
    let estimated_tokens = content.len() / 4;
    tracing::warn!(
        chars = content.len(),
        estimated_tokens,
        "tool output exceeds token cap — circuit breaker fired"
    );
    let preview = crate::types::utf8::prefix_utf8(&content, EMERGENCY_PREVIEW_CHARS);
    let has_more = content.len() > EMERGENCY_PREVIEW_CHARS;
    format!(
        "<persisted-output>\n\
         Output exceeded token budget (~{et} tokens, {ch} chars).  \
         Full output not shown. Re-run with narrower parameters.\n\
         \n\
         Preview (first {preview_ch} chars):\n\n\
         {preview}{ellipsis}\n\
         </persisted-output>",
        et = estimated_tokens,
        ch = content.len(),
        preview_ch = EMERGENCY_PREVIEW_CHARS,
        preview = preview,
        ellipsis = if has_more { "\n..." } else { "" },
    )
}

/// Normalize a single tool result: emergency cap, then optional blob spill.
pub fn finalize_tool_output(
    content: String,
    data_root: &Path,
    spill_threshold: usize,
) -> Result<String> {
    let content = apply_emergency_cap(content);
    spill_content_if_needed(content, data_root, spill_threshold)
}

pub fn finalize_tool_call_result(
    result: ToolCallResult,
    data_root: &Path,
    spill_threshold: usize,
) -> Result<ToolCallResult> {
    // Materialize media before text spill so a media failure cannot leave an
    // orphaned text blob while discarding the structured parts.
    let parts = materialize_media_parts(result.parts, data_root)?;
    Ok(ToolCallResult {
        content: finalize_tool_output(result.content, data_root, spill_threshold)?,
        data: result.data,
        error: result.error,
        parts,
        metadata: result.metadata,
        level: result.level,
        hint: result.hint,
        warning_status: result.warning_status,
        appendix: result.appendix,
        composed: result.composed,
        diagnostics: result.diagnostics,
        error_code: result.error_code,
        error_details: result.error_details,
    })
}

fn materialize_media_parts(
    parts: Vec<ToolOutputPart>,
    data_root: &Path,
) -> Result<Vec<ToolOutputPart>> {
    parts
        .into_iter()
        .map(|part| {
            let ToolOutputPart::Media { mut artifact } = part else {
                return Ok(part);
            };
            if let MediaSource::LocalFile { path } = &artifact.source {
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

fn spill_content_if_needed(body: String, data_root: &Path, threshold: usize) -> Result<String> {
    if threshold == 0 || body.len() <= threshold || body.starts_with(BLOB_PREFIX) {
        return Ok(body);
    }
    let original_len = body.len();
    let blob_id = ulid::Ulid::new().to_string();
    let blob_dir = blob_dir(data_root);
    fs::create_dir_all(&blob_dir)?;
    let blob_path = blob_dir.join(format!("{blob_id}.txt"));
    fs::write(&blob_path, body.as_bytes())?;
    let preview_len = 500.min(original_len);
    let preview = crate::types::utf8::prefix_utf8(&body, preview_len);
    Ok(format!(
        "{BLOB_PREFIX}{blob_id}]\n{preview}... [spilled, {original_len} bytes total]",
    ))
}

/// Spill oversized function_call_output text to `{data_root}/blobs/`.
pub fn spill_tool_results(items: &mut [Item], data_root: &Path, threshold: usize) -> Result<()> {
    if threshold == 0 {
        return Ok(());
    }
    for item in items.iter_mut() {
        if let Item::FunctionCallOutput(out) = item
            && let FunctionCallOutput::Text(body) = &mut out.output
        {
            if body.len() <= threshold || body.starts_with(BLOB_PREFIX) {
                continue;
            }
            let call_id = out.call_id.clone();
            let spilled = spill_content_if_needed(std::mem::take(body), data_root, threshold)?;
            tracing::debug!(
                call_id = %call_id,
                bytes = spilled.len(),
                "spilled tool result to blob"
            );
            *body = spilled;
        }
    }
    Ok(())
}

/// Resolve `[blob:{id}]` references back to full content for materialization.
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
mod tests {}
