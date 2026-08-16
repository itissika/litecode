//! Language-server project root resolution from a file path (industry-standard markers).

use std::path::{Path, PathBuf};

use crate::types::{LitecodeError, Result};

/// LSP `languageId` for a file extension.
pub fn lsp_language_id(ext: &str) -> &'static str {
    if ext.eq_ignore_ascii_case("rs") {
        "rust"
    } else if ext.eq_ignore_ascii_case("ts") {
        "typescript"
    } else if ext.eq_ignore_ascii_case("tsx") {
        "typescriptreact"
    } else if ext.eq_ignore_ascii_case("js") {
        "javascript"
    } else if ext.eq_ignore_ascii_case("jsx") {
        "javascriptreact"
    } else if ext.eq_ignore_ascii_case("py") {
        "python"
    } else if ext.eq_ignore_ascii_case("go") {
        "go"
    } else if ext.eq_ignore_ascii_case("cs") || ext.eq_ignore_ascii_case("csx") {
        "csharp"
    } else if ext.eq_ignore_ascii_case("c") || ext.eq_ignore_ascii_case("h") {
        "c"
    } else if ext.eq_ignore_ascii_case("cpp")
        || ext.eq_ignore_ascii_case("hpp")
        || ext.eq_ignore_ascii_case("cc")
        || ext.eq_ignore_ascii_case("cxx")
    {
        "cpp"
    } else {
        "plaintext"
    }
}

/// Walk ancestors of `file` (including its parent directory) for the nearest language marker.
pub fn project_root_for_file(
    file: &Path,
    program: &str,
    workspace_root: Option<&Path>,
) -> Result<PathBuf> {
    let start = if file.is_file() {
        file.parent().unwrap_or(file)
    } else {
        file
    };

    for ancestor in start.ancestors() {
        if dir_matches_program_markers(ancestor, program) {
            if program == "rust-analyzer" {
                return Ok(rust_analyzer_root(ancestor, workspace_root));
            }
            return Ok(ancestor.to_path_buf());
        }
        if let Some(ws) = workspace_root
            && ancestor == ws
        {
            break;
        }
    }

    if let Some(ws) = workspace_root
        && crate::config::path::is_under(file, ws)
    {
        tracing::warn!(
            file = %file.display(),
            program,
            workspace = %ws.display(),
            "no language project marker found; falling back to workspace root"
        );
        return Ok(crate::config::path::canon_abs_lossy(ws));
    }

    Err(LitecodeError::Config(format!(
        "no project root for '{}' with language server '{program}'",
        file.display()
    )))
}

/// Prefer the outermost Cargo workspace that actually lists this crate as a
/// member, so one rust-analyzer covers the workspace graph. Nested crates that
/// are *not* members keep their own root (a second process). Workspace sandbox
/// (`is_under`) is unchanged — this only picks which process inside the workspace.
fn rust_analyzer_root(nearest_cargo: &Path, workspace_root: Option<&Path>) -> PathBuf {
    let mut best = nearest_cargo.to_path_buf();
    for ancestor in nearest_cargo.ancestors() {
        if cargo_toml_has_workspace_table(ancestor)
            && crate_is_workspace_member(nearest_cargo, ancestor)
        {
            best = ancestor.to_path_buf();
        }
        if let Some(ws) = workspace_root
            && ancestor == ws
        {
            break;
        }
    }
    best
}

fn cargo_toml_has_workspace_table(dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
        return false;
    };
    text.lines().any(|line| {
        let t = line.trim();
        t == "[workspace]" || t.starts_with("[workspace.")
    })
}

fn crate_is_workspace_member(crate_dir: &Path, workspace_dir: &Path) -> bool {
    let crate_dir = crate::config::path::canon_abs_lossy(crate_dir);
    let members = workspace_member_dirs(workspace_dir);
    members
        .iter()
        .any(|member| crate::config::path::canon_abs_lossy(member) == crate_dir)
}

fn workspace_member_dirs(workspace_dir: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(workspace_dir.join("Cargo.toml")) else {
        return Vec::new();
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return Vec::new();
    };
    let Some(ws) = value.get("workspace") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // `[package]` at the workspace root is an implicit member of that workspace.
    if value.get("package").is_some() {
        out.push(workspace_dir.to_path_buf());
    }
    if let Some(arr) = ws.get("members").and_then(|m| m.as_array()) {
        for member in arr.iter().filter_map(|v| v.as_str()) {
            out.extend(expand_workspace_member(workspace_dir, member));
        }
    }
    if let Some(arr) = ws.get("exclude").and_then(|m| m.as_array()) {
        let excluded: Vec<PathBuf> = arr
            .iter()
            .filter_map(|v| v.as_str())
            .flat_map(|m| expand_workspace_member(workspace_dir, m))
            .map(|p| crate::config::path::canon_abs_lossy(&p))
            .collect();
        out.retain(|p| !excluded.contains(&crate::config::path::canon_abs_lossy(p)));
    }
    out
}

fn expand_workspace_member(workspace_dir: &Path, member: &str) -> Vec<PathBuf> {
    if member == "." {
        return vec![workspace_dir.to_path_buf()];
    }
    if member.contains('*') || member.contains('?') || member.contains('[') {
        let pattern = workspace_dir.join(member);
        let pattern = pattern.to_string_lossy().replace('\\', "/");
        return glob::glob(&pattern)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|p| {
                let dir = if p.is_file() {
                    p.parent().map(Path::to_path_buf)
                } else {
                    Some(p)
                }?;
                dir.join("Cargo.toml").is_file().then_some(dir)
            })
            .collect();
    }
    vec![workspace_dir.join(member)]
}

fn dir_matches_program_markers(dir: &Path, program: &str) -> bool {
    match program {
        "rust-analyzer" => {
            marker_file_exists(dir, "Cargo.toml") || marker_file_exists(dir, "rust-project.json")
        }
        "typescript-language-server" => {
            marker_file_exists(dir, "tsconfig.json")
                || marker_file_exists(dir, "jsconfig.json")
                || marker_file_exists(dir, "package.json")
        }
        "gopls" => marker_file_exists(dir, "go.work") || marker_file_exists(dir, "go.mod"),
        "pyright-langserver" | "pyright" => {
            marker_file_exists(dir, "pyrightconfig.json")
                || marker_file_exists(dir, "pyproject.toml")
                || marker_file_exists(dir, "setup.py")
                || marker_file_exists(dir, "requirements.txt")
        }
        "clangd" => {
            marker_file_exists(dir, "compile_commands.json")
                || marker_file_exists(dir, ".clangd")
                || marker_file_exists(dir, "compile_flags.txt")
                || marker_file_exists(dir, "CMakeLists.txt")
        }
        "csharp-ls" => dir_has_glob(dir, "*.sln") || dir_has_glob(dir, "*.csproj"),
        _ => {
            marker_file_exists(dir, "package.json")
                || marker_file_exists(dir, "Cargo.toml")
                || marker_file_exists(dir, "go.mod")
                || marker_file_exists(dir, "pyproject.toml")
        }
    }
}

fn marker_file_exists(dir: &Path, name: &str) -> bool {
    dir.join(name).is_file()
}

fn dir_has_glob(dir: &Path, pattern: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let suffix = pattern.strip_prefix('*').unwrap_or(pattern);
    entries.flatten().any(|e| {
        e.path()
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| name.ends_with(suffix))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monorepo_ts_file_resolves_to_web_subdir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
        let web = root.join("web");
        std::fs::create_dir_all(web.join("src")).unwrap();
        std::fs::write(web.join("tsconfig.json"), "{}").unwrap();
        let ts_file = web.join("src/foo.ts");
        std::fs::write(&ts_file, "export {}").unwrap();

        let project_root =
            project_root_for_file(&ts_file, "typescript-language-server", Some(root)).unwrap();
        assert_eq!(project_root, web);
    }

    #[test]
    fn rust_file_resolves_to_crate_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        let rs_file = root.join("src/main.rs");
        std::fs::write(&rs_file, "fn main() {}").unwrap();

        let project_root = project_root_for_file(&rs_file, "rust-analyzer", Some(root)).unwrap();
        assert_eq!(project_root, root);
    }

    #[test]
    fn rust_analyzer_uses_cargo_workspace_not_member_crate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/foo\"]\n",
        )
        .unwrap();
        let crate_dir = root.join("crates/foo");
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let rs_file = crate_dir.join("src/lib.rs");
        std::fs::write(&rs_file, "pub fn f() {}").unwrap();

        let project_root = project_root_for_file(&rs_file, "rust-analyzer", Some(root)).unwrap();
        assert_eq!(project_root, root);
    }

    #[test]
    fn rust_nested_crate_outside_workspace_members_keeps_own_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"t\"\nversion = \"0.1.0\"\n\n[workspace]\nmembers = [\".\"]\n",
        )
        .unwrap();
        let nested = root.join("dev/other");
        std::fs::create_dir_all(nested.join("src")).unwrap();
        std::fs::write(
            nested.join("Cargo.toml"),
            "[package]\nname = \"other\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let rs_file = nested.join("src/lib.rs");
        std::fs::write(&rs_file, "pub fn f() {}\n").unwrap();

        let project_root = project_root_for_file(&rs_file, "rust-analyzer", Some(root)).unwrap();
        assert_eq!(
            crate::config::path::canon_abs_lossy(&project_root),
            crate::config::path::canon_abs_lossy(&nested)
        );
    }

    #[test]
    fn package_json_only_ts_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("package.json"), "{}").unwrap();
        let ts_file = root.join("index.ts");
        std::fs::write(&ts_file, "export {}").unwrap();

        let project_root =
            project_root_for_file(&ts_file, "typescript-language-server", Some(root)).unwrap();
        assert_eq!(project_root, root);
    }

    #[test]
    fn go_mod_marker() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("go.mod"), "module example.com\n").unwrap();
        let go_file = root.join("main.go");
        std::fs::write(&go_file, "package main").unwrap();

        let project_root = project_root_for_file(&go_file, "gopls", Some(root)).unwrap();
        assert_eq!(project_root, root);
    }

    #[test]
    fn pyproject_marker() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("pyproject.toml"), "[project]\nname = \"t\"\n").unwrap();
        let py_file = root.join("main.py");
        std::fs::write(&py_file, "print('hi')").unwrap();

        let project_root =
            project_root_for_file(&py_file, "pyright-langserver", Some(root)).unwrap();
        assert_eq!(project_root, root);
    }

    #[test]
    fn lsp_language_id_maps_common_extensions() {
        assert_eq!(lsp_language_id("rs"), "rust");
        assert_eq!(lsp_language_id("ts"), "typescript");
        assert_eq!(lsp_language_id("tsx"), "typescriptreact");
        assert_eq!(lsp_language_id("py"), "python");
    }
}
