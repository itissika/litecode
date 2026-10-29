//! Native media browser: one workspace walk plus on-demand image thumbnails.
//!
//! Listing uses the Explorer filter face (files.exclude, optional gitignore),
//! so build output and dependency trees stay out of the gallery. Nested
//! .litecode is the product runtime and is never listed.
//!
//! Thumbnails are derived files under .litecode/cache/media, keyed by source
//! path, size, mtime and requested edge. An edit changes the key, so a stale
//! thumbnail cannot survive a write. Decode failures still fall back to the
//! original bytes for formats the browser can paint (AVIF and friends).

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use image::codecs::jpeg::JpegEncoder;
use image::metadata::Orientation;
use image::{DynamicImage, ExtendedColorType, ImageDecoder, ImageReader, Limits, Rgb, RgbImage};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::filter::{FilterPreset, cheap_rel_under, walk_builder};

/// Hard cap on one listing. A folder drill-down narrows big libraries.
pub const MAX_MEDIA_ENTRIES: usize = 4000;
/// Longest thumbnail edge the cache will produce.
pub const MAX_THUMB_EDGE: u32 = 512;

const MIN_THUMB_EDGE: u32 = 64;
const MAX_SOURCE_EDGE: u32 = 16_384;
const MAX_SOURCE_PIXELS: u64 = 80_000_000;
const MAX_SOURCE_ALLOC: u64 = 256 * 1024 * 1024;
/// SVG thumbnails are the source; keep one oversized logo out of the panel.
const MAX_SVG_BYTES: u64 = 1024 * 1024;
/// Formats without a Rust decoder may still be browser-decodable (AVIF, HEIC).
const MAX_FALLBACK_BYTES: u64 = 8 * 1024 * 1024;
const THUMB_QUALITY: u8 = 78;
const CACHE_DIR: &str = ".litecode/cache/media";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

impl MediaKind {
    /// Classify by extension. Unknown suffixes are not media.
    pub fn of_path(path: &str) -> Option<Self> {
        match extension(path).to_ascii_lowercase().as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "avif" | "svg" | "tif"
            | "tiff" | "heic" | "heif" | "jxl" => Some(Self::Image),
            "mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "wmv" | "flv" | "mpg" | "mpeg"
            | "m2ts" | "3gp" | "ogv" | "asf" | "rmvb" => Some(Self::Video),
            "mp3" | "wav" | "ogg" | "oga" | "opus" | "m4a" | "aac" | "flac" | "weba" | "aiff"
            | "aif" | "wma" | "mid" | "midi" | "amr" => Some(Self::Audio),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaEntry {
    pub path: String,
    pub name: String,
    /// Parent directory, slash separated; empty at the workspace root.
    pub folder: String,
    pub kind: MediaKind,
    pub size: u64,
    /// Modified time in Unix milliseconds (0 when unavailable).
    pub mtime: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaListing {
    pub entries: Vec<MediaEntry>,
    /// The walk stopped at the entry cap; more files exist on disk.
    pub truncated: bool,
}

#[derive(Debug)]
pub struct MediaThumb {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("not an image: {0}")]
    Unsupported(String),
    #[error("image is too large to preview")]
    TooLarge,
    #[error("could not decode image: {0}")]
    Decode(String),
}

/// Media files under root, sorted case-insensitively by path.
pub fn list_media(root: &Path) -> std::io::Result<MediaListing> {
    let mut builder = walk_builder(root, FilterPreset::Explorer);
    builder.filter_entry(|entry| entry.file_name() != ".litecode");
    let walker = builder.build();

    let mut entries = Vec::new();
    let mut truncated = false;
    for result in walker {
        // A permission-denied subtree should not sink the whole listing.
        let Ok(entry) = result else { continue };
        if entry.depth() == 0 {
            continue;
        }
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let Some(kind) = MediaKind::of_path(&path.to_string_lossy()) else {
            continue;
        };
        let Some(rel) = cheap_rel_under(root, path) else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        let meta = entry.metadata().ok();
        entries.push(MediaEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            folder: rel
                .rsplit_once('/')
                .map(|(dir, _)| dir)
                .unwrap_or("")
                .to_string(),
            path: rel,
            kind,
            size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            mtime: meta.as_ref().map(mtime_ms).unwrap_or(0),
        });
        if entries.len() >= MAX_MEDIA_ENTRIES {
            truncated = true;
            break;
        }
    }

    entries.sort_by_cached_key(|entry| entry.path.to_lowercase());
    Ok(MediaListing { entries, truncated })
}

/// A JPEG thumbnail (or the untransformed source for SVG) for one image.
pub fn thumbnail(root: &Path, abs: &Path, rel: &str, edge: u32) -> Result<MediaThumb, MediaError> {
    let meta = fs::metadata(abs)?;
    if !meta.is_file() {
        return Err(MediaError::NotFound(rel.to_string()));
    }
    let edge = edge.clamp(MIN_THUMB_EDGE, MAX_THUMB_EDGE);

    let lower = extension(rel).to_ascii_lowercase();
    if lower == "svg" {
        if meta.len() > MAX_SVG_BYTES {
            return Err(MediaError::TooLarge);
        }
        return Ok(MediaThumb {
            bytes: fs::read(abs)?,
            mime: "image/svg+xml",
        });
    }
    if meta.len() == 0 {
        return Err(MediaError::Unsupported(rel.to_string()));
    }

    let cache_dir = root.join(CACHE_DIR);
    let cache_path = cache_dir.join(format!("{}.jpg", cache_key(rel, &meta, edge)));
    if let Ok(bytes) = fs::read(&cache_path) {
        if !bytes.is_empty() {
            return Ok(MediaThumb {
                bytes,
                mime: "image/jpeg",
            });
        }
    }

    match render_thumb(abs, edge) {
        Ok(bytes) => {
            write_cache(&cache_dir, &cache_path, &bytes);
            Ok(MediaThumb {
                bytes,
                mime: "image/jpeg",
            })
        }
        Err(err) => {
            // The browser may still decode formats the image crate does not.
            if meta.len() > MAX_FALLBACK_BYTES {
                return Err(err);
            }
            let mime = image_mime(rel);
            if mime == "application/octet-stream" {
                return Err(err);
            }
            Ok(MediaThumb {
                bytes: fs::read(abs)?,
                mime,
            })
        }
    }
}

fn render_thumb(abs: &Path, edge: u32) -> Result<Vec<u8>, MediaError> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_EDGE);
    limits.max_image_height = Some(MAX_SOURCE_EDGE);
    limits.max_alloc = Some(MAX_SOURCE_ALLOC);

    let mut reader = ImageReader::open(abs)?.with_guessed_format()?;
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| MediaError::Decode(e.to_string()))?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
        return Err(MediaError::TooLarge);
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image =
        DynamicImage::from_decoder(decoder).map_err(|e| MediaError::Decode(e.to_string()))?;
    image.apply_orientation(orientation);

    let thumb = image.thumbnail(edge, edge);
    let rgb = flatten_on_white(&thumb);
    let mut out = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut out, THUMB_QUALITY);
    encoder
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|e| MediaError::Decode(e.to_string()))?;
    Ok(out)
}

/// JPEG has no alpha channel; transparent logos read best on the panel surface
/// when flattened to white rather than to uninitialised black.
fn flatten_on_white(image: &DynamicImage) -> RgbImage {
    let rgba = image.to_rgba8();
    let mut out = RgbImage::new(rgba.width(), rgba.height());
    for (dst, src) in out.pixels_mut().zip(rgba.pixels()) {
        let alpha = u32::from(src[3]);
        let blend = |channel: u8| ((u32::from(channel) * alpha + 255 * (255 - alpha)) / 255) as u8;
        *dst = Rgb([blend(src[0]), blend(src[1]), blend(src[2])]);
    }
    out
}

fn write_cache(dir: &Path, path: &Path, bytes: &[u8]) {
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = dir.join(format!(
        "{}.{}.tmp",
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    ));
    if fs::write(&tmp, bytes).is_err() {
        return;
    }
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

fn cache_key(rel: &str, meta: &fs::Metadata, edge: u32) -> String {
    let mut hasher = Sha256::new();
    hasher.update(rel.as_bytes());
    hasher.update([0]);
    hasher.update(meta.len().to_le_bytes());
    hasher.update([0]);
    hasher.update(mtime_ms(meta).to_le_bytes());
    hasher.update([0]);
    hasher.update(edge.to_le_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(32);
    for byte in &digest[..16] {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn extension(path: &str) -> &str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("")
}

fn image_mime(rel: &str) -> &'static str {
    match extension(rel).to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "jxl" => "image/jxl",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn write_png(path: &Path, width: u32, height: u32) {
        let mut image = RgbImage::new(width, height);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = Rgb([(x % 256) as u8, (y % 256) as u8, 180]);
        }
        image.save(path).expect("write png");
    }

    #[test]
    fn classifies_media_by_extension() {
        assert_eq!(MediaKind::of_path("a/b/photo.JPG"), Some(MediaKind::Image));
        assert_eq!(MediaKind::of_path("c/d/clip.webm"), Some(MediaKind::Video));
        assert_eq!(MediaKind::of_path("song.flac"), Some(MediaKind::Audio));
        assert_eq!(MediaKind::of_path("readme.md"), None);
        assert_eq!(MediaKind::of_path("archive.tar.gz"), None);
        // TypeScript, not an MPEG transport stream.
        assert_eq!(MediaKind::of_path("web/src/main.ts"), None);
    }

    #[test]
    fn listing_skips_runtime_and_non_media() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::create_dir_all(root.join(".litecode/cache")).unwrap();
        fs::write(root.join("photo.png"), b"x").unwrap();
        fs::write(root.join("nested/clip.mp4"), b"x").unwrap();
        fs::write(root.join("nested/song.flac"), b"x").unwrap();
        fs::write(root.join("notes.txt"), b"x").unwrap();
        fs::write(root.join(".litecode/cache/hidden.png"), b"x").unwrap();

        let listing = list_media(root).unwrap();
        let paths: Vec<_> = listing.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["nested/clip.mp4", "nested/song.flac", "photo.png"]
        );
        assert!(!listing.truncated);
        let clip = listing
            .entries
            .iter()
            .find(|e| e.path.ends_with("clip.mp4"))
            .unwrap();
        assert_eq!(clip.folder, "nested");
        assert_eq!(clip.kind, MediaKind::Video);
    }

    #[test]
    fn thumbnail_is_bounded_and_cached() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = root.join("wide.png");
        write_png(&source, 800, 400);

        let first = thumbnail(root, &source, "wide.png", 200).unwrap();
        assert_eq!(first.mime, "image/jpeg");
        assert_eq!(&first.bytes[..2], &[0xff, 0xd8]);
        let decoded = image::load_from_memory(&first.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (200, 100));

        let cache = root.join(CACHE_DIR);
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);

        let second = thumbnail(root, &source, "wide.png", 200).unwrap();
        assert_eq!(second.bytes, first.bytes);
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);
    }

    #[test]
    fn svg_thumbnails_keep_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = root.join("logo.svg");
        fs::write(&source, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap();

        let thumb = thumbnail(root, &source, "logo.svg", 128).unwrap();
        assert_eq!(thumb.mime, "image/svg+xml");
        assert_eq!(thumb.bytes, fs::read(&source).unwrap());
    }

    #[test]
    fn extension_ignores_directories() {
        assert_eq!(extension("a.b/c"), "");
        assert_eq!(extension("a/b.PNG"), "PNG");
        assert_eq!(extension("plain"), "");
    }
}
