//! Full and incremental index construction.

use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

use crate::types::{LitecodeError, Result};
use crate::workspace::filter::{
    FilterPreset, MAX_INDEX_FILE_BYTES, RelPathCtx, WalkOptions, is_indexable_rel_path,
    is_scannable_rel_path, walk_builder, walk_builder_with,
};

use super::chunk::{ChunkRecord, chunk_file};
use super::embed::Embedder;
use super::index_status::{
    IndexPhase, IndexingProgress, begin_building, mark_index_job_failed, update_build_progress,
    update_chunk_progress,
};
use super::store::CodeSearchIndex;
use super::{EMBED_BATCH, index_dir};

/// Pass-1 in-memory budget for chunk text before falling back to re-chunking.
const PASS1_HOLD_BYTES: usize = 256 * 1024 * 1024;

pub fn build_full_index(
    workspace_root: &Path,
    embedder: &mut dyn Embedder,
) -> Result<CodeSearchIndex> {
    std::fs::create_dir_all(index_dir(workspace_root))
        .map_err(|e| LitecodeError::Config(e.to_string()))?;

    begin_building(workspace_root);
    // WP-141: stamps describe the index being replaced; drop them up front.
    super::stamps::discard(workspace_root);
    let files = match scannable_files(workspace_root) {
        Ok(f) => f,
        Err(e) => {
            mark_index_job_failed(workspace_root, e.to_string());
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

    // Pass 1 (WP-131): chunk everything so the denominator is exact before
    // embedding starts. Keep chunks in memory unless they exceed the budget,
    // then fall back to counting only and re-chunk in pass 2 (ids are
    // deterministic, so the total stays exact).
    let mut next_id = 1u64;
    let mut held: Option<Vec<ChunkRecord>> = Some(Vec::new());
    let mut held_bytes = 0usize;
    let mut total_chunks = 0u64;
    // WP-141: stamps are taken before each read, so a file that changes
    // during the build carries an older stamp and is re-checked next pass.
    let mut build_stamps = std::collections::HashMap::new();
    for (i, rel) in files.iter().enumerate() {
        let stamp = super::reconcile::read_stamp(&workspace_root.join(rel));
        let Ok(content) = std::fs::read_to_string(workspace_root.join(rel)) else {
            continue;
        };
        if let Some(stamp) = stamp {
            build_stamps.insert(rel.clone(), stamp);
        }
        let (chunks, new_id) = chunk_file(rel, &content, next_id);
        next_id = new_id;
        total_chunks += chunks.len() as u64;
        if let Some(h) = held.as_mut() {
            held_bytes += chunks.iter().map(|c| c.text.len()).sum::<usize>();
            if held_bytes > PASS1_HOLD_BYTES {
                held = None;
            } else {
                h.extend(chunks);
            }
        }
        if i % 256 == 0 {
            update_chunk_progress(workspace_root, 0, None);
        }
    }
    let final_next_id = next_id;
    update_chunk_progress(workspace_root, 0, Some(total_chunks));

    // Pass 2: embed in EMBED_BATCH batches, reporting after every batch.
    let mut pending_chunks: Vec<ChunkRecord> = Vec::new();
    let mut embedded = 0usize;
    let mut files_done = 0usize;
    let mut batch_i = 0usize;
    let mut source: Box<dyn Iterator<Item = Vec<ChunkRecord>>> = match held {
        Some(all) => {
            files_done = files.len();
            Box::new(std::iter::once(all))
        }
        None => {
            let mut id = 1u64;
            let root = workspace_root.to_path_buf();
            Box::new(files.clone().into_iter().filter_map(move |rel| {
                let content = std::fs::read_to_string(root.join(&rel)).ok()?;
                let (chunks, new_id) = chunk_file(&rel, &content, id);
                id = new_id;
                Some(chunks)
            }))
        }
    };
    let held_mode = files_done == files.len();
    let flush = |index: &mut CodeSearchIndex,
                 batch: &mut Vec<ChunkRecord>,
                 embedded: &mut usize,
                 batch_i: &mut usize,
                 files_done: usize,
                 embedder: &mut dyn Embedder|
     -> Result<()> {
        *batch_i += 1;
        let n = batch.len();
        let batch_t0 = Instant::now();
        flush_batch(index, batch, embedder)?;
        *embedded += n;
        update_chunk_progress(workspace_root, *embedded as u64, Some(total_chunks));
        log_progress(
            workspace_root,
            t0,
            *embedded,
            files_done,
            files.len(),
            *batch_i,
            n,
            batch_t0.elapsed().as_secs_f64() * 1000.0,
        );
        Ok(())
    };
    while let Some(chunks) = source.next() {
        if !held_mode {
            files_done += 1;
        }
        pending_chunks.extend(chunks);
        while pending_chunks.len() >= EMBED_BATCH {
            let mut batch: Vec<ChunkRecord> = pending_chunks.drain(..EMBED_BATCH).collect();
            if let Err(e) = flush(
                &mut index,
                &mut batch,
                &mut embedded,
                &mut batch_i,
                files_done,
                embedder,
            ) {
                mark_index_job_failed(workspace_root, e.to_string());
                return Err(e);
            }
        }
    }
    if !pending_chunks.is_empty() {
        if let Err(e) = flush(
            &mut index,
            &mut pending_chunks,
            &mut embedded,
            &mut batch_i,
            files_done,
            embedder,
        ) {
            mark_index_job_failed(workspace_root, e.to_string());
            return Err(e);
        }
    }
    // A file may change between passes; pin done == total at the end.
    update_chunk_progress(workspace_root, embedded as u64, Some(embedded as u64));
    index.set_next_id(final_next_id.max(index.next_id()));
    index.set_build_stamps(build_stamps);

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

/// WP-141 periodic scan: the same walk and path filter as
/// [`scannable_files`], but metadata only (stat for size + stamp). The binary
/// sniff, which opens the file, is left to `update_files` for paths whose
/// stamp changed, so an unchanged tree is never opened.
pub(crate) fn scan_stat_candidates(
    workspace_root: &Path,
) -> Result<Vec<(String, Option<super::FileStamp>)>> {
    let mut out = Vec::new();
    let rel_ctx =
        RelPathCtx::new(workspace_root).unwrap_or_else(|_| RelPathCtx::new_lossy(workspace_root));
    // No binary sniff in the walk either: that opens every file.
    let walker = walk_builder_with(
        workspace_root,
        FilterPreset::Search,
        WalkOptions {
            skip_binary: Some(false),
            ..WalkOptions::default()
        },
    );
    for result in walker.build() {
        let Ok(entry) = result else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(rel_str) = rel_ctx.rel(entry.path()) else {
            continue;
        };
        if !is_scannable_rel_path(&rel_str) {
            continue;
        }
        let meta = std::fs::metadata(entry.path()).ok();
        if meta
            .as_ref()
            .is_some_and(|m| !m.is_file() || m.len() > MAX_INDEX_FILE_BYTES)
        {
            continue;
        }
        let stamp = meta.as_ref().and_then(super::reconcile::stamp_from_meta);
        out.push((rel_str, stamp));
    }
    out.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
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
