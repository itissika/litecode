//! File identity and query-bound cursors for the code surface.
//!
//! Hash is sha256 of the file bytes. `mtime`, `eol`, and `encoding` stay
//! internal (D-18). Cursor checks use the documented recommendation for D-07
//! (query fingerprint plus a corpus version). D-07 itself stays undecided.

use std::path::Path;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::tool::outcome::{FileRef, ToolOutcome};
use crate::types::ToolCallResult;

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathBase {
    Workspace,
    Absolute,
}

impl PathBase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Absolute => "absolute",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedPath {
    pub base: PathBase,
    pub path: String,
    pub abs: Option<String>,
}

/// Workspace files use a relative `path`. Outside files keep the caller's
/// original path and record the absolute path separately (D-17).
pub fn locate_resolved(workspace_root: &Path, resolved: &Path, requested: &str) -> LocatedPath {
    let resolved_canon = crate::config::path::canon_abs_lossy(resolved);
    let root_canon = crate::config::path::canon_abs_lossy(workspace_root);
    if crate::config::path::is_under(&resolved_canon, &root_canon) {
        let path = crate::workspace::filter::cheap_rel_under(&root_canon, &resolved_canon)
            .filter(|rel| !rel.is_empty())
            .unwrap_or_else(|| requested.trim().replace('\\', "/"));
        LocatedPath {
            base: PathBase::Workspace,
            path,
            abs: None,
        }
    } else {
        LocatedPath {
            base: PathBase::Absolute,
            path: requested.to_string(),
            abs: Some(absolute_path_string(&resolved_canon)),
        }
    }
}

pub fn absolute_path_string(path: &Path) -> String {
    crate::config::path::canon_abs_lossy(path)
        .to_string_lossy()
        .into_owned()
}

pub fn file_ref(located: LocatedPath, bytes: &[u8]) -> FileRef {
    FileRef {
        path: located.path,
        abs: located.abs,
        hash: sha256_hex(bytes),
        bytes: bytes.len() as u64,
    }
}

/// Stable query fingerprint. Field order is the caller's order.
pub fn fingerprint(fields: &[(&str, &str)]) -> String {
    let mut raw = String::new();
    for (key, value) in fields {
        raw.push_str(key);
        raw.push('=');
        raw.push_str(value);
        raw.push('\n');
    }
    sha256_hex(raw.as_bytes())
}

pub fn version_hash(chunks: impl IntoIterator<Item = impl AsRef<[u8]>>) -> String {
    let mut hasher = Sha256::new();
    for chunk in chunks {
        hasher.update(chunk.as_ref());
        hasher.update([0xff]);
    }
    format!("{:x}", hasher.finalize())
}

/// Opaque cursor: query fingerprint, corpus version, and a resume point.
/// This is not a stored copy of the result set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueryCursor {
    pub fp: String,
    pub ver: String,
    #[serde(default)]
    pub offset: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_line: Option<u64>,
}

pub fn encode_cursor(cursor: &QueryCursor) -> String {
    let bytes = serde_json::to_vec(cursor).unwrap_or_default();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn decode_cursor(raw: &str) -> Option<QueryCursor> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn stale_cursor_result(reason: &str) -> ToolCallResult {
    let message = match reason {
        "query" => "cursor does not match this query",
        "version" => "cursor no longer matches the files for this query",
        _ => "cursor is stale or invalid",
    };
    ToolOutcome::<Value>::failure("stale_cursor", message, true)
        .with_error_details(json!({ "reason": reason }))
        .into_call_result(message)
}

/// `Ok(None)` means "start at the beginning". A mismatch is `Err`.
pub fn bind_cursor(
    raw: Option<&str>,
    fingerprint: &str,
    version: &str,
) -> Result<Option<QueryCursor>, ToolCallResult> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let Some(cursor) = decode_cursor(raw) else {
        return Err(stale_cursor_result("malformed"));
    };
    if cursor.fp != fingerprint {
        return Err(stale_cursor_result("query"));
    }
    if cursor.ver != version {
        return Err(stale_cursor_result("version"));
    }
    Ok(Some(cursor))
}

#[cfg(test)]
mod tests {}
