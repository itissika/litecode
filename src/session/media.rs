//! Session-scoped media.
//!
//! Tool bytes still go through the content-addressed blob store (`blobs/`),
//! which is garbage-collected. User-pasted images are different: they live in
//! `data_root/media/{sha256}.{ext}`, are never collected, and the transcript
//! stores only a `litecode-media:` reference. The bytes are expanded to a
//! data URL on the ephemeral model view, not in the database.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use sha2::{Digest, Sha256};

use crate::authority::responses::{
    FunctionCallOutput, InputContent, InputTextContent, MessageItem,
};
use crate::types::{Item, LitecodeError, MediaArtifact, MediaSource, Result};

/// Prefix of an `input_image.image_url` that names a file under `media/`.
pub const MEDIA_REF_PREFIX: &str = "litecode-media:";

/// How many images one user message may carry.
pub const MAX_USER_IMAGES: usize = 16;

pub const MAX_MEDIA_BLOB_SIZE: u64 = 10 * 1024 * 1024;

/// Whether an Item carries any image/file content. Used by the mechanical
/// pruner and by replay projection to rebuild media-rebase provenance from the
/// persisted replacement source seq.
pub fn item_has_media(item: &Item) -> bool {
    match item {
        Item::Message(MessageItem::Input(message)) => message.content.iter().any(|part| {
            matches!(
                part,
                InputContent::InputImage(_) | InputContent::InputFile(_)
            )
        }),
        Item::FunctionCallOutput(output) => match &output.output {
            FunctionCallOutput::Text(_) => false,
            FunctionCallOutput::Content(parts) => parts.iter().any(|part| {
                matches!(
                    part,
                    InputContent::InputImage(_) | InputContent::InputFile(_)
                )
            }),
        },
        _ => false,
    }
}

pub fn media_dir(data_root: &Path) -> PathBuf {
    data_root.join("media")
}

pub fn media_blob_path(data_root: &Path, blob_id: &str) -> PathBuf {
    media_dir(data_root).join(blob_id)
}

/// Write raw bytes via the content-addressed blob store; returns the blob id.
pub fn write_media_blob(data_root: &Path, bytes: &[u8]) -> Result<String> {
    crate::session::data::put_bytes(data_root, bytes)
}

/// `litecode-media:{sha256}.{ext}` for a file this module wrote.
pub fn media_ref(name: &str) -> String {
    format!("{MEDIA_REF_PREFIX}{name}")
}

/// File name inside a `litecode-media:` URL, when the name is a safe hash.
pub fn parse_media_ref(url: &str) -> Option<&str> {
    let name = url.strip_prefix(MEDIA_REF_PREFIX)?;
    valid_media_name(name).then_some(name)
}

/// `^[0-9a-f]{64}\.(jpg|png|webp|gif)$` — nothing else may be joined onto `media/`.
pub fn valid_media_name(name: &str) -> bool {
    let Some((hex, ext)) = name.split_once('.') else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        && matches!(ext, "jpg" | "png" | "webp" | "gif")
        && !name.contains(['/', '\\'])
}

pub fn mime_for_name(name: &str) -> Option<&'static str> {
    match name.rsplit('.').next() {
        Some("jpg") => Some("image/jpeg"),
        Some("png") => Some("image/png"),
        Some("webp") => Some("image/webp"),
        Some("gif") => Some("image/gif"),
        _ => None,
    }
}

/// Magic-byte type of a user upload. The extension is what we store on disk.
pub fn sniff_image(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(("image/png", "png"));
    }
    if bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff {
        return Some(("image/jpeg", "jpg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(("image/gif", "gif"));
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(("image/webp", "webp"));
    }
    None
}

/// Accept a client list of `litecode-media:` refs, or say why it is rejected.
pub fn normalize_image_refs(
    refs: impl IntoIterator<Item = String>,
) -> std::result::Result<Vec<String>, String> {
    let refs: Vec<String> = refs.into_iter().collect();
    if refs.len() > MAX_USER_IMAGES {
        return Err(format!("at most {MAX_USER_IMAGES} images"));
    }
    for media_ref in &refs {
        if parse_media_ref(media_ref).is_none() {
            return Err("invalid image ref".into());
        }
    }
    Ok(refs)
}

/// Write user-image bytes under `media/{sha256}.{ext}`. Existing content is kept.
///
/// These files are not registered with the blob store, so startup GC never
/// deletes them. A missing file is a user deletion and stays missing.
pub fn write_user_media(data_root: &Path, bytes: &[u8], ext: &str) -> Result<String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_MEDIA_BLOB_SIZE {
        return Err(LitecodeError::ToolExecution(format!(
            "user image must be 1..={MAX_MEDIA_BLOB_SIZE} bytes"
        )));
    }
    if !matches!(ext, "jpg" | "png" | "webp" | "gif") {
        return Err(LitecodeError::ToolExecution(
            "user image extension is not supported".into(),
        ));
    }
    let name = format!("{}.{ext}", hex_encode(&Sha256::digest(bytes)));
    let dir = media_dir(data_root);
    fs::create_dir_all(&dir)?;
    let dest = dir.join(&name);
    if dest.exists() {
        return Ok(name);
    }
    let tmp = dir.join(format!(".{name}.tmp"));
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    match fs::rename(&tmp, &dest) {
        Ok(()) => Ok(name),
        Err(error) => {
            let _ = fs::remove_file(&tmp);
            if dest.exists() {
                Ok(name)
            } else {
                Err(LitecodeError::ToolExecution(format!(
                    "failed to store user image: {error}"
                )))
            }
        }
    }
}

/// Read a user image by its validated file name.
pub fn read_user_media(data_root: &Path, name: &str) -> Result<Vec<u8>> {
    if !valid_media_name(name) {
        return Err(LitecodeError::ToolExecution(format!(
            "invalid user image name '{name}'"
        )));
    }
    fs::read(media_dir(data_root).join(name)).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            LitecodeError::MediaBlobMissing(name.to_string())
        } else {
            LitecodeError::ToolExecution(format!("failed to read user image '{name}': {error}"))
        }
    })
}

/// Expand `litecode-media:` parts on an ephemeral model view.
///
/// A missing file becomes a text note so the turn still runs. Other image URLs
/// (tool data URLs, remote URLs) are left untouched.
pub fn resolve_user_media(items: &mut [Item], data_root: &Path) {
    for item in items.iter_mut() {
        let Item::Message(MessageItem::Input(message)) = item else {
            continue;
        };
        for part in message.content.iter_mut() {
            let action = user_media_action(part, data_root);
            match action {
                MediaAction::Leave => {}
                MediaAction::DataUrl(url) => {
                    if let InputContent::InputImage(image) = part {
                        image.image_url = Some(url);
                    }
                }
                MediaAction::Missing(name) => {
                    *part = InputContent::InputText(InputTextContent {
                        text: format!("[image missing: {name}]"),
                    });
                }
            }
        }
    }
}

enum MediaAction {
    Leave,
    DataUrl(String),
    Missing(String),
}

fn user_media_action(part: &InputContent, data_root: &Path) -> MediaAction {
    let InputContent::InputImage(image) = part else {
        return MediaAction::Leave;
    };
    let Some(url) = image.image_url.as_deref() else {
        return MediaAction::Leave;
    };
    let Some(name) = parse_media_ref(url) else {
        return MediaAction::Leave;
    };
    match read_user_media(data_root, name) {
        Ok(bytes) => {
            let mime = mime_for_name(name).unwrap_or("application/octet-stream");
            MediaAction::DataUrl(format!("data:{mime};base64,{}", BASE64.encode(bytes)))
        }
        Err(_) => MediaAction::Missing(name.to_string()),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn read_media_blob(data_root: &Path, blob_id: &str) -> Result<Vec<u8>> {
    match crate::session::data::read_bytes(data_root, blob_id) {
        Ok(bytes) => Ok(bytes),
        Err(_) => {
            let path = media_blob_path(data_root, blob_id);
            fs::read(path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    LitecodeError::MediaBlobMissing(blob_id.to_string())
                } else {
                    LitecodeError::ToolExecution(format!(
                        "failed to read media blob '{blob_id}': {e}"
                    ))
                }
            })
        }
    }
}

/// Resolve a typed media artifact to a URL suitable for OpenAI wire encoding.
///
/// Uses the MIME type recorded by the producing tool.
pub fn resolve_media_artifact_url(
    data_root: Option<&Path>,
    artifact: &MediaArtifact,
) -> Result<String> {
    if artifact.mime_type.trim().is_empty() {
        return Err(LitecodeError::ToolExecution(
            "media artifact is missing mime_type".into(),
        ));
    }
    match &artifact.source {
        MediaSource::Url { url } => Ok(url.clone()),
        MediaSource::BlobRef { blob_id } => {
            let data_root = data_root.ok_or_else(|| {
                LitecodeError::ToolExecution("BlobRef media requires session data_root".into())
            })?;
            let bytes = read_media_blob(data_root, blob_id)?;
            Ok(format!(
                "data:{};base64,{}",
                artifact.mime_type,
                BASE64.encode(bytes)
            ))
        }
        MediaSource::LocalFile { path } => Err(LitecodeError::ToolExecution(format!(
            "unmaterialized local media path: {path}"
        ))),
    }
}

#[cfg(test)]
mod tests {}
