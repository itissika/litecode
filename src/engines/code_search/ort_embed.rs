//! Production ONNX Runtime (ORT) granite97 embedder — CPU WOQ cold artifact.
//!
//! Locked product path: `artifacts/ort-lin-q8-emb-q4-bs128-a1.onnx`
//! (MatMulNBits Q8 + GatherBlockQuantized Q4). CLS pooling + L2.
//!
//! Session lives in the code-search worker process only (not the agent main process).

use std::path::{Path, PathBuf};

use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use crate::types::{LitecodeError, Result};

use super::{EMBED_DIM, EMBED_INDEX_BATCH, EMBED_MAX_LENGTH, EMBEDDER_ID_ORT_Q8Q4};

/// Pareto WOQ cold artifact relative to the HF model directory.
pub const ONNX_Q8Q4_REL: &str = "artifacts/ort-lin-q8-emb-q4-bs128-a1.onnx";
/// Legacy RTN path accepted only if Pareto missing.
const ONNX_Q8Q4_LEGACY_REL: &str = "artifacts/ort-lin-q8-emb-q4.onnx";

pub struct OrtGraniteEmbedder {
    session: ort::session::Session,
    tokenizer: Tokenizer,
    ort_batch: usize,
    onnx_path: PathBuf,
}

impl OrtGraniteEmbedder {
    pub fn try_open(model_dir: &Path) -> Result<Self> {
        let onnx_path = resolve_onnx_path(model_dir)?;
        let tokenizer_path = model_dir.join("tokenizer.json");
        if !tokenizer_path.is_file() {
            return Err(LitecodeError::Config(format!(
                "tokenizer missing at {} — place official IBM HF files under {}",
                tokenizer_path.display(),
                model_dir.display()
            )));
        }

        let knobs = OrtSessionKnobs::from_env();
        knobs.log_applied();

        let intra_threads = ort_intra_threads();
        let mut builder = ort::session::Session::builder()
            .map_err(|e| LitecodeError::Config(format!("ort Session::builder: {e}")))?
            .with_intra_threads(intra_threads)
            .map_err(|e| LitecodeError::Config(format!("ort with_intra_threads: {e}")))?;

        builder = knobs.apply_session(builder)?;
        builder = knobs.apply_cpu_ep(builder)?;

        let session = builder.commit_from_file(&onnx_path).map_err(|e| {
            LitecodeError::Config(format!("ort commit_from_file {}: {e}", onnx_path.display()))
        })?;

        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| LitecodeError::Config(format!("tokenizer load: {e}")))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: EMBED_MAX_LENGTH,
                stride: 0,
                ..Default::default()
            }))
            .map_err(|e| LitecodeError::Config(format!("tokenizer truncation: {e}")))?;
        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            ..Default::default()
        }));

        tracing::info!(
            embedder = EMBEDDER_ID_ORT_Q8Q4,
            path = %onnx_path.display(),
            max_seq = EMBED_MAX_LENGTH,
            "opening ORT CPU WOQ embedder"
        );

        Ok(Self {
            session,
            tokenizer,
            ort_batch: ort_embed_batch(),
            onnx_path,
        })
    }

    fn embed_texts(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| LitecodeError::Config(format!("tokenize: {e}")))?;

        let token_count = encodings[0].len();
        let batch = encodings.len();
        let mut ids_flat = Vec::with_capacity(batch * token_count);
        let mut mask_flat = Vec::with_capacity(batch * token_count);
        for enc in &encodings {
            ids_flat.extend(enc.get_ids().iter().map(|&x| x as i64));
            mask_flat.extend(enc.get_attention_mask().iter().map(|&x| x as i64));
        }

        let ids_array = ndarray::Array2::from_shape_vec((batch, token_count), ids_flat)
            .map_err(|e| LitecodeError::Config(format!("reshape input_ids: {e}")))?;
        let mask_array = ndarray::Array2::from_shape_vec((batch, token_count), mask_flat)
            .map_err(|e| LitecodeError::Config(format!("reshape attention_mask: {e}")))?;

        let ids_tensor = ort::value::Tensor::<i64>::from_array(ids_array)
            .map_err(|e| LitecodeError::Config(format!("ort tensor input_ids: {e}")))?;
        let mask_tensor = ort::value::Tensor::<i64>::from_array(mask_array)
            .map_err(|e| LitecodeError::Config(format!("ort tensor attention_mask: {e}")))?;

        let outputs = self
            .session
            .run(ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => mask_tensor,
            ])
            .map_err(|e| LitecodeError::Config(format!("ort session.run: {e}")))?;

        let hidden = outputs["last_hidden_state"]
            .try_extract_array::<f32>()
            .map_err(|e| LitecodeError::Config(format!("extract last_hidden_state: {e}")))?;

        let mut out = Vec::with_capacity(batch);
        for b in 0..batch {
            let mut vec: Vec<f32> = hidden
                .slice(ndarray::s![b, 0, ..])
                .iter()
                .copied()
                .collect();
            if vec.len() != EMBED_DIM {
                return Err(LitecodeError::Config(format!(
                    "unexpected embed dim {} (want {EMBED_DIM}) from {}",
                    vec.len(),
                    self.onnx_path.display()
                )));
            }
            normalize_l2(&mut vec);
            out.push(vec);
        }
        Ok(out)
    }

    pub fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let micro = self.ort_batch.max(1);
        let mut all = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(micro) {
            all.extend(self.embed_texts(chunk)?);
        }
        Ok(all)
    }
}

fn normalize_l2(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vec {
            *v /= norm;
        }
    }
}

fn resolve_onnx_path(model_dir: &Path) -> Result<PathBuf> {
    if let Ok(raw) = std::env::var("LITECODE_ORT_ONNX") {
        let path = PathBuf::from(raw.trim());
        if path.is_file() {
            return Ok(path);
        }
        return Err(LitecodeError::Config(format!(
            "LITECODE_ORT_ONNX set but not a file: {}",
            path.display()
        )));
    }

    let art = model_dir.join(ONNX_Q8Q4_REL);
    if art.is_file() {
        return Ok(art);
    }
    let legacy = model_dir.join(ONNX_Q8Q4_LEGACY_REL);
    if legacy.is_file() {
        tracing::warn!(
            missing = %art.display(),
            fallback = %legacy.display(),
            "Pareto ORT artifact missing; using legacy WOQ"
        );
        return Ok(legacy);
    }
    Err(LitecodeError::Config(format!(
        "ORT WOQ artifact missing at {} — place {} under the model dir (or set LITECODE_ORT_ONNX)",
        art.display(),
        ONNX_Q8Q4_REL
    )))
}

fn ort_intra_threads() -> usize {
    if let Ok(raw) = std::env::var("LITECODE_ORT_INTRA_THREADS")
        && let Ok(n) = raw.trim().parse::<usize>()
        && n > 0
    {
        return n;
    }
    std::thread::available_parallelism()
        .map(|n| n.get().min(8).max(1))
        .unwrap_or(4)
}

fn ort_embed_batch() -> usize {
    if let Ok(raw) = std::env::var("LITECODE_ORT_BATCH")
        && let Ok(n) = raw.trim().parse::<usize>()
        && n > 0
    {
        return n;
    }
    EMBED_INDEX_BATCH
}

fn env_flag_nonzero(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty() && v != "0")
}

/// Optional Session knobs (default = baseline). See `docs/lib-docs/ort/MEMORY.md`.
struct OrtSessionKnobs {
    lean: bool,
    cpu_arena: bool,
    no_spin: bool,
    mem_pattern: Option<bool>,
}

impl OrtSessionKnobs {
    fn from_env() -> Self {
        let lean = env_flag_nonzero("LITECODE_ORT_LEAN");
        let cpu_arena = if lean {
            std::env::var("LITECODE_ORT_CPU_ARENA").ok().as_deref() == Some("1")
        } else if let Ok(v) = std::env::var("LITECODE_ORT_CPU_ARENA") {
            v != "0" && !v.is_empty()
        } else {
            true
        };
        let no_spin = lean || env_flag_nonzero("LITECODE_ORT_NO_SPIN");
        let mem_pattern = if let Ok(v) = std::env::var("LITECODE_ORT_MEM_PATTERN") {
            Some(v != "0" && !v.is_empty())
        } else {
            None
        };
        Self {
            lean,
            cpu_arena,
            no_spin,
            mem_pattern,
        }
    }

    fn log_applied(&self) {
        tracing::debug!(
            lean = self.lean,
            cpu_arena = self.cpu_arena,
            no_spin = self.no_spin,
            mem_pattern = ?self.mem_pattern,
            "ort session knobs"
        );
    }

    fn apply_session(
        &self,
        mut builder: ort::session::builder::SessionBuilder,
    ) -> Result<ort::session::builder::SessionBuilder> {
        if let Some(enable) = self.mem_pattern {
            builder = builder
                .with_memory_pattern(enable)
                .map_err(|e| LitecodeError::Config(format!("with_memory_pattern: {e}")))?;
        }
        if self.no_spin {
            builder = builder
                .with_config_entry("session.intra_op.allow_spinning", "0")
                .map_err(|e| LitecodeError::Config(format!("allow_spinning intra: {e}")))?;
            builder = builder
                .with_config_entry("session.inter_op.allow_spinning", "0")
                .map_err(|e| LitecodeError::Config(format!("allow_spinning inter: {e}")))?;
        }
        Ok(builder)
    }

    fn apply_cpu_ep(
        &self,
        builder: ort::session::builder::SessionBuilder,
    ) -> Result<ort::session::builder::SessionBuilder> {
        use ort::ep::CPU;
        builder
            .with_execution_providers([CPU::default().with_arena_allocator(self.cpu_arena).build()])
            .map_err(|e| LitecodeError::Config(format!("CPU EP register: {e}")))
    }
}
