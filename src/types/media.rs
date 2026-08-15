//! Product / tool media adjuncts — **not** transcript authority.
//!
//! Transcript atoms live in [`crate::types::transcript`]. These types only
//! describe tool-local media payloads.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaSource {
    Url {
        url: String,
    },
    BlobRef {
        blob_id: String,
    },
    /// A tool-local path. The tool executor materializes this into a BlobRef
    /// before the result is persisted or encoded for an LLM request.
    LocalFile {
        path: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaArtifact {
    pub kind: MediaKind,
    pub source: MediaSource,
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_resolution: Option<String>,
}

impl MediaArtifact {
    pub fn image(source: MediaSource, mime_type: impl Into<String>) -> Self {
        Self {
            kind: MediaKind::Image,
            source,
            mime_type: mime_type.into(),
            width: None,
            height: None,
            duration_secs: None,
            detail: None,
            fps: None,
            media_resolution: None,
        }
    }

    pub fn video(source: MediaSource, mime_type: impl Into<String>) -> Self {
        Self {
            kind: MediaKind::Video,
            source,
            mime_type: mime_type.into(),
            width: None,
            height: None,
            duration_secs: None,
            detail: None,
            fps: None,
            media_resolution: None,
        }
    }

    pub fn audio(source: MediaSource, mime_type: impl Into<String>) -> Self {
        Self {
            kind: MediaKind::Audio,
            source,
            mime_type: mime_type.into(),
            width: None,
            height: None,
            duration_secs: None,
            detail: None,
            fps: None,
            media_resolution: None,
        }
    }
}

/// Tool-local structured output parts (not a Responses transcript Item).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolOutputPart {
    Text { text: String },
    Media { artifact: MediaArtifact },
}

impl ToolOutputPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image(source: MediaSource, mime_type: impl Into<String>) -> Self {
        Self::Media {
            artifact: MediaArtifact::image(source, mime_type),
        }
    }

    pub fn image_file(path: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self::image(MediaSource::LocalFile { path: path.into() }, mime_type)
    }

    pub fn video(source: MediaSource, mime_type: impl Into<String>) -> Self {
        Self::Media {
            artifact: MediaArtifact::video(source, mime_type),
        }
    }

    pub fn audio(source: MediaSource, mime_type: impl Into<String>) -> Self {
        Self::Media {
            artifact: MediaArtifact::audio(source, mime_type),
        }
    }

    pub fn required_capabilities(&self) -> &'static [&'static str] {
        match self {
            Self::Text { .. } => &["text"],
            Self::Media { artifact } => match artifact.kind {
                MediaKind::Image => &["image"],
                MediaKind::Video => &["video"],
                MediaKind::Audio => &["audio"],
            },
        }
    }
}
