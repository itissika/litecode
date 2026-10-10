//! Agent `litecode_workspace` — the workspace facade.
//!
//! One `action` string in command-line form. No argument opens the panel:
//! current state, plus the buttons that open the next popup. Every command
//! reads. This tool never writes a config file and never enables a tool.
//! A valid excludes file is already in effect. MCP and custom tools take
//! effect when the workspace has no running session.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::config::global_db::tools::mcp_catalog_id;
use crate::config::guides::{self, GuideTopic};
use crate::config::schema::{CustomToolDefinition, McpServerDefinition, ToolOrigin};
use crate::config::settings_writer::{validate_custom_definition, validate_mcp_definition};
use crate::tools::custom::validate_custom_tool_run;
use crate::config::workspace::{read_workspace_custom_tools, read_workspace_mcp};
use crate::context_pipeline::Context;
use crate::engines::session_search::short_session_ref;
use crate::mcp::{McpRunState, McpServerSnapshot};
use crate::runtime::RuntimeHandle;
use crate::session::SessionDataReader;
use crate::session::manager::{SessionActivityRow, SessionManager, SessionStatus};
use crate::tool::Tool;
use crate::tool::availability::agent_tool_enabled;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;
use crate::workspace::filter::{
    WorkspaceExcludesFile, active_workspace_excludes, read_workspace_excludes,
    workspace_excludes_path,
};

/// Running rows in the panel / `sessions` before the list is truncated.
const MAX_RUNNING_ROWS: usize = 10;
/// Recently active rows (idle sessions) in the panel / `sessions`.
const MAX_RECENT_ROWS: usize = 5;
/// "Recently active" window for the idle list.
const RECENT_WINDOW_MS: i64 = 5 * 60 * 1000;
/// One-line user-message preview cap.
const MAX_PREVIEW_CHARS: usize = 100;
/// Bound the self-subtree walk.
const MAX_SUBTREE_NODES: usize = 64;
/// `sessions --limit` upper bound.
const MAX_LIMIT: usize = 200;

const FILE_BROKEN: &str = "The file is broken; fix it.";
const APPLIES_WHEN_IDLE: &str = "When this workspace has no running session, this file takes \
     effect on its own. Tell the human.";
const ASK_HUMAN: &str = "If you actually need it, ask a human to enable it in Settings → \
     Agents. Otherwise stay silent.";
struct CommandSpec {
    name: &'static str,
    usage: &'static str,
    summary: &'static str,
    help: &'static str,
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        usage: "help [command]",
        summary: "the panel, or one command",
        help: "`help` opens the panel. `help <command>` prints that command.",
    },
    CommandSpec {
        name: "status",
        usage: "status",
        summary: "the picture again, without the button table",
        help: "`status` prints the same sections as the panel and skips the button table.",
    },
    CommandSpec {
        name: "sessions",
        usage: "sessions [--all] [--limit N]",
        summary: "sessions in this workspace",
        help: "`sessions` lists running sessions (longest first) and idle sessions from the \
               last 5 minutes. By default it hides you, your ancestors, and your subtree (your \
               children are `subagent_list`); `--all` shows you and your ancestors but still \
               hides your subtree. Your own row is marked `it's you`. `--limit N` caps each \
               list (default: 10 running, 5 idle).",
    },
    CommandSpec {
        name: "guide",
        usage: "guide [excludes|mcp|custom_tools]",
        summary: "how to edit that file, and who has to act",
        help: "`guide` lists topics. `guide <topic>` prints that topic.",
    },
    CommandSpec {
        name: "refresh",
        usage: "refresh [excludes|mcp|custom_tools|all]",
        summary: "whether those files will be accepted",
        help: "`refresh` checks `.litecode/excludes.json`, `mcp.json`, and \
               `custom_tools.json`. A valid excludes file is already in effect. MCP and \
               custom tools take effect when this workspace has no running session. \
               A file that fails validation names the error; fix it and run `refresh` again.",
    },
    CommandSpec {
        name: "validate_custom",
        usage: "validate_custom  (also pass definition + sample_input fields)",
        summary: "run a custom tool once without registering it",
        help: "Set `action` to `validate_custom`, plus `definition` (custom tool body) and `
               sample_input` (stdin JSON object). Does not write files, enable the tool,
               or evaluate `rules` (PASS is process/envelope only). Strips
               `run_in_background` from sample stdin like production.",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    Panel,
    Help(String),
    Status,
    Sessions {
        all: bool,
        /// Rows per list; `None` keeps the panel defaults (10 running / 5 recent).
        limit: Option<usize>,
    },
    Guide(Option<GuideTopic>),
    Refresh(Option<RefreshTarget>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefreshTarget {
    Excludes,
    Mcp,
    CustomTools,
}

const TOPICS: &str = "\
topics:\n\
\n\
- `excludes`\n\
- `mcp`\n\
- `custom_tools`";


fn parse_validate_custom(input: &Value) -> Result<(CustomToolDefinition, Value), String> {
    let def_val = input.get("definition").ok_or_else(|| {
        "# error\n\n`validate_custom` requires a `definition` object (custom tool body).\n"
            .to_string()
    })?;
    let mut def: CustomToolDefinition = serde_json::from_value(def_val.clone())
        .map_err(|e| format!("# error\n\ninvalid `definition`: {e}\n"))?;
    if def.name.trim().is_empty() {
        if let Some(n) = input.get("name").and_then(Value::as_str) {
            def.name = n.to_string();
        }
    }
    if def.name.trim().is_empty() {
        return Err("# error\n\n`definition.name` is required.\n".into());
    }
    let sample_input = input
        .get("sample_input")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if !sample_input.is_object() {
        return Err("# error\n\n`sample_input` must be a JSON object (tool stdin).\n".into());
    }
    Ok((def, sample_input))
}

fn parse_action(raw: &str) -> Result<Action, String> {
    let mut tokens = raw.split_whitespace();
    let Some(command) = tokens.next() else {
        return Ok(Action::Panel);
    };
    let rest: Vec<&str> = tokens.collect();
    match command {
        "help" => match rest.as_slice() {
            [] => Ok(Action::Panel),
            [name] => Ok(Action::Help((*name).to_string())),
            _ => Err(usage_error("help")),
        },
        "status" => {
            if rest.is_empty() {
                Ok(Action::Status)
            } else {
                Err(usage_error("status"))
            }
        }
        "sessions" => parse_sessions(&rest),
        "guide" => match rest.as_slice() {
            [] => Ok(Action::Guide(None)),
            [topic] => GuideTopic::parse(topic)
                .map(|topic| Action::Guide(Some(topic)))
                .ok_or_else(|| format!("# error\n\nunknown guide topic '{topic}'\n\n{TOPICS}\n")),
            _ => Err(usage_error("guide")),
        },
        "refresh" => match rest.as_slice() {
            [] | ["all"] => Ok(Action::Refresh(None)),
            [topic] => match *topic {
                "excludes" => Ok(Action::Refresh(Some(RefreshTarget::Excludes))),
                "mcp" => Ok(Action::Refresh(Some(RefreshTarget::Mcp))),
                "custom_tools" => Ok(Action::Refresh(Some(RefreshTarget::CustomTools))),
                other => Err(format!(
                    "# error\n\nunknown refresh topic '{other}'\n\n{TOPICS}\n\nor `all`.\n"
                )),
            },
            _ => Err(usage_error("refresh")),
        },
        other => Err(format!(
            "# error\n\nunknown command '{other}'\n\n{}\n",
            buttons()
        )),
    }
}

fn parse_sessions(rest: &[&str]) -> Result<Action, String> {
    let mut all = false;
    let mut limit: Option<usize> = None;
    let mut index = 0;
    while index < rest.len() {
        match rest[index] {
            "--all" => {
                all = true;
                index += 1;
            }
            "--limit" => {
                let Some(value) = rest.get(index + 1) else {
                    return Err(format!(
                        "`--limit` needs a number\n\n{}",
                        usage_error("sessions")
                    ));
                };
                let parsed: usize = value.parse().map_err(|_| {
                    format!(
                        "`--limit {value}` is not a number\n\n{}",
                        usage_error("sessions")
                    )
                })?;
                if parsed == 0 || parsed > MAX_LIMIT {
                    return Err(format!(
                        "`--limit {value}` is out of range (1..={MAX_LIMIT})\n\n{}",
                        usage_error("sessions")
                    ));
                }
                limit = Some(parsed);
                index += 2;
            }
            other => {
                return Err(format!(
                    "unknown flag '{other}'\n\n{}",
                    usage_error("sessions")
                ));
            }
        }
    }
    Ok(Action::Sessions { all, limit })
}

fn usage_error(command: &str) -> String {
    match COMMANDS.iter().find(|spec| spec.name == command) {
        Some(spec) => format!(
            "# {command}\n\n- usage: `{}`\n\n{}\n",
            spec.usage, spec.help
        ),
        None => buttons(),
    }
}

fn buttons() -> String {
    let mut out = String::from("# Buttons\n\n| command | popup |\n| --- | --- |\n");
    for spec in COMMANDS {
        if spec.name == "help" {
            continue;
        }
        out.push_str(&format!(
            "| `{}` | {} |\n",
            spec.usage.replace('|', "\\|"),
            spec.summary
        ));
    }
    out
}

pub struct LitecodeWorkspaceTool {
    runtime: RuntimeHandle,
    sessions: Arc<SessionManager>,
    agent_id: String,
}

impl LitecodeWorkspaceTool {
    pub fn new(runtime: RuntimeHandle, sessions: Arc<SessionManager>, agent_id: String) -> Self {
        Self {
            runtime,
            sessions,
            agent_id,
        }
    }

    fn run(&self, input: &Value, execution: &ToolExecutionContext) -> ToolCallResult {
        let raw = input.get("action").and_then(Value::as_str).unwrap_or("");
        if raw.trim() == "validate_custom" {
            return match parse_validate_custom(input) {
                Err(message) => ToolCallResult::error(message),
                Ok((mut def, sample_input)) => {
                    let id = def.name.clone();
                    if let Err(error) = validate_custom_definition(&id, &mut def) {
                        return ToolCallResult::error(format!(
                            "# validate_custom — FAIL\n\n- definition rejected: {error}"
                        ));
                    }
                    validate_custom_tool_run(&def, &sample_input, self.runtime.workspace_root())
                }
            };
        }
        match parse_action(raw) {
            Err(message) => ToolCallResult::error(message),
            Ok(Action::Panel) => self.panel(execution, true),
            Ok(Action::Help(name)) => match COMMANDS.iter().find(|spec| spec.name == name) {
                Some(spec) => ToolCallResult::ok(format!(
                    "# {}\n\n- usage: `{}`\n- {}\n\n{}\n",
                    spec.name, spec.usage, spec.summary, spec.help
                )),
                None => ToolCallResult::error(format!(
                    "# error\n\nunknown command '{name}'\n\n{}\n",
                    buttons()
                )),
            },
            Ok(Action::Status) => self.panel(execution, false),
            Ok(Action::Sessions { all, limit }) => {
                let (running_limit, recent_limit) = match limit {
                    Some(limit) => (limit, limit),
                    None => (MAX_RUNNING_ROWS, MAX_RECENT_ROWS),
                };
                match self.sessions_section(execution, all, running_limit, recent_limit) {
                    Ok((body, warning)) => {
                        let result = ToolCallResult::ok(format!("# sessions\n\n{body}"));
                        match warning {
                            Some(warning) => result.with_warning(warning),
                            None => result,
                        }
                    }
                    Err(error) => ToolCallResult::error(error),
                }
            }
            Ok(Action::Guide(topic)) => match topic {
                Some(topic) => ToolCallResult::ok(guides::topic(topic)),
                None => ToolCallResult::ok(guides::GUIDE_INDEX),
            },
            Ok(Action::Refresh(target)) => self.refresh(target),
        }
    }

    /// Panel (`with_buttons`) or the same picture without the button table.
    fn panel(&self, execution: &ToolExecutionContext, with_buttons: bool) -> ToolCallResult {
        let (sessions, sessions_warning) =
            match self.sessions_section(execution, false, MAX_RUNNING_ROWS, MAX_RECENT_ROWS) {
                Ok((body, warning)) => (body, warning),
                Err(error) => (unavailable(&error), None),
            };
        let mut sections = vec![
            section("excludes", self.excludes_section()),
            section("mcp", self.mcp_section()),
            section("custom tools", self.custom_tools_section()),
            section("sessions", Ok(sessions)),
        ];
        if with_buttons {
            sections.push(buttons());
        }
        let out = format!("# Workspace\n\n{}\n", sections.join("\n---\n\n"));
        let mut warnings = Vec::new();
        if let Some(warning) = sessions_warning {
            warnings.push(warning);
        }
        let result = ToolCallResult::ok(out);
        if warnings.is_empty() {
            result
        } else {
            result.with_warning(warnings.join(" · "))
        }
    }

    fn excludes_section(&self) -> Result<String, String> {
        let root = self.runtime.workspace_root();
        let exists = workspace_excludes_path(root).exists();
        let active = active_workspace_excludes();
        let mut out = format!(
            "- file: `.litecode/excludes.json`{}\n",
            if exists {
                ""
            } else {
                " (missing — builtin defaults)"
            }
        );
        out.push_str(&format!("- active: {}\n", excludes_counts(&active)));
        if exists && read_workspace_excludes(root).is_err() {
            out.push_str(&format!("- {FILE_BROKEN}\n"));
        }
        Ok(out)
    }

    fn mcp_section(&self) -> Result<String, String> {
        let resolved = &self.runtime.resolved;
        let servers = resolved.mcp_servers();
        let snapshots = self.mcp_snapshots();
        let mut ids: Vec<String> = servers.keys().cloned().collect();
        ids.sort();
        let mut out = String::new();
        let mut any_off = false;
        if ids.is_empty() {
            out.push_str("- none\n");
        }
        for id in &ids {
            let snapshot = snapshots.get(&resolved.mcp_pool_key(id));
            let on = agent_tool_enabled(resolved, &self.agent_id, &mcp_catalog_id(id));
            if !on {
                any_off = true;
            }
            out.push_str(&format!(
                "- `{id}` · {} · {} · {}\n",
                run_state_label(snapshot),
                gate_label(on),
                origin_label(resolved.mcp_origin(id))
            ));
        }
        match read_workspace_mcp(self.runtime.workspace_root()) {
            Ok(file) => {
                let drift = def_drift(resolved.workspace_mcp_servers(), &file.servers);
                if !drift.is_empty() {
                    out.push_str(&format!("- drift: {drift}\n"));
                    out.push_str(&format!("- next: {APPLIES_WHEN_IDLE}\n"));
                }
            }
            Err(_error) => {
                out.push_str(&format!("- {FILE_BROKEN}\n"));
            }
        }
        if any_off {
            out.push_str(&format!("- next: {ASK_HUMAN}\n"));
        }
        Ok(out)
    }

    fn custom_tools_section(&self) -> Result<String, String> {
        let resolved = &self.runtime.resolved;
        let mut tools = resolved.custom_tools();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        let mut out = String::new();
        let mut any_off = false;
        if tools.is_empty() {
            out.push_str("- none\n");
        }
        for tool in &tools {
            let on = agent_tool_enabled(resolved, &self.agent_id, &tool.name);
            if !on {
                any_off = true;
            }
            out.push_str(&format!(
                "- `{}` · `{}` · {} · {} · {}\n",
                tool.name,
                command_line_parts(&tool.command, &tool.args),
                timeout_label(tool.timeout),
                gate_label(on),
                origin_label(resolved.custom_origin(&tool.name))
            ));
        }
        match read_workspace_custom_tools(self.runtime.workspace_root()) {
            Ok(file) => {
                let loaded: HashMap<String, CustomToolDefinition> =
                    resolved.workspace_custom_tools().clone();
                let drift = def_drift(&loaded, &file.tools);
                if !drift.is_empty() {
                    out.push_str(&format!("- drift: {drift}\n"));
                    out.push_str(&format!("- next: {APPLIES_WHEN_IDLE}\n"));
                }
            }
            Err(_error) => {
                out.push_str(&format!("- {FILE_BROKEN}\n"));
            }
        }
        if any_off {
            out.push_str(&format!("- next: {ASK_HUMAN}\n"));
        }
        Ok(out)
    }

    /// Running turns + recently active sessions + total count.
    fn sessions_section(
        &self,
        execution: &ToolExecutionContext,
        all: bool,
        running_limit: usize,
        recent_limit: usize,
    ) -> Result<(String, Option<String>), String> {
        let reader = execution.session_reader().map_err(|error| reason(&error))?;
        let me = (!execution.session_id.is_empty()).then_some(execution.session_id.as_str());
        let hidden = hidden_session_ids(reader, me, all);
        let now = now_ms();

        let mut running: Vec<SessionActivityRow> = self
            .sessions
            .activity_snapshot()
            .into_iter()
            .filter(|row| row.status != SessionStatus::Idle)
            .filter(|row| !hidden.contains(&row.session_id))
            .collect();
        running.sort_by(|a, b| {
            turn_elapsed_ms(b, now)
                .cmp(&turn_elapsed_ms(a, now))
                .then_with(|| a.session_id.cmp(&b.session_id))
        });
        let running_total = running.len();

        let running_ids: HashSet<String> =
            running.iter().map(|row| row.session_id.clone()).collect();
        let mut recent: Vec<(String, Option<String>, i64, String, String)> = reader
            .list_session_activity_blocking(now - RECENT_WINDOW_MS)
            .map_err(|error| reason(&error))?
            .into_iter()
            .filter(|(id, ..)| !hidden.contains(id) && !running_ids.contains(id))
            .collect();
        recent.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        let recent_total = recent.len();

        let total = reader
            .list_session_ids_blocking()
            .map_err(|error| reason(&error))?
            .len();

        let running_shown = running_total.min(running_limit);
        running.truncate(running_shown);
        let recent_shown = recent_total.min(recent_limit);
        recent.truncate(recent_shown);

        let mut out = format!("- running ({running_total}):\n");
        if running.is_empty() {
            out.push_str("  - none\n");
        }
        for row in &running {
            out.push_str(&format!("  - {}\n", running_row(reader, row, now, me)));
        }
        if running_total > running_shown {
            out.push_str(&format!(
                "  - … and {} more running (showing the {running_shown} longest running sessions)\n",
                running_total - running_shown
            ));
        }
        out.push_str(&format!(
            "- idle (last {}m, {recent_total}):\n",
            RECENT_WINDOW_MS / 60_000
        ));
        if recent.is_empty() {
            out.push_str("  - none\n");
        }
        for (id, parent, updated_at, agent, last_message) in &recent {
            out.push_str(&format!(
                "  - {}\n",
                recent_row(
                    id,
                    parent.as_deref(),
                    *updated_at,
                    agent,
                    last_message,
                    now,
                    me,
                )
            ));
        }
        if recent_total > recent_shown {
            out.push_str(&format!(
                "  - … and {} more idle sessions in the last {}m\n",
                recent_total - recent_shown,
                RECENT_WINDOW_MS / 60_000
            ));
        }
        out.push_str(&format!("- total: {total} sessions in this workspace\n"));
        if running_total > running_shown || recent_total > recent_shown {
            out.push_str(
                "- next: `sessions --limit N`; your own children stay on `subagent_list`\n",
            );
        }

        let warning = (running_total > running_shown).then(|| {
            format!(
                "{running_total} running sessions in this workspace; only the {running_shown} \
                 longest are listed — use `subagent_list` for your own children, or \
                 `sessions --limit N`."
            )
        });
        Ok((out, warning))
    }

    fn mcp_snapshots(&self) -> HashMap<String, McpServerSnapshot> {
        let pool = Arc::clone(&self.runtime.mcp_pool);
        let hub = Arc::clone(&pool);
        pool.block_on_hub(async move { hub.snapshots().await })
            .unwrap_or_default()
    }

    fn refresh(&self, target: Option<RefreshTarget>) -> ToolCallResult {
        let mut out = String::new();
        let mut warnings: Vec<String> = Vec::new();
        let targets: Vec<RefreshTarget> = match target {
            Some(target) => vec![target],
            None => vec![
                RefreshTarget::Excludes,
                RefreshTarget::Mcp,
                RefreshTarget::CustomTools,
            ],
        };
        let mut blocks = Vec::new();
        for target in &targets {
            let (body, mut found) = match target {
                RefreshTarget::Excludes => self.refresh_excludes(),
                RefreshTarget::Mcp => self.refresh_mcp(),
                RefreshTarget::CustomTools => self.refresh_custom_tools(),
            };
            blocks.push(body.trim_end().to_string());
            warnings.append(&mut found);
        }
        out.push_str(&blocks.join("\n\n---\n\n"));
        out.push('\n');
        let result = ToolCallResult::ok(out);
        if warnings.is_empty() {
            result
        } else {
            result.with_warning(warnings.join(" · "))
        }
    }

    fn refresh_excludes(&self) -> (String, Vec<String>) {
        let root = self.runtime.workspace_root();
        let mut out = String::from("# refresh excludes\n\n");
        let mut warnings: Vec<String> = Vec::new();
        let active = active_workspace_excludes();
        let exists = workspace_excludes_path(root).exists();
        if exists && read_workspace_excludes(root).is_err() {
            out.push_str(&format!(
                "- file: `.litecode/excludes.json`\n- active: {}\n- {FILE_BROKEN}\n",
                excludes_counts(&active)
            ));
            warnings.push("excludes.json is broken; the counts above are what is in effect".into());
            return (out, warnings);
        }
        out.push_str(&format!(
            "- file: `.litecode/excludes.json`{}\n- active: {}\n",
            if exists {
                ""
            } else {
                " (missing — builtin defaults)"
            },
            excludes_counts(&active)
        ));
        (out, warnings)
    }

    fn refresh_mcp(&self) -> (String, Vec<String>) {
        let root = self.runtime.workspace_root();
        let mut out = String::from("# refresh mcp\n\n");
        let mut warnings: Vec<String> = Vec::new();
        let file = match read_workspace_mcp(root) {
            Ok(file) => file,
            Err(_error) => {
                out.push_str(&format!("- file: `.litecode/mcp.json`\n- {FILE_BROKEN}\n"));
                warnings.push("mcp.json is broken".into());
                return (out, warnings);
            }
        };
        let mut ids: Vec<String> = file.servers.keys().cloned().collect();
        ids.sort();
        out.push_str(&format!(
            "- file: `.litecode/mcp.json` — {} server(s)\n",
            ids.len()
        ));
        if ids.is_empty() {
            out.push_str("- none in this file\n");
        }
        let mut valid: HashMap<String, McpServerDefinition> = HashMap::new();
        let mut rejected = 0usize;
        let mut off: Vec<String> = Vec::new();
        let resolved = &self.runtime.resolved;
        for id in &ids {
            let original = file.servers[id].clone();
            let mut checked = original.clone();
            match validate_mcp_definition(id, &mut checked) {
                Ok(()) => {
                    out.push_str(&format!("- `{id}`: ok — `{}`\n", command_line(&original)));
                    if !agent_tool_enabled(resolved, &self.agent_id, &mcp_catalog_id(id)) {
                        off.push(id.clone());
                    }
                    valid.insert(id.clone(), original);
                }
                Err(error) => {
                    rejected += 1;
                    out.push_str(&format!("- `{id}`: rejected — {}\n", reason(&error)));
                }
            }
        }
        let drift = def_drift(resolved.workspace_mcp_servers(), &valid);
        if !drift.is_empty() {
            out.push_str(&format!("- not in effect yet: {drift}\n"));
        }
        out.push_str(&finish_refresh("mcp", rejected, &off));
        if rejected > 0 {
            warnings.push(format!(
                "{rejected} MCP definition(s) would be rejected by the settings page"
            ));
        }
        (out, warnings)
    }

    fn refresh_custom_tools(&self) -> (String, Vec<String>) {
        let root = self.runtime.workspace_root();
        let mut out = String::from("# refresh custom_tools\n\n");
        let mut warnings: Vec<String> = Vec::new();
        let file = match read_workspace_custom_tools(root) {
            Ok(file) => file,
            Err(_error) => {
                out.push_str(&format!(
                    "- file: `.litecode/custom_tools.json`\n- {FILE_BROKEN}\n"
                ));
                warnings.push("custom_tools.json is broken".into());
                return (out, warnings);
            }
        };
        let mut ids: Vec<String> = file.tools.keys().cloned().collect();
        ids.sort();
        out.push_str(&format!(
            "- file: `.litecode/custom_tools.json` — {} tool(s)\n",
            ids.len()
        ));
        if ids.is_empty() {
            out.push_str("- none in this file\n");
        }
        let mut valid: HashMap<String, CustomToolDefinition> = HashMap::new();
        let mut rejected = 0usize;
        let mut off: Vec<String> = Vec::new();
        let resolved = &self.runtime.resolved;
        for id in &ids {
            let original = file.tools[id].clone();
            let mut checked = original.clone();
            match validate_custom_definition(id, &mut checked) {
                Ok(()) => {
                    out.push_str(&format!(
                        "- `{id}`: ok — `{}` · {}\n",
                        command_line_parts(&original.command, &original.args),
                        timeout_label(original.timeout)
                    ));
                    if original.timeout == 0 {
                        out.push_str("  `timeout: 0` fails immediately.\n");
                    }
                    if !agent_tool_enabled(resolved, &self.agent_id, id) {
                        off.push(id.clone());
                    }
                    valid.insert(id.clone(), original);
                }
                Err(error) => {
                    rejected += 1;
                    out.push_str(&format!("- `{id}`: rejected — {}\n", reason(&error)));
                }
            }
        }
        let drift = def_drift(resolved.workspace_custom_tools(), &valid);
        if !drift.is_empty() {
            out.push_str(&format!("- not in effect yet: {drift}\n"));
        }
        out.push_str(&finish_refresh("custom_tools", rejected, &off));
        if rejected > 0 {
            warnings.push(format!(
                "{rejected} custom tool definition(s) would be rejected by the settings page"
            ));
        }
        (out, warnings)
    }
}

impl Tool for LitecodeWorkspaceTool {
    fn name(&self) -> &str {
        "litecode_workspace"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "action": {
                    "type": "string",
                    "description": "CLI-style command. Omit or leave empty to open the panel. Use `validate_custom` with `definition` + `sample_input` to trial a tool without registering it (no rules eval; run_in_background stripped from stdin)."
                },
                "definition": {
                    "type": "object",
                    "description": "Custom tool body for `validate_custom` (name, command, schema, args, timeout, rules)."
                },
                "sample_input": {
                    "type": "object",
                    "description": "JSON object written to custom tool stdin during `validate_custom` (run_in_background is stripped, same as production). Does not evaluate rules."
                },
                "name": {
                    "type": "string",
                    "description": "Optional name when definition.name is empty (validate_custom only)."
                }
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(std::future::ready(self.run(&input, &execution)))
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.run(
            &input,
            &ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: self.runtime.workspace_root().to_path_buf(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: self.max_result_size(),
                session_id: String::new(),
                session: Some(SessionDataReader::open(
                    &self.runtime.workspace.paths.sessions_db,
                )),
            },
        )
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/litecode_workspace.md"))
    }
}

fn section(title: &str, body: Result<String, String>) -> String {
    match body {
        Ok(body) => format!("## {title}\n\n{body}"),
        Err(error) => format!("## {title}\n\n{}", unavailable(&error)),
    }
}

fn unavailable(error: &str) -> String {
    format!("- unavailable: {error}\n")
}

/// User-facing reason: validation and tool errors read better without the
/// `config error: ` / `tool execution error: ` kind prefix.
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

fn command_line(def: &McpServerDefinition) -> String {
    command_line_parts(&def.command, &def.args)
}

fn command_line_parts(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        command.to_string()
    } else {
        format!("{command} {}", args.join(" "))
    }
}

fn timeout_label(timeout: u64) -> String {
    format!("timeout {timeout}s")
}

fn gate_label(on: bool) -> &'static str {
    if on { "on for you" } else { "off for you" }
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

/// Passed files take effect when the workspace is idle. Rejections stay a
/// separate next. Switches that are off are named on their own line.
fn finish_refresh(topic: &str, rejected: usize, off: &[String]) -> String {
    if rejected > 0 {
        return format!("- next: fix the rejected entries and run `refresh {topic}` again\n");
    }
    let mut out = format!("- next: {APPLIES_WHEN_IDLE}\n");
    if !off.is_empty() {
        let names: Vec<String> = off
            .iter()
            .map(|id| {
                if topic == "mcp" {
                    mcp_catalog_id(id)
                } else {
                    id.clone()
                }
            })
            .collect();
        out.push_str(&format!("- {}\n", off_for_you_line(&names)));
    }
    out
}

fn off_for_you_line(names: &[String]) -> String {
    let listed = names
        .iter()
        .map(|id| format!("`{id}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let pronoun = if names.len() == 1 { "it" } else { "them" };
    format!(
        "{listed} off for you. If you actually need {pronoun}, ask a human to enable {pronoun} \
         in Settings → Agents. Otherwise stay silent."
    )
}

fn excludes_counts(file: &WorkspaceExcludesFile) -> String {
    format!(
        "files {} · search {} · watcher {} · git_ignore {} · explorer_git_ignore {}",
        file.files_exclude.len(),
        file.search_exclude.len(),
        file.watcher_exclude.len(),
        file.git_ignore,
        file.explorer_git_ignore
    )
}

fn def_drift<T: PartialEq>(before: &HashMap<String, T>, disk: &HashMap<String, T>) -> String {
    let mut ids: Vec<&String> = before.keys().chain(disk.keys()).collect();
    ids.sort();
    ids.dedup();
    let mut parts: Vec<String> = Vec::new();
    for id in ids {
        match (before.get(id), disk.get(id)) {
            (None, Some(_)) => parts.push(format!("`{id}` added on disk")),
            (Some(_), None) => parts.push(format!("`{id}` removed on disk")),
            (Some(old), Some(new)) if old != new => parts.push(format!("`{id}` changed on disk")),
            _ => {}
        }
    }
    parts.join(" · ")
}

/// Self, self's ancestor chain, and self's whole subtree (best effort).
fn hidden_session_ids(
    reader: &SessionDataReader,
    me: Option<&str>,
    include_self_and_ancestors: bool,
) -> HashSet<String> {
    let mut hidden: HashSet<String> = HashSet::new();
    let Some(me) = me else {
        return hidden;
    };
    if !include_self_and_ancestors {
        hidden.insert(me.to_string());
        let mut cursor = me.to_string();
        for _ in 0..8 {
            let Ok(meta) = reader.meta_blocking(&cursor) else {
                break;
            };
            match meta.parent_session_id {
                Some(parent) => {
                    if !hidden.insert(parent.clone()) {
                        break;
                    }
                    cursor = parent;
                }
                None => break,
            }
        }
    }
    let mut queue = vec![me.to_string()];
    while let Some(parent) = queue.pop() {
        if hidden.len() > MAX_SUBTREE_NODES {
            break;
        }
        if let Ok(children) = reader.list_child_ids_blocking(&parent) {
            for child in children {
                if hidden.insert(child.clone()) {
                    queue.push(child);
                }
            }
        }
    }
    hidden
}

fn turn_elapsed_ms(row: &SessionActivityRow, now: i64) -> i64 {
    row.turn_started_at_ms.map(|start| now - start).unwrap_or(0)
}

fn running_row(
    reader: &SessionDataReader,
    row: &SessionActivityRow,
    now: i64,
    me: Option<&str>,
) -> String {
    let mut parts = vec![format!("`{}`", short_session_ref(&row.session_id))];
    if me == Some(row.session_id.as_str()) {
        parts.push("it's you".to_string());
    }
    if !row.agent_id.is_empty() {
        parts.push(row.agent_id.clone());
    }
    if let Some(started) = row.turn_started_at_ms {
        parts.push(format!("turn {}", duration_label(now - started)));
    }
    if let Some((step, step_max)) = row.step {
        parts.push(format!("step {step}/{step_max}"));
    }
    if row.status == SessionStatus::Stopping {
        parts.push("stopping".to_string());
    }
    if let Some(model) = row.model_id.as_deref().filter(|model| !model.is_empty()) {
        parts.push(model.to_string());
    }
    let mut line = parts.join(" · ");
    if let Ok(meta) = reader.meta_blocking(&row.session_id) {
        let preview = one_line_preview(&meta.preview);
        if !preview.is_empty() {
            line.push_str(&format!(" · “{preview}”"));
        }
    }
    line
}

fn recent_row(
    id: &str,
    parent: Option<&str>,
    updated_at: i64,
    agent: &str,
    last_message: &str,
    now: i64,
    me: Option<&str>,
) -> String {
    let mut parts = vec![format!("`{}`", short_session_ref(id))];
    if me == Some(id) {
        parts.push("it's you".to_string());
    }
    let who = match parent {
        Some(parent) => format!("{agent}, subagent of {}", short_session_ref(parent)),
        None => agent.to_string(),
    };
    if !who.is_empty() {
        parts.push(who);
    }
    parts.push(format!("idle {} ago", duration_label(now - updated_at)));
    let mut line = parts.join(" · ");
    let preview = one_line_preview(last_message);
    if !preview.is_empty() {
        line.push_str(&format!(" · “{preview}”"));
    }
    line
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn duration_label(elapsed_ms: i64) -> String {
    let seconds = elapsed_ms.max(0) as u64 / 1000;
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m{:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h{:02}m", seconds / 3600, (seconds % 3600) / 60),
    }
}

/// Collapse a stored user-message preview onto one short line.
fn one_line_preview(raw: &str) -> String {
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = collapsed.chars().take(MAX_PREVIEW_CHARS).collect();
    if collapsed.chars().count() > MAX_PREVIEW_CHARS {
        out.push('…');
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnGuard;
    use crate::config::resolved::{WorkspaceState, resolve};
    use crate::config::schema::{
        AgentProfile, AgentToolBinding, CustomToolDefinition, GlobalSettings, McpTransport,
        ToolSchema,
    };
    use crate::ide_base::IdeBaseHandle;
    use crate::optional::EngineManager;
    use crate::provider_catalog::{DEFAULT_CATALOG, ProviderCatalog};
    use crate::session::data::command::{MutationId, SessionMutation};
    use crate::types::{ToolSignalLevel, user_text};
    use crate::workspace::filter::write_workspace_excludes;
    use std::path::Path;
    use std::sync::atomic::AtomicU64;

    fn embedded_catalog() -> Arc<ProviderCatalog> {
        Arc::new(
            ProviderCatalog::parse(DEFAULT_CATALOG, Path::new("provider-catalog.toml"))
                .expect("embedded catalog"),
        )
    }

    fn test_runtime(root: &Path, global: GlobalSettings) -> (RuntimeHandle, Arc<SessionManager>) {
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            root.join(".litecode")
                .join("sessions.db")
                .to_str()
                .unwrap()
                .to_string(),
        ));
        let resolved = resolve(global, WorkspaceState::new(root), embedded_catalog());
        let engines = crate::engines::WorkspaceEngines::new();
        let ide = IdeBaseHandle::open(resolved.workspace_root(), Arc::new(engines.clone()))
            .expect("ide base");
        let runtime = RuntimeHandle::new(
            resolved,
            "default".into(),
            WorkspaceState::new(root),
            Arc::new(EngineManager::new()),
            Arc::new(engines),
            ide,
            Arc::new(AtomicU64::new(0)),
            root.join("global.db"),
        );
        (runtime, sessions)
    }

    fn tool_for(
        root: &Path,
        global: GlobalSettings,
    ) -> (LitecodeWorkspaceTool, Arc<SessionManager>) {
        let (runtime, sessions) = test_runtime(root, global);
        (
            LitecodeWorkspaceTool::new(runtime, Arc::clone(&sessions), "default".into()),
            sessions,
        )
    }

    fn execution(root: &Path, sessions: &Arc<SessionManager>, me: &str) -> ToolExecutionContext {
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: root.to_path_buf(),
            call_id: String::new(),
            cancel: tokio_util::sync::CancellationToken::new(),
            output_limit: 12_000,
            session_id: me.to_string(),
            session: Some(sessions.reader()),
        }
    }

    fn run(
        tool: &LitecodeWorkspaceTool,
        execution: &ToolExecutionContext,
        action: &str,
    ) -> ToolCallResult {
        let input = if action.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::json!({ "action": action })
        };
        tool.run(&input, execution)
    }

    async fn create_session(root: &Path, sessions: &Arc<SessionManager>, text: &str) -> String {
        let id = sessions
            .open_session(root.to_str().unwrap(), "default", None)
            .await
            .unwrap();
        sessions
            .data()
            .insert_items(&id, &[user_text(text)])
            .unwrap();
        id
    }

    fn create_child(
        root: &Path,
        sessions: &Arc<SessionManager>,
        parent: &str,
        text: &str,
    ) -> String {
        let id = sessions
            .data()
            .mutate_blocking(SessionMutation::Create {
                operation_id: MutationId::new(),
                project: root.to_str().unwrap().into(),
                agent_id: "default".into(),
                model_id: None,
                parent_session_id: Some(parent.to_string()),
                parent_call_id: Some(format!("call_{parent}")),
                responsibility: String::new(),
            })
            .unwrap()
            .session_id;
        sessions
            .data()
            .insert_items(&id, &[user_text(text)])
            .unwrap();
        id
    }

    #[test]
    fn parser_accepts_the_documented_commands() {
        assert_eq!(parse_action(""), Ok(Action::Panel));
        assert_eq!(parse_action("   "), Ok(Action::Panel));
        assert_eq!(parse_action("status"), Ok(Action::Status));
        assert_eq!(parse_action("help"), Ok(Action::Panel));
        assert_eq!(
            parse_action("help refresh"),
            Ok(Action::Help("refresh".into()))
        );
        assert_eq!(
            parse_action("sessions --all --limit 3"),
            Ok(Action::Sessions {
                all: true,
                limit: Some(3)
            })
        );
        assert_eq!(
            parse_action("guide mcp"),
            Ok(Action::Guide(Some(GuideTopic::Mcp)))
        );
        assert_eq!(
            parse_action("refresh custom_tools"),
            Ok(Action::Refresh(Some(RefreshTarget::CustomTools)))
        );
        assert!(parse_action("guide provider").is_err());
        assert!(parse_action("refresh provider").is_err());
        assert_eq!(parse_action("refresh all"), Ok(Action::Refresh(None)));
        assert!(parse_action("seed").is_err());
    }

    #[test]
    fn parser_rejects_unknown_input_with_the_buttons() {
        let error = parse_action("statuz").unwrap_err();
        assert!(error.contains("unknown command 'statuz'"), "{error}");
        assert!(error.contains("# Buttons"), "{error}");
        assert!(parse_action("sessions --limit 0").is_err());
        assert!(parse_action("sessions --limit x").is_err());
        assert!(parse_action("sessions --wat").is_err());
        assert!(parse_action("guide nope").is_err());
        assert!(parse_action("refresh nope").is_err());
        assert!(parse_action("mcp").is_err());
        assert!(parse_action("mcp restart").is_err());
    }

    #[test]
    fn unknown_command_result_is_an_error_with_the_buttons() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, sessions) = tool_for(dir.path(), GlobalSettings::default());
        let result = run(&tool, &execution(dir.path(), &sessions, ""), "statuz");
        assert_eq!(result.level, ToolSignalLevel::Error);
        assert!(result.content.contains("unknown command 'statuz'"));
        assert!(
            result
                .content
                .contains("| `sessions [--all] [--limit N]` |")
        );
    }

    #[test]
    fn panel_lists_current_state_and_marks_one_off_switch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut global = GlobalSettings::default();
        global.mcp_servers.insert(
            "docs".into(),
            McpServerDefinition {
                command: "npx".into(),
                args: vec!["-y".into(), "docs-server".into()],
                env: HashMap::new(),
                transport: McpTransport::Stdio,
                timeout: 60,
            },
        );
        let (tool, sessions) = tool_for(root, global);
        let result = run(&tool, &execution(root, &sessions, ""), "");
        assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
        for expected in [
            "# Workspace",
            "# Buttons",
            "\n---\n",
            "## excludes",
            "## mcp",
            "## custom tools",
            "## sessions",
            "`docs` · stopped · off for you · global",
            "- next: If you actually need it, ask a human to enable it in Settings → Agents. Otherwise stay silent.",
            "- running (0):",
            "total: 0 sessions in this workspace",
        ] {
            assert!(
                result.content.contains(expected),
                "missing {expected}:\n{}",
                result.content
            );
        }
        assert_eq!(
            result.content.matches("ask a human").count(),
            1,
            "the human gate is one line:\n{}",
            result.content
        );
        assert!(
            !result.content.contains("npx"),
            "the launch command stays off the panel:\n{}",
            result.content
        );
        assert!(
            !result.content.contains("- on disk:") && !result.content.contains("- drift:"),
            "a quiet file does not print a diff:\n{}",
            result.content
        );
        assert!(
            !result.content.contains("provider config")
                && !result.content.contains("provider-catalog"),
            "the panel does not talk about the provider catalog:\n{}",
            result.content
        );
        let status = run(&tool, &execution(root, &sessions, ""), "status");
        assert!(
            !status.content.contains("# Buttons"),
            "status reprints the picture without the button table:\n{}",
            status.content
        );
    }

    #[test]
    fn disk_mcp_drift_shows_on_the_panel_without_the_launch_command() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".litecode")).unwrap();
        std::fs::write(
            root.join(".litecode").join("mcp.json"),
            r#"{
                "version": 1,
                "servers": { "docs": { "command": "npx", "args": ["-y", "docs-server"] } }
            }"#,
        )
        .unwrap();
        let (tool, sessions) = tool_for(root, GlobalSettings::default());
        let result = run(&tool, &execution(root, &sessions, ""), "status");
        assert!(
            result.content.contains("- drift: `docs` added on disk"),
            "{}",
            result.content
        );
        assert!(
            result
                .content
                .contains(&format!("- next: {APPLIES_WHEN_IDLE}")),
            "{}",
            result.content
        );
        assert!(!result.content.contains("npx"), "{}", result.content);
    }

    #[test]
    fn an_enabled_custom_tool_omits_the_human_line_until_disk_drifts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut global = GlobalSettings::default();
        global.custom_tools.push(CustomToolDefinition {
            name: "demo".into(),
            description: String::new(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "demo".into(),
            args: vec!["--json".into()],
            timeout: 120,
            rules: Vec::new(),
            suite: None,
            suite_label: None,
        });
        let mut agent = AgentProfile::default();
        agent.tools.insert(
            "demo".into(),
            AgentToolBinding {
                enabled: true,
                policy: crate::permission::ToolPolicy::allow_all(),
                path_mode: crate::permission::BindingPathMode::default(),
                last_applied_preset: None,
                allowed_tools: None,
            },
        );
        global.agents.insert("default".into(), agent);
        let (tool, sessions) = tool_for(root, global);
        let quiet = run(&tool, &execution(root, &sessions, ""), "status");
        assert!(
            quiet
                .content
                .contains("`demo` · `demo --json` · timeout 120s · on for you · global"),
            "{}",
            quiet.content
        );
        assert!(
            !quiet.content.contains("ask a human"),
            "every switch that is on stays quiet:\n{}",
            quiet.content
        );
        assert!(!quiet.content.contains("- drift:"), "{}", quiet.content);

        std::fs::create_dir_all(root.join(".litecode")).unwrap();
        std::fs::write(
            root.join(".litecode").join("custom_tools.json"),
            r#"{
                "version": 1,
                "tools": {
                    "demo": {
                        "name": "demo",
                        "schema": { "type": "object", "properties": {} },
                        "command": "other",
                        "timeout": 30
                    }
                }
            }"#,
        )
        .unwrap();
        let drifted = run(&tool, &execution(root, &sessions, ""), "status");
        assert!(
            drifted.content.contains("- drift: `demo` added on disk"),
            "{}",
            drifted.content
        );
        assert!(
            drifted
                .content
                .contains(&format!("- next: {APPLIES_WHEN_IDLE}")),
            "{}",
            drifted.content
        );
        assert!(
            !drifted.content.contains("other"),
            "the disk command stays off the panel:\n{}",
            drifted.content
        );
    }

    #[tokio::test]
    async fn sessions_section_lists_running_and_hides_self_and_subtree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (tool, sessions) = tool_for(root, GlobalSettings::default());
        let me = create_session(root, &sessions, "mine").await;
        let other = create_session(root, &sessions, "other work").await;
        let mine_child = create_child(root, &sessions, &me, "my helper");
        let other_child = create_child(root, &sessions, &other, "their helper");
        sessions
            .reserve_turn(&other, "t1".into(), 25, "reviewer", root.to_str().unwrap())
            .unwrap();

        let result = run(&tool, &execution(root, &sessions, &me), "sessions");
        assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
        assert!(
            result.content.contains("running (1):"),
            "{}",
            result.content
        );
        assert!(
            result.content.contains(short_session_ref(&other)),
            "other session missing:\n{}",
            result.content
        );
        assert!(
            result.content.contains("turn ") && result.content.contains("step 1/25"),
            "turn/step missing:\n{}",
            result.content
        );
        assert!(
            !result.content.contains(short_session_ref(&me)),
            "self must be hidden by default:\n{}",
            result.content
        );
        assert!(
            !result.content.contains("it's you"),
            "a hidden self is not labeled:\n{}",
            result.content
        );
        assert!(
            !result.content.contains(short_session_ref(&mine_child)),
            "own subtree must stay hidden:\n{}",
            result.content
        );
        assert!(
            result.content.contains(short_session_ref(&other_child)),
            "another session's child must be listed:\n{}",
            result.content
        );
        assert!(
            result
                .content
                .contains("total: 4 sessions in this workspace"),
            "{}",
            result.content
        );

        let all = run(&tool, &execution(root, &sessions, &me), "sessions --all");
        assert!(
            all.content
                .contains(&format!("`{}` · it's you", short_session_ref(&me))),
            "--all must include self and mark the row:\n{}",
            all.content
        );
        assert!(
            !all.content.contains(short_session_ref(&mine_child)),
            "--all must still hide the own subtree:\n{}",
            all.content
        );
    }

    #[tokio::test]
    async fn sessions_limit_truncates_and_warns() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (tool, sessions) = tool_for(root, GlobalSettings::default());
        let me = create_session(root, &sessions, "me").await;
        for index in 0..4 {
            let id = create_session(root, &sessions, &format!("other {index}")).await;
            sessions
                .reserve_turn(
                    &id,
                    format!("t{index}"),
                    10,
                    "reviewer",
                    root.to_str().unwrap(),
                )
                .unwrap();
        }
        let result = run(
            &tool,
            &execution(root, &sessions, &me),
            "sessions --limit 2",
        );
        assert_eq!(result.level, ToolSignalLevel::Warning, "{}", result.content);
        assert!(
            result.content.contains("running (4):"),
            "{}",
            result.content
        );
        assert!(
            result
                .content
                .contains("… and 2 more running (showing the 2 longest running sessions)"),
            "{}",
            result.content
        );
        assert!(
            result.content.contains(
                "- next: `sessions --limit N`; your own children stay on `subagent_list`"
            ),
            "{}",
            result.content
        );
        assert!(
            result
                .warning_status
                .as_deref()
                .is_some_and(|warning| warning.contains("4 running sessions")),
            "saturation warning missing: {:?}",
            result.warning_status
        );
    }

    #[test]
    fn guide_prints_the_index_then_one_topic() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, sessions) = tool_for(dir.path(), GlobalSettings::default());
        let index = run(&tool, &execution(dir.path(), &sessions, ""), "guide");
        assert!(index.content.contains("# guide"));
        assert!(index.content.contains("`guide mcp`"));
        assert!(!index.content.contains("`guide provider`"));
        assert!(index.content.contains("Settings → Agents"));
        assert!(!index.content.contains("writes definitions"));

        let topic = run(&tool, &execution(dir.path(), &sessions, ""), "guide mcp");
        assert_eq!(topic.content, guides::topic(GuideTopic::Mcp));
        assert!(topic.content.contains("**怎么验证。**"));
    }

    #[test]
    fn refresh_reports_invalid_mcp_definitions_outside_the_accepted_diff() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".litecode")).unwrap();
        std::fs::write(
            root.join(".litecode").join("mcp.json"),
            r#"{
                "version": 1,
                "servers": {
                    "Good": { "command": "npx" },
                    "empty": { "command": "  " },
                    "bash": { "command": "npx" }
                }
            }"#,
        )
        .unwrap();
        let (tool, sessions) = tool_for(root, GlobalSettings::default());
        let result = run(&tool, &execution(root, &sessions, ""), "refresh mcp");
        assert_eq!(result.level, ToolSignalLevel::Warning, "{}", result.content);
        assert!(
            result
                .content
                .contains("`Good`: rejected — invalid id 'Good'"),
            "{}",
            result.content
        );
        assert!(
            result
                .content
                .contains("`empty`: rejected — MCP stdio server command must not be empty"),
            "{}",
            result.content
        );
        assert!(
            result
                .content
                .contains("`bash`: rejected — MCP server id 'bash' conflicts with a builtin tool"),
            "{}",
            result.content
        );
        assert!(
            !result.content.contains("not in effect yet"),
            "rejected ids are not an accepted diff:\n{}",
            result.content
        );
        assert!(
            result
                .content
                .contains("fix the rejected entries and run `refresh mcp` again"),
            "{}",
            result.content
        );
        assert!(
            result.warning_status.as_deref().is_some_and(|warning| {
                warning.contains("3 MCP definition(s) would be rejected")
            }),
            "{:?}",
            result.warning_status
        );
    }

    #[test]
    fn refresh_reports_broken_json_without_claiming_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".litecode")).unwrap();
        std::fs::write(root.join(".litecode").join("mcp.json"), "{ not json").unwrap();
        std::fs::write(
            root.join(".litecode").join("custom_tools.json"),
            "{ not json",
        )
        .unwrap();
        let (tool, sessions) = tool_for(root, GlobalSettings::default());
        let result = run(&tool, &execution(root, &sessions, ""), "refresh");
        assert_eq!(result.level, ToolSignalLevel::Warning, "{}", result.content);
        assert!(result.content.contains(FILE_BROKEN), "{}", result.content);
        assert!(
            !result.content.contains("this turn") && !result.content.contains("reloaded"),
            "{}",
            result.content
        );
    }

    #[test]
    fn refresh_reports_custom_tool_rules_without_rewriting_timeout_zero() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".litecode")).unwrap();
        std::fs::write(
            root.join(".litecode").join("custom_tools.json"),
            r#"{
                "version": 1,
                "tools": {
                    "demo": {
                        "name": "other",
                        "schema": { "type": "object", "properties": {} },
                        "command": "demo",
                        "timeout": 0
                    },
                    "zero": {
                        "name": "zero",
                        "schema": { "type": "object", "properties": {} },
                        "command": "demo",
                        "timeout": 0
                    }
                }
            }"#,
        )
        .unwrap();
        let (tool, sessions) = tool_for(root, GlobalSettings::default());
        let result = run(
            &tool,
            &execution(root, &sessions, ""),
            "refresh custom_tools",
        );
        assert!(
            result.content.contains(
                "`demo`: rejected — custom tool body name 'other' must match path id 'demo'"
            ),
            "{}",
            result.content
        );
        assert!(
            result.content.contains("`zero`: ok — `demo` · timeout 0s"),
            "{}",
            result.content
        );
        assert!(
            result.content.contains("`timeout: 0` fails immediately."),
            "{}",
            result.content
        );
        assert!(
            !result.content.contains("120"),
            "stored 0 must not be printed as the normalized 120:\n{}",
            result.content
        );
        assert!(
            result
                .content
                .contains("not in effect yet: `zero` added on disk"),
            "{}",
            result.content
        );
    }

    #[test]
    fn refresh_excludes_reports_what_is_in_effect() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".litecode")).unwrap();
        let mut file = WorkspaceExcludesFile::builtin_defaults();
        file.search_exclude.push("**/dist".into());
        write_workspace_excludes(root, file).unwrap();
        crate::workspace::filter::activate_workspace_excludes(
            WorkspaceExcludesFile::builtin_defaults(),
        );
        let before = active_workspace_excludes();
        let (tool, sessions) = tool_for(root, GlobalSettings::default());
        let result = run(&tool, &execution(root, &sessions, ""), "refresh excludes");
        assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
        assert!(result.content.contains("- active:"), "{}", result.content);
        assert!(
            !result.content.contains("- on disk:") && !result.content.contains("- drift:"),
            "a file the watcher has not caught up with is not a drift report:\n{}",
            result.content
        );
        assert!(
            !result.content.contains(FILE_BROKEN),
            "a valid file is not broken:\n{}",
            result.content
        );
        assert_eq!(
            active_workspace_excludes(),
            before,
            "refresh must not publish the file into the process cache"
        );
        std::fs::write(root.join(".litecode").join("excludes.json"), "{").unwrap();
        let broken = run(&tool, &execution(root, &sessions, ""), "status");
        assert!(
            broken.content.contains(FILE_BROKEN) && broken.content.contains("- active:"),
            "{}",
            broken.content
        );
        assert!(
            !broken.content.contains("- drift:"),
            "a broken excludes file is not drift:\n{}",
            broken.content
        );
        let broken_refresh = run(&tool, &execution(root, &sessions, ""), "refresh excludes");
        assert_eq!(
            broken_refresh.level,
            ToolSignalLevel::Warning,
            "{}",
            broken_refresh.content
        );
        assert!(
            broken_refresh.content.contains(FILE_BROKEN),
            "{}",
            broken_refresh.content
        );
        assert_eq!(active_workspace_excludes(), before);
        crate::workspace::filter::activate_workspace_excludes(
            WorkspaceExcludesFile::builtin_defaults(),
        );
    }

    #[test]
    fn read_commands_are_concurrency_safe() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, _sessions) = tool_for(dir.path(), GlobalSettings::default());
        assert!(tool.is_concurrency_safe(&serde_json::json!({})));
        assert!(tool.is_concurrency_safe(&serde_json::json!({ "action": "status" })));
        assert!(tool.is_concurrency_safe(&serde_json::json!({ "action": "refresh" })));
        assert!(tool.is_concurrency_safe(&serde_json::json!({ "action": "guide mcp" })));
    }
}
