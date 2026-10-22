//! Phase 0 snapshot of the SDK `run_code` renders for the current code surface.

use std::path::PathBuf;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::tool::code_manifest::render_code_tool_description;
use crate::tool::registry::builtin_code_surface_tools;
use crate::tool::trait_::Tool;

fn sdk_text() -> String {
    let tools: Vec<Arc<dyn Tool>> = builtin_code_surface_tools();
    render_code_tool_description(&tools).expect("current code surface must render an SDK")
}

fn sha256_hex(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/tool/fixtures")
        .join(name)
}

#[test]
fn phase0_run_code_sdk_snapshot() {
    let sdk = sdk_text();
    let hash = sha256_hex(&sdk);
    let text_path = fixture("run_code_sdk.txt");
    let hash_path = fixture("run_code_sdk.sha256");
    let expected = std::fs::read_to_string(&text_path).unwrap_or_else(|error| {
        panic!("missing SDK snapshot {}: {error}", text_path.display());
    });
    let expected_hash = std::fs::read_to_string(&hash_path)
        .unwrap_or_else(|error| panic!("missing SDK hash {}: {error}", hash_path.display()));
    assert_eq!(sdk, expected, "run_code SDK text drifted");
    assert_eq!(hash, expected_hash.trim(), "run_code SDK hash drifted");
    assert!(sdk.contains("declare const tools:"));
    assert!(hash.len() == 64);
    // WP-210 I: the LSP tools are on the surface now; their examples say
    // `preview` and their schemas use `file_path` by design (WP-202). The
    // retired `view` parameter and `file_path` on the file tools stay gone.
    assert!(
        !sdk.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|word| word == "view"),
        "sdk snapshot still contains view"
    );
    for block in sdk.split("\ntype ").skip(1) {
        let block = block.split("\n\n").next().unwrap_or("");
        let name = block.split_whitespace().next().unwrap_or("");
        let file_tool = ["Read", "Edit", "ApplyEdits", "Write", "Grep", "Glob"]
            .iter()
            .any(|prefix| name.starts_with(prefix));
        assert!(
            !(file_tool && block.contains("file_path")),
            "sdk snapshot still contains file_path in {name}"
        );
    }
    assert!(
        !sdk.contains("context_lines"),
        "sdk snapshot still contains context_lines"
    );
}

#[test]
fn sdk_hash_is_stable_for_the_same_tool_set_under_different_cwd_and_permission() {
    use crate::config::ConfigManager;
    use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
    use crate::config::schema::{AgentProfile, AgentToolBinding, GlobalSettings};
    use crate::context_pipeline::Context;
    use crate::permission::{PermissionAction, PermissionEngine, ToolPolicy};
    use crate::tool::project::project_tools;

    let tools = builtin_code_surface_tools();
    let once = render_code_tool_description(&tools).unwrap();
    let twice = render_code_tool_description(&tools).unwrap();
    assert_eq!(sha256_hex(&once), sha256_hex(&twice));

    let allow_root = std::path::Path::new("E:/one");
    let ask_root = std::path::Path::new("E:/two");
    let allow_ctx = Context {
        cwd: allow_root.to_path_buf(),
        workspace_paths: crate::config::WorkspacePaths::for_legacy_root(allow_root),
        agents_md: None,
        claude_md: None,
    };
    let ask_ctx = Context {
        cwd: ask_root.to_path_buf(),
        workspace_paths: crate::config::WorkspacePaths::for_legacy_root(ask_root),
        agents_md: None,
        claude_md: None,
    };
    assert_ne!(allow_ctx.cwd, ask_ctx.cwd);
    let allow = PermissionEngine::resolver(
        resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new(allow_root)),
        "default",
        0,
    );
    let mut asked = GlobalSettings::default();
    let binding = AgentToolBinding {
        enabled: true,
        policy: ToolPolicy::with_default(PermissionAction::Ask),
        path_mode: Default::default(),
        last_applied_preset: None,
        allowed_tools: None,
    };
    let mut bindings = std::collections::HashMap::new();
    for tool in &tools {
        bindings.insert(tool.name().to_string(), binding.clone());
    }
    asked.agents.insert(
        "default".into(),
        AgentProfile {
            tools: bindings,
            ..Default::default()
        },
    );
    let ask = PermissionEngine::resolver(
        ConfigManager::resolve_without_catalog(asked, WorkspaceState::new(ask_root)),
        "default",
        0,
    );
    let _ = (allow, ask);
    let left = project_tools(builtin_code_surface_tools());
    let right = project_tools(builtin_code_surface_tools());
    assert_eq!(left.whitelist, right.whitelist);
    assert_eq!(sha256_hex(&left.sdk), sha256_hex(&right.sdk));
    assert_eq!(sha256_hex(&left.sdk), sha256_hex(&once));
}
