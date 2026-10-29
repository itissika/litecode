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
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::metadata::Orientation;
use image::{DynamicImage, ExtendedColorType, ImageDecoder, ImageFormat, ImageReader, Limits};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::filter::{ExcludeMatcher, FilterPreset, cheap_rel_under, path_gitignored, walk_builder};

/// Safety valve on one listing, not a page size. A media-heavy workspace is
/// ordinary (a game project holds ten thousand sounds and textures), and the
/// cap drops whole subtrees from the walk, so it has to sit well above what a
/// real workspace produces. The panel pages the grid on its own.
pub const MAX_MEDIA_ENTRIES: usize = 20_000;
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
/// Version of the thumbnail encoding, part of both the disk key and the URL
/// the browser caches. Bump it whenever a thumbnail would come out
/// different from the same source; mirrored by `MEDIA_THUMB_VERSION` in
/// `web/src/api/workspace.ts`, which keeps an `immutable` response from
/// outliving the encoder that wrote it.
pub const THUMB_CACHE_VERSION: &str = "2";
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

/// Generation-keyed snapshot of one workspace listing.
///
/// Producers of change (an internal write, a watcher event, a lost watcher
/// stream) only bump the generation; a request that sees a newer generation
/// walks again. Walking stays on the request path — nothing scans in the
/// background and nothing expires on a timer.
#[derive(Debug, Default)]
pub struct MediaCache {
    generation: AtomicU64,
    snapshot: RwLock<Option<(u64, Arc<MediaListing>)>>,
}

impl MediaCache {
    /// Mark the snapshot stale. Lock-free, so any producer may call it from any
    /// context; the next ensure() pays for the walk.
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// The listing for root, walking only when the snapshot is stale.
    pub fn ensure(&self, root: &Path) -> std::io::Result<Arc<MediaListing>> {
        self.ensure_with(|| list_media(root))
    }

    /// ensure() with the walk injected, so a test can change the tree mid-walk.
    fn ensure_with(
        &self,
        walk: impl FnOnce() -> std::io::Result<MediaListing>,
    ) -> std::io::Result<Arc<MediaListing>> {
        // Read the generation before the walk: a change that lands while the walk
        // runs leaves the stored snapshot older than the counter, so the next call
        // walks again instead of trusting a list built from a moving tree.
        let generation = self.generation.load(Ordering::SeqCst);
        if let Some(stored) = self.read_snapshot() {
            if stored.0 == generation {
                return Ok(stored.1);
            }
        }

        let mut slot = self.snapshot.write().unwrap_or_else(|e| e.into_inner());
        // A racing request may have filled the slot between the miss and the lock.
        if let Some((stored, listing)) = slot.as_ref() {
            if *stored == generation {
                return Ok(Arc::clone(listing));
            }
        }
        let listing = Arc::new(walk()?);
        *slot = Some((generation, Arc::clone(&listing)));
        Ok(listing)
    }

    /// Patch the snapshot for paths a writer can name, instead of dropping it.
    ///
    /// A rename or a trash action touches one entry; walking the workspace to
    /// rediscover that is exactly the cost this cache exists to avoid. Every
    /// path is re-stated from disk, so the patch stays exact: a path that is
    /// gone drops itself (and anything under it), a media file is re-stated, and
    /// anything else is a no-op. A directory is the one shape a name cannot
    /// describe, so it leaves the snapshot stale and the next listing walks.
    ///
    /// Returns whether the patch was applied. The snapshot is left alone when no
    /// changed path is media, so an edit elsewhere costs nothing.
    pub fn apply(&self, root: &Path, changed: &[String]) -> bool {
        let mut slot = self.snapshot.write().unwrap_or_else(|e| e.into_inner());
        let Some((stored_generation, listing)) = slot.as_mut() else {
            return true;
        };
        let generation = self.generation.load(Ordering::SeqCst);
        // Another change already invalidated the snapshot: it cannot claim to
        // describe the tree even after this patch, so leave the walk for later.
        if *stored_generation != generation {
            return false;
        }
        let excludes = ExcludeMatcher::for_preset(FilterPreset::Explorer);
        let Some(plan) = plan_patch(root, &excludes, &listing.entries, changed) else {
            self.generation.fetch_add(1, Ordering::SeqCst);
            return false;
        };
        if plan.is_empty() {
            return true;
        }

        let next = Arc::make_mut(listing);
        for patch in plan {
            match patch {
                Patch::Drop(rel) => {
                    let prefix = format!("{rel}/");
                    next.entries
                        .retain(|entry| entry.path != rel && !entry.path.starts_with(&prefix));
                }
                Patch::Upsert(entry) => {
                    next.entries.retain(|old| old.path != entry.path);
                    next.entries.push(entry);
                }
            }
        }
        next.entries
            .sort_by_cached_key(|entry| entry.path.to_lowercase());

        // Only claim the new generation if no other change slipped in while the
        // patch held the lock; a lost race leaves the snapshot stale, which
        // costs one walk but never hides a change.
        if self
            .generation
            .compare_exchange(
                generation,
                generation + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
        {
            *stored_generation = generation + 1;
            true
        } else {
            false
        }
    }

    fn read_snapshot(&self) -> Option<(u64, Arc<MediaListing>)> {
        let guard = self.snapshot.read().unwrap_or_else(|e| e.into_inner());
        guard
            .as_ref()
            .map(|(generation, listing)| (*generation, Arc::clone(listing)))
    }
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

/// One exact edit to the listing, derived from a path that changed on disk.
enum Patch {
    /// The path is gone: drop it and everything under it.
    Drop(String),
    /// The path is a media file: replace (or add) its entry.
    Upsert(MediaEntry),
}

/// What a changed path means for the listing, or None when it cannot be
/// applied exactly (a directory, or a name that resolves to nothing).
fn plan_patch(
    root: &Path,
    excludes: &ExcludeMatcher,
    entries: &[MediaEntry],
    changed: &[String],
) -> Option<Vec<Patch>> {
    let mut plan = Vec::new();
    for rel in changed {
        let rel = rel.replace('\\', "/");
        let rel = rel.trim_start_matches("./").trim_end_matches('/');
        if rel.is_empty() {
            return None;
        }
        let Ok(meta) = fs::symlink_metadata(root.join(rel)) else {
            // Gone. The name may have been a folder, so the subtree goes too.
            let prefix = format!("{rel}/");
            if entries
                .iter()
                .any(|entry| entry.path == rel || entry.path.starts_with(&prefix))
            {
                plan.push(Patch::Drop(rel.to_string()));
            }
            continue;
        };
        if meta.is_dir() {
            // A folder's new contents are not knowable from its name.
            return None;
        }
        if !meta.is_file() {
            continue;
        }
        match MediaKind::of_path(rel) {
            Some(kind) if media_visible(root, excludes, rel) => {
                plan.push(Patch::Upsert(MediaEntry {
                    name: rel.rsplit('/').next().unwrap_or(rel).to_string(),
                    folder: rel
                        .rsplit_once('/')
                        .map(|(dir, _)| dir)
                        .unwrap_or("")
                        .to_string(),
                    path: rel.to_string(),
                    kind,
                    size: meta.len(),
                    mtime: mtime_ms(&meta),
                }))
            }
            // A path that stopped being media (renamed to .txt, or now excluded).
            _ if entries.iter().any(|entry| entry.path == rel) => {
                plan.push(Patch::Drop(rel.to_string()));
            }
            _ => {}
        }
    }
    Some(plan)
}

/// The same visibility the walk applies: product-internal trees, the Explorer
/// exclude face, and gitignore when that face honors it.
fn media_visible(root: &Path, excludes: &ExcludeMatcher, rel: &str) -> bool {
    if rel.split('/').any(|part| part == ".litecode") {
        return false;
    }
    if excludes.matches(rel) {
        return false;
    }
    !path_gitignored(root, rel, FilterPreset::Explorer)
}

/// A JPEG thumbnail (a PNG when the source has alpha, the source itself for
/// SVG) for one image.
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
    let key = cache_key(rel, &meta, edge);
    // Either format may be cached; the key carries the encoder version, so a
    // hit was always written by this code.
    for (ext, mime) in [("png", "image/png"), ("jpg", "image/jpeg")] {
        if let Ok(bytes) = fs::read(cache_dir.join(format!("{key}.{ext}"))) {
            if !bytes.is_empty() {
                return Ok(MediaThumb { bytes, mime });
            }
        }
    }

    match render_thumb(abs, edge) {
        Ok(rendered) => {
            let cache_path = cache_dir.join(format!("{key}.{}", rendered.ext));
            write_cache(&cache_dir, &cache_path, &rendered.bytes);
            Ok(MediaThumb {
                bytes: rendered.bytes,
                mime: rendered.mime,
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

struct RenderedThumb {
    bytes: Vec<u8>,
    mime: &'static str,
    ext: &'static str,
}

fn render_thumb(abs: &Path, edge: u32) -> Result<RenderedThumb, MediaError> {
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
    // A source with an alpha channel keeps it: the panel paints its own surface
    // behind the thumbnail, so a flattened white box would misrepresent the
    // asset. Opaque sources stay a small JPEG.
    if thumb.color().has_alpha() {
        let mut out = Cursor::new(Vec::new());
        thumb
            .write_to(&mut out, ImageFormat::Png)
            .map_err(|e| MediaError::Decode(e.to_string()))?;
        return Ok(RenderedThumb {
            bytes: out.into_inner(),
            mime: "image/png",
            ext: "png",
        });
    }

    let rgb = thumb.to_rgb8();
    let mut bytes = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut bytes, THUMB_QUALITY);
    encoder
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|e| MediaError::Decode(e.to_string()))?;
    Ok(RenderedThumb {
        bytes,
        mime: "image/jpeg",
        ext: "jpg",
    })
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
    // Encoder version: changing how thumbnails render must not be answered
    // from a cache file the old code wrote.
    hasher.update(THUMB_CACHE_VERSION.as_bytes());
    hasher.update([0]);
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
    fn a_transparent_source_keeps_its_alpha() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = root.join("logo.png");
        let mut pixels = image::RgbaImage::new(8, 8);
        for pixel in pixels.pixels_mut() {
            *pixel = image::Rgba([255, 0, 0, 0]);
        }
        pixels.save(&source).unwrap();

        let first = thumbnail(root, &source, "logo.png", 64).unwrap();
        assert_eq!(first.mime, "image/png");
        let decoded = image::load_from_memory(&first.bytes).unwrap();
        assert!(decoded.color().has_alpha());
        assert_eq!(
            decoded.to_rgba8().get_pixel(0, 0)[3],
            0,
            "still transparent"
        );

        // The cache answers with the same PNG.
        let second = thumbnail(root, &source, "logo.png", 64).unwrap();
        assert_eq!(second.mime, "image/png");
        assert_eq!(second.bytes, first.bytes);
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

    fn empty_listing() -> MediaListing {
        MediaListing {
            entries: Vec::new(),
            truncated: false,
        }
    }

    #[test]
    fn cache_holds_the_listing_until_a_change_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("one.png"), b"x").unwrap();

        let cache = MediaCache::default();
        let first = cache.ensure(root).unwrap();
        assert_eq!(first.entries.len(), 1);
        // A hit reuses the same listing: no second walk, no copy.
        assert!(Arc::ptr_eq(&first, &cache.ensure(root).unwrap()));

        // A write alone is invisible: only an invalidation makes it walk again.
        fs::write(root.join("two.png"), b"x").unwrap();
        assert_eq!(cache.ensure(root).unwrap().entries.len(), 1);

        cache.invalidate();
        let second = cache.ensure(root).unwrap();
        assert_eq!(second.entries.len(), 2);
        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn a_change_during_the_walk_leaves_the_snapshot_stale() {
        let cache = MediaCache::default();
        let walks = std::cell::Cell::new(0usize);
        let mut walk = || {
            walks.set(walks.get() + 1);
            if walks.get() == 1 {
                // The tree moves while the first walk is still running.
                cache.invalidate();
            }
            Ok(empty_listing())
        };

        let first = cache.ensure_with(&mut walk).unwrap();
        let second = cache.ensure_with(&mut walk).unwrap();
        assert_eq!(walks.get(), 2, "a mid-walk change must force a re-walk");
        assert!(!Arc::ptr_eq(&first, &second));

        // Settled: no further change, so the fresh snapshot holds.
        let third = cache.ensure_with(&mut walk).unwrap();
        assert_eq!(walks.get(), 2);
        assert!(Arc::ptr_eq(&second, &third));
    }

    #[test]
    fn a_named_change_patches_the_snapshot_without_walking() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.png"), b"x").unwrap();
        let cache = MediaCache::default();
        let walks = std::cell::Cell::new(0usize);
        let mut walk = || {
            walks.set(walks.get() + 1);
            list_media(root)
        };

        let first = cache.ensure_with(&mut walk).unwrap();
        assert_eq!(first.entries.len(), 1);

        std::fs::rename(root.join("a.png"), root.join("b.png")).unwrap();
        assert!(cache.apply(root, &["a.png".to_string(), "b.png".to_string()]));

        let second = cache.ensure_with(&mut walk).unwrap();
        assert_eq!(walks.get(), 1, "a named change answers without a walk");
        assert_eq!(second.entries.len(), 1);
        assert_eq!(second.entries[0].path, "b.png");
    }

    #[test]
    fn a_change_that_names_no_media_keeps_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.png"), b"x").unwrap();
        let cache = MediaCache::default();
        let walks = std::cell::Cell::new(0usize);
        let mut walk = || {
            walks.set(walks.get() + 1);
            list_media(root)
        };
        let first = cache.ensure_with(&mut walk).unwrap();

        std::fs::write(root.join("notes.txt"), b"hi").unwrap();
        assert!(cache.apply(root, &["notes.txt".to_string()]));

        let second = cache.ensure_with(&mut walk).unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "nothing moved, so nothing is rebuilt"
        );
        assert_eq!(walks.get(), 1);
    }

    #[test]
    fn a_directory_change_leaves_the_snapshot_stale() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("clips")).unwrap();
        let cache = MediaCache::default();
        let walks = std::cell::Cell::new(0usize);
        let mut walk = || {
            walks.set(walks.get() + 1);
            list_media(root)
        };
        let _ = cache.ensure_with(&mut walk).unwrap();

        // A folder's contents are not knowable from its name.
        assert!(!cache.apply(root, &["clips".to_string()]));

        let _ = cache.ensure_with(&mut walk).unwrap();
        assert_eq!(walks.get(), 2);
    }

    #[test]
    fn extension_ignores_directories() {
        assert_eq!(extension("a.b/c"), "");
        assert_eq!(extension("a/b.PNG"), "PNG");
        assert_eq!(extension("plain"), "");
    }
}
