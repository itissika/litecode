//! LSP file URI helpers.

use std::path::{Path, PathBuf};

pub fn file_to_uri(path: &Path) -> String {
    let path = crate::config::path::strip_verbatim(path);
    url::Url::from_file_path(&path)
        .map(|u| u.to_string())
        .unwrap_or_else(|_| format!("file://{}", path.display()))
}

pub(crate) fn uri_to_path(uri: &str) -> Option<PathBuf> {
    // Defense: frontend used to emit `file:////?/E:/...` when projectRoot still
    // carried the Windows canonicalize verbatim prefix. Normalize before parse.
    let normalized = normalize_windows_file_uri(uri);
    url::Url::parse(&normalized)
        .ok()
        .and_then(|u| u.to_file_path().ok())
        .map(|p| crate::config::path::strip_verbatim(&p))
}

/// Rewrite mangled Windows verbatim file URIs into a form `url` can parse.
///
/// `\\?\E:\foo` → path `//?/E:/foo` → URI `file:////?/E:/foo` (invalid).
/// Recover to `file:///E:/foo`.
pub(crate) fn normalize_windows_file_uri(uri: &str) -> String {
    let Some(rest) = uri
        .strip_prefix("file://")
        .or_else(|| uri.strip_prefix("FILE://"))
    else {
        return uri.to_string();
    };

    // `//?/E:/...` or `///?/E:/...` (and UNC variants) after the scheme.
    let verbatim = rest
        .strip_prefix("//?/")
        .or_else(|| rest.strip_prefix("///?/"))
        .or_else(|| rest.strip_prefix("/?/"));
    if let Some(path) = verbatim {
        if let Some(unc) = path
            .strip_prefix("UNC/")
            .or_else(|| path.strip_prefix("unc/"))
        {
            return format!("file://{unc}");
        }
        return format!("file:///{path}");
    }

    uri.to_string()
}

pub(crate) fn canonical_project_root(path: &Path) -> PathBuf {
    crate::config::path::canon_abs_lossy(path)
}

pub(crate) fn publish_diagnostics_uri_matches(notification_uri: &str, expected_uri: &str) -> bool {
    if notification_uri == expected_uri {
        return true;
    }
    // LS may differ on drive-letter case or verbatim encoding; compare as paths.
    match (uri_to_path(notification_uri), uri_to_path(expected_uri)) {
        (Some(a), Some(b)) => a == b,
        _ => normalize_windows_file_uri(notification_uri)
            .eq_ignore_ascii_case(&normalize_windows_file_uri(expected_uri)),
    }
}
