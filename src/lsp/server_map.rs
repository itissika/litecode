//! Extension → language-server command table and workspace detection.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::lsp::deps;
use crate::types::{LitecodeError, Result};

pub(crate) fn default_server_map() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("rs".into(), "rust-analyzer".into());
    m.insert("ts".into(), "typescript-language-server --stdio".into());
    m.insert("tsx".into(), "typescript-language-server --stdio".into());
    m.insert("js".into(), "typescript-language-server --stdio".into());
    m.insert("jsx".into(), "typescript-language-server --stdio".into());
    m.insert("py".into(), "pyright-langserver --stdio".into());
    m.insert("go".into(), "gopls".into());
    m.insert("cs".into(), "csharp-ls".into());
    m.insert("csx".into(), "csharp-ls".into());
    m.insert("c".into(), "clangd".into());
    m.insert("h".into(), "clangd".into());
    m.insert("cpp".into(), "clangd".into());
    m.insert("hpp".into(), "clangd".into());
    m
}

/// Built-in ext → LS command table with optional `LITECODE_LSP_SERVERS` overrides.
pub fn server_map() -> HashMap<String, String> {
    let mut map = default_server_map();
    if let Ok(extra) = std::env::var("LITECODE_LSP_SERVERS") {
        for pair in extra.split(',') {
            let mut parts = pair.splitn(2, '=');
            if let (Some(ext), Some(cmd)) = (parts.next(), parts.next()) {
                map.insert(ext.trim().to_string(), cmd.trim().to_string());
            }
        }
    }
    map
}

pub fn server_command_for_ext(ext: &str) -> Option<String> {
    server_map().get(ext).cloned()
}

pub fn command_parts(command: &str) -> std::result::Result<Vec<String>, String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in command.chars() {
        if ch == '"' {
            quoted = !quoted;
        } else if ch.is_whitespace() && !quoted {
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if quoted {
        return Err(format!("invalid quoted language-server command: {command}"));
    }
    if !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        return Err("empty language-server command".into());
    }
    Ok(parts)
}

pub fn program_from_command(command: &str) -> String {
    command_parts(command)
        .ok()
        .and_then(|parts| parts.into_iter().next())
        .unwrap_or_else(|| command.to_string())
}

/// Verify LS binaries for languages detected in the workspace can actually run.
pub fn check_workspace_dependencies(root: &Path) -> Result<()> {
    let needed = detect_needed_server_commands(root);
    if needed.is_empty() {
        return Ok(());
    }
    let mut missing = Vec::new();
    for cmd in needed {
        let program = program_from_command(&cmd);
        if let Err(e) = deps::command_runnable_command(&cmd) {
            missing.push(format!("{program}: {e}"));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(LitecodeError::Config(format!(
            "language server(s) not runnable: {}. Install them or set LITECODE_LSP_SERVERS",
            missing.join("; ")
        )))
    }
}

/// Detect unique LS command strings needed for this workspace.
pub fn detect_needed_server_commands(root: &Path) -> Vec<String> {
    let map = server_map();
    let mut exts = HashSet::new();

    if root.join("Cargo.toml").exists() {
        exts.insert("rs".to_string());
    }
    if root.join("go.mod").exists() {
        exts.insert("go".to_string());
    }
    if root.join("package.json").exists() || root.join("tsconfig.json").exists() {
        exts.insert("ts".to_string());
    }
    if root.join("pyproject.toml").exists() || root.join("setup.py").exists() {
        exts.insert("py".to_string());
    }

    collect_extensions_shallow(root, 4, &mut exts);

    let mut commands = HashSet::new();
    for ext in exts {
        if let Some(cmd) = map.get(&ext) {
            commands.insert(cmd.clone());
        }
    }
    commands.into_iter().collect()
}

fn collect_extensions_shallow(dir: &Path, depth: usize, exts: &mut HashSet<String>) {
    if depth == 0 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') && name != ".git" {
            continue;
        }
        if path.is_dir() {
            if crate::workspace::filter::is_discovery_or_product_dir_name(&name) {
                continue;
            }
            collect_extensions_shallow(&path, depth - 1, exts);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            exts.insert(ext.to_lowercase());
        }
    }
}
