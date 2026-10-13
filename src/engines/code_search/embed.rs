//! Embedding backends: ORT CPU WOQ (production) and hash (tests / CI).
//!
//! Model layout under [`model_dir()`]: official HF `config.json` + `tokenizer.json`
//! plus self-built cold artifact `artifacts/ort-lin-q8-emb-q4-bs128-a1.onnx`.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::types::{LitecodeError, Result};

use super::ort_embed::{self, OrtGraniteEmbedder};
use super::{EMBED_DIM, EMBEDDER_ID_ORT_Q8Q4};

/// Product default embedder id (ORT CPU WOQ Pareto).
pub const EMBEDDER_ID_PASS: &str = EMBEDDER_ID_ORT_Q8Q4;
/// Legacy alias kept for callers that still reference the old symbol name.
pub const EMBEDDER_ID_GRANITE97Q: &str = EMBEDDER_ID_ORT_Q8Q4;
pub const EMBEDDER_ID_HASH: &str = "hash";

const HF_TOKENIZER: &str = "tokenizer.json";
const HF_CONFIG: &str = "config.json";

/// Product-bundled HF layout under `{product_root}/models/`.
pub const BUNDLED_MODEL_REL: &str = "ibm-granite/granite-embedding-97m-multilingual-r2";

#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingModelStatus {
    pub model_dir: String,
    /// Path to the ORT WOQ ONNX artifact when present.
    pub model_path: String,
    pub tokenizer_path: String,
    pub model_found: bool,
    pub tokenizer_found: bool,
    pub ready: bool,
}

pub trait Embedder: Send {
    fn embedder_id(&self) -> &'static str;
    /// Live inference device (`cuda-ort`, `cpu-ort`, `hash`). Captured when the
    /// ORT session (or hash stub) opens — not a compile-time feature flag.
    fn inference_device(&self) -> &'static str {
        "cpu-ort"
    }
    fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn embed_one(&mut self, text: &str) -> Result<Vec<f32>> {
        self.embed_batch(&[text.to_string()])
            .map(|mut v| v.pop().unwrap_or_default())
    }
}

/// Deterministic unit-norm vectors for tests without loading models.
pub struct HashEmbedder;

impl Embedder for HashEmbedder {
    fn embedder_id(&self) -> &'static str {
        EMBEDDER_ID_HASH
    }

    fn inference_device(&self) -> &'static str {
        "hash"
    }

    fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| hash_vector(t)).collect())
    }
}

pub fn hash_vector(text: &str) -> Vec<f32> {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    let mut vec = vec![0f32; EMBED_DIM];
    for (i, v) in vec.iter_mut().enumerate() {
        *v = digest[i % digest.len()] as f32 / 255.0;
    }
    normalize(&mut vec);
    vec
}

pub(crate) fn normalize(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vec {
            *v /= norm;
        }
    }
}

/// Directory containing official Granite HF files (+ `artifacts/` WOQ ONNX).
///
/// Resolution order (same as the desktop host's `resolveBundledModelDir`):
/// 1. `LITECODE_MODEL_DIR` environment variable (override only).
/// 2. `LITECODE_BUNDLE_ROOT` / per-user bundles dir (`%LOCALAPPDATA%\litecode\bundles`,
///    `$XDG_DATA_HOME/litecode/bundles`), where the slim desktop installer downloads
///    the model on demand. Re-checked on every call, so a model installed while the
///    sidecar is running is picked up on the next engine start.
/// 3. Bundled `models/{[`BUNDLED_MODEL_REL`]}` next to the executable (walk up from exe).
/// 4. Same bundled path under the compile-time crate root (`cargo run` / `cargo test`).
pub fn model_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("LITECODE_MODEL_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            let resolved = resolve_hf_layout(&p);
            if is_model_dir_ready(&resolved) {
                return Ok(resolved);
            }
            return Err(LitecodeError::Config(format!(
                "LITECODE_MODEL_DIR={} is missing tokenizer, config, or {}",
                p.display(),
                ort_embed::ONNX_Q8Q4_REL
            )));
        }
    }

    if let Some(dir) = find_downloaded_model_dir() {
        return Ok(dir);
    }

    if let Some(dir) = find_bundled_model_dir() {
        return Ok(dir);
    }

    let expected = litecode_bundles_dir()
        .map(|root| bundled_model_path(&root))
        .unwrap_or_else(|| bundled_model_path(Path::new("<product_root>")));
    Err(LitecodeError::Config(format!(
        "embed model not installed (expected at {}). In the desktop app open \
         Settings \u{2192} Engines and download it; for dev builds run \
         scripts/bundle_embed_model.sh or set LITECODE_MODEL_DIR",
        expected.display()
    )))
}

/// Per-user bundles directory shared with the desktop host (`litecodeBundlesDir`).
fn litecode_bundles_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .map(|h| PathBuf::from(h).join("AppData").join("Local"))
            })?;
        Some(local.join("litecode").join("bundles"))
    }
    #[cfg(not(windows))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })?;
        Some(base.join("litecode").join("bundles"))
    }
}

/// Model under a bundle root, accepting the HF dir, `models/` root or bundle root.
fn model_in_bundle_root(root: &Path) -> Option<PathBuf> {
    [
        root.to_path_buf(),
        root.join(BUNDLED_MODEL_REL),
        bundled_model_path(root),
    ]
    .into_iter()
    .find(|candidate| is_model_dir_ready(candidate))
}

fn find_downloaded_model_dir() -> Option<PathBuf> {
    let explicit = std::env::var_os("LITECODE_BUNDLE_ROOT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    explicit
        .into_iter()
        .chain(litecode_bundles_dir())
        .find_map(|root| model_in_bundle_root(&root))
        .map(|dir| {
            crate::config::path::os_probe_abs(&dir)
                .unwrap_or_else(|_| crate::config::path::canon_abs_lossy(&dir))
        })
}

fn find_bundled_model_dir() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent()?.to_path_buf();
        loop {
            let candidate = bundled_model_path(&dir);
            if is_model_dir_ready(&candidate) {
                return Some(
                    crate::config::path::os_probe_abs(&candidate)
                        .unwrap_or_else(|_| crate::config::path::canon_abs_lossy(&candidate)),
                );
            }
            match dir.parent() {
                Some(parent) if parent != dir => dir = parent.to_path_buf(),
                _ => break,
            }
        }
    }

    let candidate = source_tree_model_path()?;
    if is_model_dir_ready(&candidate) {
        return Some(
            crate::config::path::os_probe_abs(&candidate)
                .unwrap_or_else(|_| crate::config::path::canon_abs_lossy(&candidate)),
        );
    }
    None
}

/// Dev convenience: `cargo run` / `cargo test` from a checkout finds
/// `<repo>/models/...`. Debug builds only: a release binary must not carry the
/// build machine's source path, let alone probe it on a user's disk.
#[cfg(debug_assertions)]
fn source_tree_model_path() -> Option<PathBuf> {
    Some(bundled_model_path(Path::new(env!("CARGO_MANIFEST_DIR"))))
}

#[cfg(not(debug_assertions))]
fn source_tree_model_path() -> Option<PathBuf> {
    None
}

fn bundled_model_path(product_root: &Path) -> PathBuf {
    product_root.join("models").join(BUNDLED_MODEL_REL)
}

/// Accept explicit HF dir or `models/` root with nested bundled layout.
fn resolve_hf_layout(root: &Path) -> PathBuf {
    if is_model_dir_ready(root) {
        return root.to_path_buf();
    }
    let nested = root.join(BUNDLED_MODEL_REL);
    if is_model_dir_ready(&nested) {
        return nested;
    }
    root.to_path_buf()
}

fn is_model_dir_ready(dir: &Path) -> bool {
    dir.join(HF_TOKENIZER).is_file()
        && dir.join(HF_CONFIG).is_file()
        && (dir.join(ort_embed::ONNX_Q8Q4_REL).is_file()
            || dir.join("artifacts/ort-lin-q8-emb-q4.onnx").is_file())
}

pub fn probe_embedding_model() -> Result<EmbeddingModelStatus> {
    let dir = model_dir()?;
    let tokenizer = dir.join(HF_TOKENIZER);
    let config = dir.join(HF_CONFIG);
    let onnx = dir.join(ort_embed::ONNX_Q8Q4_REL);
    let legacy = dir.join("artifacts/ort-lin-q8-emb-q4.onnx");
    let tokenizer_found = tokenizer.is_file();
    let config_found = config.is_file();
    let onnx_found = onnx.is_file() || legacy.is_file();
    let model_path = if onnx.is_file() {
        onnx
    } else if legacy.is_file() {
        legacy
    } else {
        PathBuf::new()
    };
    let ready = tokenizer_found && config_found && onnx_found;
    Ok(EmbeddingModelStatus {
        model_dir: dir.display().to_string(),
        model_path: model_path.display().to_string(),
        tokenizer_path: tokenizer.display().to_string(),
        model_found: onnx_found,
        tokenizer_found,
        ready,
    })
}

impl Embedder for OrtGraniteEmbedder {
    fn embedder_id(&self) -> &'static str {
        EMBEDDER_ID_ORT_Q8Q4
    }

    fn inference_device(&self) -> &'static str {
        self.device
    }

    fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        OrtGraniteEmbedder::embed_batch(self, texts)
    }
}

fn ensure_official_cls_pooling(model_dir: &Path) -> Result<()> {
    let p = model_dir.join("1_Pooling").join("config.json");
    if !p.is_file() {
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&p)
            .map_err(|e| LitecodeError::Config(format!("read {}: {e}", p.display())))?,
    )
    .map_err(|e| LitecodeError::Config(format!("parse {}: {e}", p.display())))?;
    let cls = v
        .get("pooling_mode_cls_token")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    if !cls {
        return Err(LitecodeError::Config(format!(
            "official pooling config {} requires pooling_mode_cls_token=true",
            p.display()
        )));
    }
    Ok(())
}

pub fn production_embedder_id() -> &'static str {
    if use_hash_embedder() {
        EMBEDDER_ID_HASH
    } else {
        EMBEDDER_ID_ORT_Q8Q4
    }
}

pub fn open_production_embedder() -> Result<Box<dyn Embedder>> {
    if use_hash_embedder() {
        return Ok(Box::new(HashEmbedder));
    }
    let dir = model_dir()?;
    ensure_official_cls_pooling(&dir)?;
    OrtGraniteEmbedder::try_open(&dir).map(|e| Box::new(e) as Box<dyn Embedder>)
}

fn use_hash_embedder() -> bool {
    cfg!(test)
        || std::env::var("LITECODE_CODE_SEARCH_USE_HASH")
            .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_root_accepts_models_layout_and_rejects_incomplete() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(model_in_bundle_root(tmp.path()).is_none());
        let dir = bundled_model_path(tmp.path());
        std::fs::create_dir_all(dir.join("artifacts")).unwrap();
        std::fs::write(dir.join(HF_TOKENIZER), "{}").unwrap();
        std::fs::write(dir.join(HF_CONFIG), "{}").unwrap();
        assert!(model_in_bundle_root(tmp.path()).is_none());
        std::fs::write(dir.join(ort_embed::ONNX_Q8Q4_REL), "onnx").unwrap();
        assert_eq!(model_in_bundle_root(tmp.path()), Some(dir.clone()));
        assert_eq!(model_in_bundle_root(&dir), Some(dir));
    }

    #[test]
    fn source_tree_fallback_is_debug_only() {
        let fallback = source_tree_model_path();
        if cfg!(debug_assertions) {
            let path = fallback.expect("debug builds keep the checkout fallback");
            assert!(path.starts_with(env!("CARGO_MANIFEST_DIR")));
        } else {
            assert!(
                fallback.is_none(),
                "release must not probe the build machine's repo"
            );
        }
    }

    #[test]
    fn hash_embedder_produces_unit_vectors() {
        let mut emb = HashEmbedder;
        let v = emb.embed_one("hello world").unwrap();
        assert_eq!(v.len(), EMBED_DIM);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }
}
