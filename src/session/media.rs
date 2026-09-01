//! Session-scoped media blob storage (`data_root/media/{blob_id}`).

use std::fs;
use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::types::{LitecodeError, MediaArtifact, MediaSource, Result};

pub const MAX_MEDIA_BLOB_SIZE: u64 = 10 * 1024 * 1024;

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
mod tests {
    use super::*;
    use crate::types::MediaArtifact;

    #[test]
    fn blob_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let blob_id = write_media_blob(dir.path(), b"hello").unwrap();
        let bytes = read_media_blob(dir.path(), &blob_id).unwrap();
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn typed_artifact_uses_recorded_mime_type() {
        let dir = tempfile::tempdir().unwrap();
        let blob_id = write_media_blob(dir.path(), b"png").unwrap();
        let url = resolve_media_artifact_url(
            Some(dir.path()),
            &MediaArtifact::image(MediaSource::BlobRef { blob_id }, "image/png"),
        )
        .unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }
}
