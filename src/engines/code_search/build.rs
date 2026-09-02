//! Full and incremental index construction.

use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

use crate::types::{LitecodeError, Result};
use crate::workspace::filter::{FilterPreset, RelPathCtx, is_indexable_rel_path, walk_builder};

use super::chunk::{ChunkRecord, chunk_file};
use super::embed::Embedder;
use super::index_status::{
    IndexPhase, IndexingProgress, begin_building, clear_index_job, update_build_progress,
};
use super::store::CodeSearchIndex;
use super::{EMBED_BATCH, index_dir};

pub fn build_full_index(
    workspace_root: &Path,
    embedder: &mut dyn Embedder,
) -> Result<CodeSearchIndex> {
    std::fs::create_dir_all(index_dir(workspace_root))
        .map_err(|e| LitecodeError::Config(e.to_string()))?;

    begin_building(workspace_root);
    let files = match scannable_files(workspace_root) {
        Ok(f) => f,
        Err(e) => {
            clear_index_job(workspace_root);
            return Err(e);
        }
    };
    let t0 = Instant::now();
    log_line(&format!(
        "[build_full_index] start embedder={} files={} embed_batch={EMBED_BATCH}",
        embedder.embedder_id(),
        files.len()
    ));
    tracing::info!(
        embedder = embedder.embedder_id(),
        files = files.len(),
        embed_batch = EMBED_BATCH,
        "build_full_index: scanning done, chunking + embedding"
    );
    update_build_progress(
        workspace_root,
        IndexingProgress {
            phase: IndexPhase::Embedding,
            files_done: 0,
            files_total: files.len(),
            chunks_done: 0,
        },
    );

    let mut index = CodeSearchIndex::new_empty()?;
    index.set_embedder_id(embedder.embedder_id());
    let mut next_id = 1u64;
    let mut pending_chunks: Vec<ChunkRecord> = Vec::new();
    let mut embedded = 0usize;
    let mut files_done = 0usize;
    let mut batch_i = 0usize;

    for rel in &files {
        let abs = workspace_root.join(rel);
        let content = match std::fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let (chunks, new_id) = chunk_file(rel, &content, next_id);
        next_id = new_id;
        pending_chunks.extend(chunks);
        files_done += 1;
        while pending_chunks.len() >= EMBED_BATCH {
            batch_i += 1;
            let batch: Vec<ChunkRecord> = pending_chunks.drain(..EMBED_BATCH).collect();
            let n = batch.len();
            let batch_t0 = Instant::now();
            if let Err(e) = index.embed_and_add(batch, embedder) {
                clear_index_job(workspace_root);
                return Err(e);
            }
            let batch_ms = batch_t0.elapsed().as_secs_f64() * 1000.0;
            embedded += n;
            log_progress(
                workspace_root,
                t0,
                embedded,
                files_done,
                files.len(),
                batch_i,
                n,
                batch_ms,
            );
        }
    }
    if !pending_chunks.is_empty() {
        batch_i += 1;
        let n = pending_chunks.len();
        let batch_t0 = Instant::now();
        if let Err(e) = flush_batch(&mut index, &mut pending_chunks, embedder) {
            clear_index_job(workspace_root);
            return Err(e);
        }
        let batch_ms = batch_t0.elapsed().as_secs_f64() * 1000.0;
        embedded += n;
        log_progress(
            workspace_root,
            t0,
            embedded,
            files_done,
            files.len(),
            batch_i,
            n,
            batch_ms,
        );
    }
    index.set_next_id(next_id);

    update_build_progress(
        workspace_root,
        IndexingProgress {
            phase: IndexPhase::Saving,
            files_done,
            files_total: files.len(),
            chunks_done: embedded,
        },
    );

    let secs = t0.elapsed().as_secs_f64().max(0.001);
    let rate = embedded as f64 / secs;
    log_line(&format!(
        "[build_full_index] done embedder={} files={files_done} chunks={embedded} \
         elapsed={secs:.1}s rate={rate:.2} chunks/s",
        embedder.embedder_id()
    ));
    tracing::info!(
        embedder = embedder.embedder_id(),
        files = files_done,
        chunks = embedded,
        elapsed_secs = format!("{secs:.1}"),
        chunks_per_sec = format!("{rate:.2}"),
        "build_full_index: done"
    );
    // Caller saves + clear_index_job on success; leave job until saved.
    Ok(index)
}

fn log_progress(
    workspace_root: &Path,
    t0: Instant,
    embedded: usize,
    files_done: usize,
    files_total: usize,
    batch_i: usize,
    batch_n: usize,
    batch_ms: f64,
) {
    update_build_progress(
        workspace_root,
        IndexingProgress {
            phase: IndexPhase::Embedding,
            files_done,
            files_total,
            chunks_done: embedded,
        },
    );
    let secs = t0.elapsed().as_secs_f64().max(0.001);
    let rate = embedded as f64 / secs;
    let eta = if rate > 0.01 && files_done > 0 {
        let remain = files_total.saturating_sub(files_done) as f64;
        let secs_per_file = secs / files_done as f64;
        format!("{:.0}s", remain * secs_per_file)
    } else {
        "?".into()
    };
    let batch_cps = if batch_ms > 0.0 {
        batch_n as f64 / (batch_ms / 1000.0)
    } else {
        0.0
    };
    log_line(&format!(
        "[build_full_index] progress batch={batch_i} +{batch_n} in {batch_ms:.0}ms \
         ({batch_cps:.2} chunks/s this batch) | total_chunks={embedded} \
         files={files_done}/{files_total} elapsed={secs:.1}s avg={rate:.2} chunks/s eta≈{eta}"
    ));
    tracing::info!(
        batch_i,
        batch_n,
        batch_ms = format!("{batch_ms:.0}"),
        embedded_chunks = embedded,
        files_done,
        files_total,
        avg_chunks_per_sec = format!("{rate:.2}"),
        "build_full_index: progress"
    );
}

fn log_line(msg: &str) {
    let _ = writeln!(io::stderr(), "{msg}");
    let _ = io::stderr().flush();
}

fn flush_batch(
    index: &mut CodeSearchIndex,
    pending: &mut Vec<ChunkRecord>,
    embedder: &mut dyn Embedder,
) -> Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    let batch: Vec<ChunkRecord> = std::mem::take(pending);
    index.embed_and_add(batch, embedder)
}

pub fn scannable_files(workspace_root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let rel_ctx =
        RelPathCtx::new(workspace_root).unwrap_or_else(|_| RelPathCtx::new_lossy(workspace_root));
    let walker = walk_builder(workspace_root, FilterPreset::Search);
    for result in walker.build() {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let Some(rel_str) = rel_ctx.rel(path) else {
            continue;
        };
        if is_indexable_rel_path(&rel_str, workspace_root) {
            out.push(rel_str);
        }
    }
    out.sort_unstable();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::embed::HashEmbedder;
    use crate::workspace::filter::is_scannable_rel_path;
    use tempfile::TempDir;

    #[test]
    fn build_index_skips_non_scannable() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("ok.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("skip.bin"), b"\0\0\0").unwrap();
        let mut emb = HashEmbedder;
        let index = build_full_index(root, &mut emb).unwrap();
        assert!(index.chunks().values().any(|c| c.path == "ok.rs"));
    }

    #[test]
    fn scannable_respects_policy() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "x\n").unwrap();
        std::fs::write(root.join("LICENSE"), "MIT\n").unwrap();
        std::fs::write(root.join("serve.ps1"), "Write-Host hi\n").unwrap();
        let files = scannable_files(root).unwrap();
        assert!(files.iter().any(|f| f == "a.rs"));
        assert!(files.iter().any(|f| f == "LICENSE"));
        assert!(files.iter().any(|f| f == "serve.ps1"));
        assert!(is_scannable_rel_path("a.rs"));
    }

    #[test]
    fn embedding_corpus_honors_index_gitignore_switch() {
        // The real embedding input path: walk(Index) + content gates. Toggling
        // the search-side gitignore switch must add/drop gitignored files from
        // the corpus, without a full reindex fingerprint.
        use crate::workspace::filter::{WorkspaceExcludesFile, with_excludes_cache_for_test};

        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // The ignore crate only honors .gitignore with a git repo marker present.
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "ref_vendor.rs\n").unwrap();
        std::fs::write(root.join("ref_vendor.rs"), "fn r() {}\n").unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();

        // Default: git_ignore=true → the pulled reference repo stays out.
        with_excludes_cache_for_test(WorkspaceExcludesFile::builtin_defaults(), || {
            let files = scannable_files(root).unwrap();
            assert!(files.iter().any(|f| f == "main.rs"), "{files:?}");
            assert!(
                !files.iter().any(|f| f == "ref_vendor.rs"),
                "gitignored file must not enter the embedding corpus: {files:?}"
            );
        });

        // git_ignore=false → the ignored file joins the corpus.
        with_excludes_cache_for_test(
            WorkspaceExcludesFile {
                git_ignore: false,
                ..WorkspaceExcludesFile::builtin_defaults()
            },
            || {
                let files = scannable_files(root).unwrap();
                assert!(
                    files.iter().any(|f| f == "ref_vendor.rs"),
                    "git_ignore=false must admit the file: {files:?}"
                );
            },
        );
    }
}
