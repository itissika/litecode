//! Shared Item-native media token accounting (vision tiles + named fallbacks).
//!
//! Used by [`crate::session::estimate::compute_token_estimate`] and
//! [`crate::context_pipeline::media_budget`] so trim math and budget truth share one cost model.
//!
//! **Not** Message/ContentBlock dialect — operates on Responses [`InputContent`] only.

use crate::authority::responses::{ImageDetail, InputContent, InputFileContent, InputImageContent};

/// OpenAI vision low-detail / per-image base cost (gpt-4-vision style).
pub const IMAGE_BASE_TOKENS: usize = 85;

/// Tokens per 512×512 tile after scaling (gpt-4-vision high-detail).
pub const IMAGE_TILE_TOKENS: usize = 170;

/// When width/height unknown and detail ≠ low: assume ~4 tiles → 85 + 4×170.
pub const IMAGE_FALLBACK_TOKENS: usize = IMAGE_BASE_TOKENS + 4 * IMAGE_TILE_TOKENS;

/// Conservative fallback for video `InputFile` (no duration/resolution in Responses shape).
pub const VIDEO_FALLBACK_TOKENS: usize = 2_000;

/// Conservative fallback for audio `InputFile`.
pub const AUDIO_FALLBACK_TOKENS: usize = 500;

/// Unclassifiable document-like `InputFile` (pdf/txt/etc.).
pub const FILE_FALLBACK_TOKENS: usize = 1_100;

/// OpenAI-style high-detail vision tile count for known pixel dimensions.
///
/// Algorithm (cookbook / gpt-4-vision):
/// 1. Scale so longest side ≤ 2048
/// 2. Scale so shortest side ≤ 768
/// 3. Count ceil(w/512) × ceil(h/512) tiles
/// 4. tokens = [`IMAGE_BASE_TOKENS`] + [`IMAGE_TILE_TOKENS`] × tiles
pub fn image_tokens_for_dimensions(width: u32, height: u32, detail: ImageDetail) -> usize {
    if matches!(detail, ImageDetail::Low) {
        return IMAGE_BASE_TOKENS;
    }
    let (mut w, mut h) = (width.max(1) as f64, height.max(1) as f64);
    let longest = w.max(h);
    if longest > 2048.0 {
        let scale = 2048.0 / longest;
        w *= scale;
        h *= scale;
    }
    let shortest = w.min(h);
    if shortest > 768.0 {
        let scale = 768.0 / shortest;
        w *= scale;
        h *= scale;
    }
    let tiles_w = (w / 512.0).ceil() as usize;
    let tiles_h = (h / 512.0).ceil() as usize;
    let tiles = tiles_w.saturating_mul(tiles_h).max(1);
    IMAGE_BASE_TOKENS + IMAGE_TILE_TOKENS * tiles
}

/// Token cost for an [`InputImageContent`] when pixel size is unknown.
pub fn input_image_tokens(image: &InputImageContent) -> usize {
    match image.detail {
        ImageDetail::Low => IMAGE_BASE_TOKENS,
        _ => IMAGE_FALLBACK_TOKENS,
    }
}

/// Classify an `InputFile` as image / video / audio / document for costing + capability checks.
///
/// Returns `Some("image"|"video"|"audio")` when mime, filename, or URL suffix is clear;
/// `None` for unclassifiable document-like files.
pub fn classify_input_file(file: &InputFileContent) -> Option<&'static str> {
    let hints = [
        file.filename.as_deref().unwrap_or(""),
        file.file_url.as_deref().unwrap_or(""),
        file.file_data.as_deref().unwrap_or(""),
    ];
    let joined = hints.join(" ").to_ascii_lowercase();

    if looks_like_image(&joined) {
        return Some("image");
    }
    if looks_like_video(&joined) {
        return Some("video");
    }
    if looks_like_audio(&joined) {
        return Some("audio");
    }
    None
}

fn looks_like_image(s: &str) -> bool {
    s.contains("image/")
        || s.contains(".png")
        || s.contains(".jpg")
        || s.contains(".jpeg")
        || s.contains(".gif")
        || s.contains(".webp")
        || s.contains(".bmp")
        || s.contains("data:image/")
}

fn looks_like_video(s: &str) -> bool {
    s.contains("video/")
        || s.contains(".mp4")
        || s.contains(".webm")
        || s.contains(".mov")
        || s.contains(".mkv")
        || s.contains("data:video/")
}

fn looks_like_audio(s: &str) -> bool {
    s.contains("audio/")
        || s.contains(".mp3")
        || s.contains(".wav")
        || s.contains(".ogg")
        || s.contains(".flac")
        || s.contains(".m4a")
        || s.contains("data:audio/")
}

/// Per-part media token cost for one Responses [`InputContent`] (0 for text).
pub fn input_content_media_tokens(content: &InputContent) -> usize {
    match content {
        InputContent::InputText(_) => 0,
        InputContent::InputImage(img) => input_image_tokens(img),
        InputContent::InputFile(file) => match classify_input_file(file) {
            Some("image") => IMAGE_FALLBACK_TOKENS,
            Some("video") => VIDEO_FALLBACK_TOKENS,
            Some("audio") => AUDIO_FALLBACK_TOKENS,
            _ => FILE_FALLBACK_TOKENS,
        },
    }
}

/// Optional dims from a tool [`crate::types::MediaArtifact`] before encode (shared with estimate).
pub fn media_artifact_tokens(
    kind: crate::types::MediaKind,
    width: Option<u32>,
    height: Option<u32>,
    detail: ImageDetail,
) -> usize {
    match kind {
        crate::types::MediaKind::Image => match (width, height) {
            (Some(w), Some(h)) => image_tokens_for_dimensions(w, h, detail),
            _ if matches!(detail, ImageDetail::Low) => IMAGE_BASE_TOKENS,
            _ => IMAGE_FALLBACK_TOKENS,
        },
        crate::types::MediaKind::Video => VIDEO_FALLBACK_TOKENS,
        crate::types::MediaKind::Audio => AUDIO_FALLBACK_TOKENS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::InputTextContent;

    #[test]
    fn low_detail_is_base_only() {
        assert_eq!(
            image_tokens_for_dimensions(2048, 2048, ImageDetail::Low),
            IMAGE_BASE_TOKENS
        );
    }

    #[test]
    fn known_dims_use_tile_math() {
        // 1024×1024 → after scale shortest=768 → 768×768 → 2×2 tiles
        let n = image_tokens_for_dimensions(1024, 1024, ImageDetail::High);
        assert_eq!(n, IMAGE_BASE_TOKENS + IMAGE_TILE_TOKENS * 4);
    }

    #[test]
    fn text_part_costs_zero_media() {
        assert_eq!(
            input_content_media_tokens(&InputContent::InputText(InputTextContent {
                text: "hi".into(),
            })),
            0
        );
    }

    #[test]
    fn classify_video_url() {
        let f = InputFileContent {
            file_data: None,
            file_id: None,
            file_url: Some("https://cdn.example.com/clip.mp4".into()),
            filename: None,
            detail: None,
        };
        assert_eq!(classify_input_file(&f), Some("video"));
        assert_eq!(
            input_content_media_tokens(&InputContent::InputFile(f)),
            VIDEO_FALLBACK_TOKENS
        );
    }

    #[test]
    fn artifact_dims_prefer_tile_math() {
        let n = media_artifact_tokens(
            crate::types::MediaKind::Image,
            Some(1024),
            Some(1024),
            ImageDetail::High,
        );
        assert_eq!(n, IMAGE_BASE_TOKENS + IMAGE_TILE_TOKENS * 4);
    }
}
