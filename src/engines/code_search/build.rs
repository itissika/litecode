//! Workspace scans that feed the index. WP-143: there is no separate full
//! build; first build and rebuild go through the incremental path
//! ([`super::update_files`]).

use std::path::Path;

use crate::types::Result;
use crate::workspace::filter::{
    FilterPreset, MAX_INDEX_FILE_BYTES, RelPathCtx, WalkOptions, code_index_extension_allowed,
    is_indexable_rel_path, is_scannable_rel_path, walk_builder, walk_builder_with,
};

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
        FilterPreset::CodeIndex,
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
        if !is_scannable_rel_path(&rel_str) || !code_index_extension_allowed(&rel_str) {
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
    let walker = walk_builder(workspace_root, FilterPreset::CodeIndex);
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
    
    
    
    
    

}
