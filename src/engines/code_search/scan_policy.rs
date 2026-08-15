//! Re-export index / binary policy from [`crate::workspace::filter`].
//!
//! Kept so existing `engines::code_search::scan_policy` / pub-use paths compile.

pub use crate::workspace::filter::{
    MAX_INDEX_FILE_BYTES, SKIP_DIRS, TEXT_EXTENSIONS, is_indexable_rel_path, is_scannable_rel_path,
    looks_binary, path_has_skipped_dir,
};
