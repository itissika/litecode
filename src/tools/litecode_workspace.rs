//! Workspace configuration guides, one-shot JSON refresh, and live status.
//!
//! Split cleanly into three code-surface tools:
//! - `workspace.help` — read embedded guides (`overview | setup | excludes | mcp | custom_tools | apps`)
//! - `workspace.refresh` — validate `.litecode/{excludes,mcp,custom_tools,apps}.json` in one check
//! - `workspace.status` — inspect active excludes, MCP servers, custom tools, and Apps

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use serde::Serialize;
use serde_json::Value;

use crate::apps::registry::AppRegistry;
use crate::config::global_db::tools::mcp_catalog_id;
use crate::config::guides::{self, GUIDE_INDEX, GuideTopic};
use crate::config::schema::{McpServerDefinition, McpTransport, ToolOrigin};
use crate::config::settings_writer::{
    custom_definition_diagnostic, normalize_custom_definition, validate_custom_suite,
    validate_mcp_definition,
};
use crate::config::workspace::{read_workspace_custom_tools, read_workspace_mcp};
use crate::mcp::{McpRunState, McpServerSnapshot};
use crate::runtime::RuntimeHandle;
use crate::tool::Tool;
use crate::tool::availability::agent_tool_enabled;
use crate::tool::outcome::ToolOutcome;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;
use crate::workspace::filter::{
    WorkspaceExcludesFile, activate_workspace_excludes, active_workspace_excludes,
    read_workspace_excludes, workspace_excludes_path,
};

pub const WORKSPACE_HELP_TOOL: &str = "workspace.help";
pub const WORKSPACE_REFRESH_TOOL: &str = "workspace.refresh";
pub const WORKSPACE_STATUS_TOOL: &str = "workspace.status";

#[derive(Debug, Serialize)]
struct WorkspaceHelpData {
    topic: &'static str,
    content: &'static str,
}

#[derive(Debug, Serialize)]
struct WorkspaceRefreshData {
    ok: bool,
    excludes_ok: bool,
    mcp_ok: bool,
    custom_tools_ok: bool,
    apps_ok: bool,
    issues: Vec<String>,
}

#[derive(Debug, Serialize)]
struct McpStatusRow {
    id: String,
    enabled: bool,
    origin: String,
    state: String,
    command: String,
}

#[derive(Debug, Serialize)]
struct CustomToolStatusRow {
    id: String,
    suite: Option<String>,
    enabled: bool,
    origin: String,
    timeout_secs: u64,
    command: String,
}

#[derive(Debug, Serialize)]
struct AppStatusRow {
    id: String,
    kind: String,
    status: String,
    url: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorkspaceStatusData {
    excludes: String,
    mcp: Vec<McpStatusRow>,
    custom_tools: Vec<CustomToolStatusRow>,
    apps: Vec<AppStatusRow>,
}

/// Read workspace setup or `.litecode/*.json` guides.
#[derive(Debug, Default)]
pub struct WorkspaceHelpTool;

impl WorkspaceHelpTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for WorkspaceHelpTool {
    fn name(&self) -> &str {
        WORKSPACE_HELP_TOOL
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let raw_topic = match input.get("topic") {
            None | Some(Value::Null) => "overview",
            Some(Value::String(s)) => s.trim(),
            Some(_) => {
                return coded_error("invalid_input", "workspace.help: `topic` must be a string");
            }
        };

        let (topic, content) = match raw_topic {
            "" | "overview" => ("overview", GUIDE_INDEX),
            "setup" => ("setup", guides::topic(GuideTopic::Setup)),
            "excludes" => ("excludes", guides::topic(GuideTopic::Excludes)),
            "mcp" => ("mcp", guides::topic(GuideTopic::Mcp)),
            "custom_tools" => ("custom_tools", guides::topic(GuideTopic::CustomTools)),
            "apps" => ("apps", guides::topic(GuideTopic::Apps)),
            other => {
                return coded_error(
                    "invalid_input",
                    format!(
                        "unknown workspace.help topic '{other}'; expected overview, setup, excludes, mcp, custom_tools, or apps"
                    ),
                );
            }
        };

        ToolOutcome::ok(WorkspaceHelpData { topic, content }).into_call_result(content)
    }
}

/// Validate `.litecode/{excludes,mcp,custom_tools,apps}.json` in one check.
pub struct WorkspaceRefreshTool {
    runtime: Option<RuntimeHandle>,
}

impl WorkspaceRefreshTool {
    pub fn new(runtime: RuntimeHandle) -> Self {
        Self {
            runtime: Some(runtime),
        }
    }

    pub fn cold() -> Self {
        Self { runtime: None }
    }

    fn run_for_root(&self, workspace_root: &Path, reload_excludes_cache: bool) -> ToolCallResult {
        let mut issues = Vec::new();

        // 1. .litecode/excludes.json
        let excludes_ok = if workspace_excludes_path(workspace_root).exists() {
            match read_workspace_excludes(workspace_root) {
                Ok(file) => {
                    if reload_excludes_cache {
                        activate_workspace_excludes(file);
                    }
                    true
                }
                Err(error) => {
                    issues.push(format!("excludes.json: {}", reason(&error)));
                    false
                }
            }
        } else {
            true
        };

        // 2. .litecode/mcp.json
        let mut mcp_ok = true;
        match read_workspace_mcp(workspace_root) {
            Ok(mut file) => {
                let mut ids: Vec<String> = file.servers.keys().cloned().collect();
                ids.sort();
                for id in ids {
                    if let Some(def) = file.servers.get_mut(&id) {
                        if let Err(error) = validate_mcp_definition(&id, def) {
                            mcp_ok = false;
                            issues.push(format!("mcp.json `{id}`: {}", reason(&error)));
                        }
                    }
                }
            }
            Err(error) => {
                mcp_ok = false;
                issues.push(format!("mcp.json: {}", reason(&error)));
            }
        }

        // 3. .litecode/custom_tools.json
        let mut custom_tools_ok = true;
        match read_workspace_custom_tools(workspace_root) {
            Ok(mut file) => {
                let mut suite_ids: Vec<String> = file.suites.keys().cloned().collect();
                suite_ids.sort();
                for id in suite_ids {
                    if let Some(suite_def) = file.suites.get_mut(&id) {
                        if let Err(error) = validate_custom_suite(&id, suite_def) {
                            custom_tools_ok = false;
                            issues.push(format!(
                                "custom_tools.json suite `{id}`: {}",
                                reason(&error)
                            ));
                        }
                    }
                }

                let mut tool_ids: Vec<String> = file.tools.keys().cloned().collect();
                tool_ids.sort();
                for id in tool_ids {
                    if let Some(def) = file.tools.get_mut(&id) {
                        normalize_custom_definition(&id, def);
                        if let Some(diag) = custom_definition_diagnostic(
                            &id,
                            def,
                            &file.suites,
                            "workspace",
                            Some(".litecode/custom_tools.json".into()),
                        ) {
                            custom_tools_ok = false;
                            issues.push(format!("custom_tools.json: {}", diag.message));
                        }
                    }
                }
            }
            Err(error) => {
                custom_tools_ok = false;
                issues.push(format!("custom_tools.json: {}", reason(&error)));
            }
        }

        // 4. .litecode/apps.json
        let app_errors = AppRegistry::validate_workspace_file(workspace_root);
        let apps_ok = !app_errors.iter().any(|err| err.severity == "error");
        for err in app_errors {
            let app_part = err
                .app_id
                .as_deref()
                .map(|id| format!(" `{id}`"))
                .unwrap_or_default();
            issues.push(format!(
                "apps.json{app_part} ({}): {}",
                err.severity, err.message
            ));
        }

        let ok = excludes_ok && mcp_ok && custom_tools_ok && apps_ok;
        let summary = if ok && issues.is_empty() {
            "workspace configs valid (excludes, mcp, custom_tools, apps)".to_string()
        } else if ok {
            format!(
                "workspace configs valid with {} warning(s) (excludes, mcp, custom_tools, apps)",
                issues.len()
            )
        } else {
            let mut failed = Vec::new();
            if !excludes_ok {
                failed.push("excludes");
            }
            if !mcp_ok {
                failed.push("mcp");
            }
            if !custom_tools_ok {
                failed.push("custom_tools");
            }
            if !apps_ok {
                failed.push("apps");
            }
            format!(
                "workspace validation failed: {} ({} issue(s))",
                failed.join(", "),
                issues.len()
            )
        };

        let mut text = summary.clone();
        if !issues.is_empty() {
            text.push('\n');
            for issue in &issues {
                text.push_str(&format!("- {issue}\n"));
            }
        }

        ToolOutcome::ok(WorkspaceRefreshData {
            ok,
            excludes_ok,
            mcp_ok,
            custom_tools_ok,
            apps_ok,
            issues,
        })
        .into_call_result(text)
    }
}

impl Tool for WorkspaceRefreshTool {
    fn name(&self) -> &str {
        WORKSPACE_REFRESH_TOOL
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        _input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        let root = self
            .runtime
            .as_ref()
            .map(|rt| rt.workspace_root())
            .unwrap_or(&execution.workspace_root);
        let reload = self.runtime.is_some();
        Box::pin(std::future::ready(self.run_for_root(root, reload)))
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        if let Some(rt) = &self.runtime {
            self.run_for_root(rt.workspace_root(), true)
        } else {
            coded_error("unavailable", "workspace.refresh requires an active runtime")
        }
    }
}

/// Inspect live workspace runtime state: excludes, MCP servers, custom tools, and Apps.
pub struct WorkspaceStatusTool {
    runtime: Option<RuntimeHandle>,
    agent_id: String,
}

impl WorkspaceStatusTool {
    pub fn new(runtime: RuntimeHandle, agent_id: impl Into<String>) -> Self {
        Self {
            runtime: Some(runtime),
            agent_id: agent_id.into(),
        }
    }

    pub fn cold() -> Self {
        Self {
            runtime: None,
            agent_id: String::new(),
        }
    }

    fn mcp_snapshots(runtime: &RuntimeHandle) -> HashMap<String, McpServerSnapshot> {
        let pool = std::sync::Arc::clone(&runtime.mcp_pool);
        let hub = std::sync::Arc::clone(&pool);
        pool.block_on_hub(async move { hub.snapshots().await })
            .unwrap_or_default()
    }

    fn build_status(&self, runtime: &RuntimeHandle) -> (String, WorkspaceStatusData) {
        let resolved = &runtime.resolved;
        let excludes = excludes_counts(&active_workspace_excludes());

        let snapshots = Self::mcp_snapshots(runtime);
        let servers = resolved.mcp_servers();
        let mut mcp_ids: Vec<&String> = servers.keys().collect();
        mcp_ids.sort();
        let mcp: Vec<McpStatusRow> = mcp_ids
            .into_iter()
            .filter_map(|id| {
                let def = servers.get(id)?;
                let pool_key = resolved.mcp_pool_key(id);
                let state = run_state_label(snapshots.get(&pool_key));
                Some(McpStatusRow {
                    id: id.clone(),
                    enabled: agent_tool_enabled(resolved, &self.agent_id, &mcp_catalog_id(id)),
                    origin: origin_label(resolved.mcp_origin(id)).to_string(),
                    state,
                    command: mcp_command_label(def),
                })
            })
            .collect();

        let mut custom = resolved.custom_tools();
        custom.sort_by(|a, b| a.name.cmp(&b.name));
        let custom_tools: Vec<CustomToolStatusRow> = custom
            .into_iter()
            .map(|tool| {
                let enabled = agent_tool_enabled(resolved, &self.agent_id, &tool.name);
                let origin = origin_label(resolved.custom_origin(&tool.name)).to_string();
                let command = command_line_parts(&tool.command, &tool.args);
                CustomToolStatusRow {
                    id: tool.name,
                    suite: tool.suite,
                    enabled,
                    origin,
                    timeout_secs: tool.timeout,
                    command,
                }
            })
            .collect();

        let apps: Vec<AppStatusRow> = runtime
            .apps()
            .map(|service| {
                service
                    .list()
                    .apps
                    .into_iter()
                    .map(|app| {
                        let error = app.runtime_error.or_else(|| {
                            app.install
                                .and_then(|install| install.error.map(|e| e.message))
                        });
                        AppStatusRow {
                            id: app.id,
                            kind: app.source.as_str().to_string(),
                            status: app.status.as_str().to_string(),
                            url: app.url,
                            error,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let summary = format!(
            "excludes ({}); {} mcp, {} custom tool(s), {} app(s)",
            excludes,
            mcp.len(),
            custom_tools.len(),
            apps.len()
        );

        (
            summary,
            WorkspaceStatusData {
                excludes,
                mcp,
                custom_tools,
                apps,
            },
        )
    }
}

impl Tool for WorkspaceStatusTool {
    fn name(&self) -> &str {
        WORKSPACE_STATUS_TOOL
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        _input: Value,
        _execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(std::future::ready(self.call_inner(Value::Null)))
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        let Some(runtime) = &self.runtime else {
            return coded_error("unavailable", "workspace.status requires an active runtime");
        };
        let (summary, status) = self.build_status(runtime);
        ToolOutcome::ok(status).into_call_result(summary)
    }
}

fn reason(error: &crate::types::LitecodeError) -> String {
    const PREFIXES: [&str; 5] = [
        "config error: ",
        "tool execution error: ",
        "session storage error: ",
        "json error: ",
        "io error: ",
    ];
    let mut message = error.to_string();
    loop {
        match PREFIXES
            .iter()
            .find_map(|prefix| message.strip_prefix(prefix))
        {
            Some(rest) => message = rest.to_string(),
            None => return message,
        }
    }
}

fn mcp_command_label(def: &McpServerDefinition) -> String {
    match &def.transport {
        McpTransport::Stdio => command_line_parts(&def.command, &def.args),
        McpTransport::Remote { url, .. } => {
            if def.command.trim().is_empty() {
                url.clone()
            } else {
                command_line_parts(&def.command, &def.args)
            }
        }
    }
}

fn command_line_parts(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        command.to_string()
    } else {
        format!("{command} {}", args.join(" "))
    }
}

fn origin_label(origin: Option<ToolOrigin>) -> &'static str {
    match origin {
        Some(ToolOrigin::Workspace) => "workspace",
        _ => "global",
    }
}

fn run_state_label(snapshot: Option<&McpServerSnapshot>) -> String {
    match snapshot {
        Some(snapshot) if snapshot.status == McpRunState::Running => "running".to_string(),
        Some(snapshot) if snapshot.status == McpRunState::Starting => "starting".to_string(),
        Some(snapshot) if snapshot.status == McpRunState::Error => format!(
            "error: {}",
            snapshot.error.as_deref().unwrap_or("unknown error")
        ),
        _ => "stopped".to_string(),
    }
}

fn excludes_counts(file: &WorkspaceExcludesFile) -> String {
    format!(
        "files {} · search {} · watcher {} · code_index_extensions {} · git_ignore {} · explorer_git_ignore {}",
        file.files_exclude.len(),
        file.search_exclude.len(),
        file.watcher_exclude.len(),
        file.code_index_extensions.len(),
        file.git_ignore,
        file.explorer_git_ignore
    )
}

#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    use crate::tool::code_contract::DriftCase;
    use crate::tool::outcome::ok_data;

    vec![
        DriftCase {
            tool: WORKSPACE_HELP_TOOL,
            all_optional_present: ok_data(WorkspaceHelpData {
                topic: "mcp",
                content: guides::topic(GuideTopic::Mcp),
            }),
            all_optional_absent: ok_data(WorkspaceHelpData {
                topic: "overview",
                content: GUIDE_INDEX,
            }),
        },
        DriftCase {
            tool: WORKSPACE_REFRESH_TOOL,
            all_optional_present: ok_data(WorkspaceRefreshData {
                ok: false,
                excludes_ok: true,
                mcp_ok: false,
                custom_tools_ok: true,
                apps_ok: true,
                issues: vec!["mcp.json `bad`: command must not be empty".into()],
            }),
            all_optional_absent: ok_data(WorkspaceRefreshData {
                ok: true,
                excludes_ok: true,
                mcp_ok: true,
                custom_tools_ok: true,
                apps_ok: true,
                issues: Vec::new(),
            }),
        },
        DriftCase {
            tool: WORKSPACE_STATUS_TOOL,
            all_optional_present: ok_data(WorkspaceStatusData {
                excludes: "files 1 · search 1 · watcher 1 · code_index_extensions 1 · git_ignore true · explorer_git_ignore true".into(),
                mcp: vec![McpStatusRow {
                    id: "fs".into(),
                    enabled: true,
                    origin: "workspace".into(),
                    state: "running".into(),
                    command: "npx -y server".into(),
                }],
                custom_tools: vec![CustomToolStatusRow {
                    id: "lint_check".into(),
                    suite: Some("qa".into()),
                    enabled: true,
                    origin: "workspace".into(),
                    timeout_secs: 60,
                    command: "npm run lint".into(),
                }],
                apps: vec![AppStatusRow {
                    id: "studio".into(),
                    kind: "workspace".into(),
                    status: "error".into(),
                    url: Some("http://127.0.0.1:4100".into()),
                    error: Some("port in use".into()),
                }],
            }),
            all_optional_absent: ok_data(WorkspaceStatusData {
                excludes: "files 0 · search 0 · watcher 0 · code_index_extensions 0 · git_ignore false · explorer_git_ignore false".into(),
                mcp: vec![McpStatusRow {
                    id: "fs".into(),
                    enabled: false,
                    origin: "global".into(),
                    state: "stopped".into(),
                    command: "npx".into(),
                }],
                custom_tools: vec![CustomToolStatusRow {
                    id: "lint".into(),
                    suite: None,
                    enabled: false,
                    origin: "global".into(),
                    timeout_secs: 120,
                    command: "npm run lint".into(),
                }],
                apps: vec![AppStatusRow {
                    id: "gallery".into(),
                    kind: "builtin".into(),
                    status: "stopped".into(),
                    url: None,
                    error: None,
                }],
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn exec_ctx(root: &Path) -> ToolExecutionContext {
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: root.to_path_buf(),
            call_id: "ws-test".into(),
            cancel: tokio_util::sync::CancellationToken::new(),
            output_limit: 8000,
            session_id: String::new(),
            session: None,
        }
    }

    #[test]
    fn help_returns_overview_and_named_topics_and_rejects_unknown() {
        let help = WorkspaceHelpTool::new();

        let overview = help.call(json!({}));
        assert_eq!(overview.level, crate::types::ToolSignalLevel::Ok);
        let data = overview.data.expect("overview data");
        assert_eq!(data["topic"], "overview");
        assert_eq!(data["content"], GUIDE_INDEX);

        let apps = help.call(json!({ "topic": "apps" }));
        assert_eq!(apps.level, crate::types::ToolSignalLevel::Ok);
        let data = apps.data.expect("apps data");
        assert_eq!(data["topic"], "apps");
        assert_eq!(data["content"], guides::topic(GuideTopic::Apps));

        let bad = help.call(json!({ "topic": "sessions" }));
        assert_eq!(bad.level, crate::types::ToolSignalLevel::Error);
    }

    #[tokio::test]
    async fn refresh_returns_terse_success_when_all_workspace_configs_valid() {
        let dir = TempDir::new().unwrap();
        let dot = dir.path().join(".litecode");
        std::fs::create_dir_all(&dot).unwrap();
        std::fs::write(
            dot.join("excludes.json"),
            serde_json::to_string_pretty(&WorkspaceExcludesFile::builtin_defaults()).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dot.join("mcp.json"),
            r#"{"version":1,"servers":{"docs":{"command":"npx","args":["-y","docs"]}}}"#,
        )
        .unwrap();
        std::fs::write(
            dot.join("custom_tools.json"),
            r#"{"version":2,"tools":{"echo_ok":{"name":"echo_ok","command":"echo","schema":{"type":"object","properties":{}}}}}"#,
        )
        .unwrap();
        std::fs::write(
            dot.join("apps.json"),
            r#"{"version":2,"apps":{}}"#,
        )
        .unwrap();

        let refresh = WorkspaceRefreshTool::cold();
        let result = refresh.execute(json!({}), exec_ctx(dir.path())).await;
        assert_eq!(result.level, crate::types::ToolSignalLevel::Ok, "{}", result.content);
        let data = result.data.expect("refresh data");
        assert_eq!(data["ok"], true, "data={data}");
        assert_eq!(data["excludes_ok"], true);
        assert_eq!(data["mcp_ok"], true);
        assert_eq!(data["custom_tools_ok"], true);
        assert_eq!(data["apps_ok"], true);
        assert_eq!(data["issues"], json!([]));
        assert!(data.get("summary").is_none());
        assert_eq!(
            result.content,
            "workspace configs valid (excludes, mcp, custom_tools, apps)"
        );
    }

    #[tokio::test]
    async fn refresh_collects_all_broken_config_issues_without_short_circuiting() {
        let dir = TempDir::new().unwrap();
        let dot = dir.path().join(".litecode");
        std::fs::create_dir_all(&dot).unwrap();
        std::fs::write(dot.join("excludes.json"), "{not-json").unwrap();
        std::fs::write(
            dot.join("mcp.json"),
            r#"{"version":1,"servers":{"bad_mcp":{"command":""}}}"#,
        )
        .unwrap();
        std::fs::write(
            dot.join("custom_tools.json"),
            r#"{"version":1,"tools":{}}"#,
        )
        .unwrap();
        std::fs::write(dot.join("apps.json"), "{bad-apps-json").unwrap();

        let refresh = WorkspaceRefreshTool::cold();
        let result = refresh.execute(json!({}), exec_ctx(dir.path())).await;
        assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
        let data = result.data.expect("refresh data");
        assert_eq!(data["ok"], false);
        assert_eq!(data["excludes_ok"], false);
        assert_eq!(data["mcp_ok"], false);
        assert_eq!(data["custom_tools_ok"], false);
        assert_eq!(data["apps_ok"], false);
        let issues = data["issues"].as_array().unwrap();
        assert_eq!(issues.len(), 4, "issues: {issues:?}");
    }
}
