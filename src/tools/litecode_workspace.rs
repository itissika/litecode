//! Agent `litecode_workspace` — the workspace facade.
//!
//! One `action` string in command-line form. No argument opens the panel:
//! current state, plus the buttons that open the next popup. Every command
//! reads. This tool never writes a config file and never enables a tool.
//! A valid excludes file is already in effect. MCP and custom tools take
//! effect at the start of the next turn.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::config::global_db::tools::mcp_catalog_id;
use crate::config::guides::{self, GuideTopic};
use crate::config::schema::{CallSurface, CustomToolDefinition, McpServerDefinition, ToolOrigin};
use crate::config::settings_writer::{validate_custom_definition, validate_mcp_definition};
use crate::config::workspace::{read_workspace_custom_tools, read_workspace_mcp};
use crate::context_pipeline::Context;
use crate::engines::session_search::short_session_ref;
use crate::mcp::{McpRunState, McpServerSnapshot};
use crate::runtime::RuntimeHandle;
use crate::session::SessionDataReader;
use crate::session::manager::{SessionActivityRow, SessionManager, SessionStatus};
use crate::tool::Tool;
use crate::tool::availability::{agent_tool_enabled, is_available};
use crate::tool::project::project_tools;
use crate::tool::registry::builtin_code_surface_tools;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::custom::{CustomTool, validate_custom_tool_run};
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
/// A definitions file that passed validation applies at the next turn start.
const APPLIES_NEXT_TURN: &str = "Takes effect at the start of the next turn. Tell the human.";
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
               custom tools take effect at the start of the next turn. \
               A file that fails validation names the error; fix it and run `refresh` again.",
    },
    CommandSpec {
        name: "validate_custom",
        usage: "validate_custom  (also pass definition + sample_input fields)",
        summary: "run a custom tool once without registering it",
        help: "Set `action` to `validate_custom`, plus `definition` (custom tool body) and `
               sample_input` (stdin JSON object). It validates the definition and, for
               `surface: code`, the run_code admission contract before running the process.
               Does not write files, enable the tool, or evaluate `rules` (PASS covers
               process/envelope and code admission only). Strips `run_in_background`
               from sample stdin like production.",
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

fn call_surface_label(surface: CallSurface) -> &'static str {
    match surface {
        CallSurface::Direct => "surface=direct",
        CallSurface::Code => "surface=code",
    }
}

fn code_binding_id(tool_name: &str) -> &str {
    if crate::tools::lsp::LSP_CODE_TOOL_NAMES.contains(&tool_name) {
        return crate::tools::lsp::LSP_CARD_ID;
    }
    match tool_name {
        "wait" | "kill" => "run",
        "apply_edits" => "edit",
        _ => tool_name,
    }
}

fn code_composition_error(
    resolved: &crate::config::resolved::ResolvedConfig,
    agent_id: &str,
    hub: Arc<crate::tools::custom_hub::CustomToolHub>,
    def: &CustomToolDefinition,
) -> Option<String> {
    if def.surface != CallSurface::Code {
        return None;
    }
    let mut candidates: Vec<Arc<dyn Tool>> = builtin_code_surface_tools()
        .into_iter()
        .filter(|tool| {
            let binding = code_binding_id(tool.name());
            agent_tool_enabled(resolved, agent_id, binding)
                && (binding != crate::tools::lsp::LSP_CARD_ID || is_available(resolved, binding))
        })
        .collect();
    for existing in resolved.custom_tools() {
        if existing.surface == CallSurface::Code
            && existing.name != def.name
            && agent_tool_enabled(resolved, agent_id, &existing.name)
        {
            candidates.push(Arc::new(CustomTool::new(
                existing,
                Arc::clone(&hub),
            )) as Arc<dyn Tool>);
        }
    }
    candidates.push(Arc::new(CustomTool::new(def.clone(), hub)) as Arc<dyn Tool>);
    project_tools(candidates)
        .rejected
        .into_iter()
        .find(|rejected| rejected.name == def.name)
        .map(|rejected| rejected.reason)
}

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

    /// Check SDK type-name collisions against the Code tools this agent can
    /// currently enable. Single-tool admission cannot see this: it is a
    /// composition failure discovered only when the bundle is projected.
    fn code_composition_error(&self, def: &CustomToolDefinition) -> Option<String> {
        code_composition_error(
            &self.runtime.resolved,
            &self.agent_id,
            Arc::clone(&self.runtime.custom_tool_hub),
            def,
        )
    }

    fn run(&self, input: &Value, execution: &ToolExecutionContext) -> ToolCallResult {
        let raw = input.get("action").and_then(Value::as_str).unwrap_or("");
        if raw.trim() == "validate_custom" {
            return match parse_validate_custom(input) {
                Err(message) => ToolCallResult::error(message),
                Ok((mut def, sample_input)) => {
                    let id = def.name.clone();
                    let surface = def.surface;
                    if let Err(error) = validate_custom_definition(&id, &mut def) {
                        return ToolCallResult::error(format!(
                            "# validate_custom — FAIL\n\n- tool: `{id}`\n- {}\n- definition: FAIL\n- detail: {error}\n- next: fix the definition; do not register or enable it until validate_custom passes\n",
                            call_surface_label(surface)
                        ));
                    }
                    if let Some(reason) = self.code_composition_error(&def) {
                        return ToolCallResult::error(format!(
                            "# validate_custom — FAIL\n\n- tool: `{id}`\n- {}\n- code admission: PASS\n- composition: FAIL\n- detail: {reason}\n- next: rename the tool or one of its schema type fields, or disable the colliding Code tool; do not enable both together\n",
                            call_surface_label(surface)
                        ));
                    }
                    let mut result =
                        validate_custom_tool_run(&def, &sample_input, self.runtime.workspace_root());
                    if def.surface == CallSurface::Code && result.error.is_none() {
                        result.content = result.content.replacen(
                            "- code admission: PASS\n",
                            "- code admission: PASS\n- composition: PASS (current enabled and available Code tools)\n",
                            1,
                        );
                    }
                    result
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
                    out.push_str(&format!("- next: {APPLIES_NEXT_TURN}\n"));
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
                "- `{}` · {} · `{}` · {} · {} · {}\n",
                tool.name,
                call_surface_label(tool.surface),
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
                    out.push_str(&format!("- next: {APPLIES_NEXT_TURN}\n"));
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
                        "- `{id}`: ok — {} · `{}` · {}\n",
                        call_surface_label(original.surface),
                        command_line_parts(&original.command, &original.args),
                        timeout_label(original.timeout)
                    ));
                    if original.surface == CallSurface::Code {
                        out.push_str(
                            "  code result: `{ ok: true, text: <stdout/content> }`; `run_in_background` is unavailable.\n",
                        );
                    }
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
                    "description": "Custom tool body for `validate_custom` (name, command, schema, args, timeout, rules, surface). Set surface to direct (default) or code. Code validates run_code admission and returns {ok,text}; direct supports run_in_background."
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
                // ISSUE-042: reuse the session layer's reader; never open sessions.db here.
                session: Some(self.sessions.reader()),
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

/// Passed files take effect at the start of the next turn. Rejections stay a
/// separate next. Switches that are off are named on their own line.
fn finish_refresh(topic: &str, rejected: usize, off: &[String]) -> String {
    if rejected > 0 {
        return format!("- next: fix the rejected entries and run `refresh {topic}` again\n");
    }
    let mut out = format!("- next: {APPLIES_NEXT_TURN}\n");
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
        "files {} · search {} · watcher {} · code_index_extensions {} · git_ignore {} · explorer_git_ignore {}",
        file.files_exclude.len(),
        file.search_exclude.len(),
        file.watcher_exclude.len(),
        file.code_index_extensions.len(),
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
    use crate::config::schema::{
        AgentProfile, AgentToolBinding, GlobalSettings, ToolSchema,
    };
    use crate::config::{ConfigManager, WorkspaceState};
    use crate::permission::ToolPolicy;
    use crate::tools::custom_hub::CustomToolHub;

    fn def(name: &str) -> CustomToolDefinition {
        CustomToolDefinition {
            name: name.into(),
            data_schema: None,
            description: "composition probe".into(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "echo".into(),
            args: Vec::new(),
            timeout: 120,
            rules: Vec::new(),
            surface: CallSurface::Code,
            suite: None,
            suite_label: None,
        }
    }

    fn binding() -> AgentToolBinding {
        AgentToolBinding {
            enabled: true,
            policy: ToolPolicy::allow_all(),
            path_mode: crate::permission::BindingPathMode::default(),
            last_applied_preset: None,
            allowed_tools: None,
        }
    }

    #[test]
    fn composition_check_reports_sdk_type_collision() {
        let dir = tempfile::tempdir().unwrap();
        let mut global = GlobalSettings::default();
        let mut profile = AgentProfile::default();
        profile.tools.insert("foo_bar".into(), binding());
        profile.tools.insert("foo__bar".into(), binding());
        global.agents.insert("default".into(), profile);
        global.custom_tools.push(def("foo__bar"));
        let resolved = ConfigManager::resolve_without_catalog(
            global,
            WorkspaceState::new(dir.path()),
        );
        let reason = code_composition_error(
            &resolved,
            "default",
            Arc::new(CustomToolHub::new()),
            &def("foo_bar"),
        )
        .expect("same PascalCase type must collide");
        assert!(reason.contains("collides"), "{reason}");
    }

    #[test]
    fn composition_check_ignores_direct_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut def = def("direct_probe");
        def.surface = CallSurface::Direct;
        let resolved = ConfigManager::resolve_without_catalog(
            GlobalSettings::default(),
            WorkspaceState::new(dir.path()),
        );
        assert!(
            code_composition_error(
                &resolved,
                "default",
                Arc::new(CustomToolHub::new()),
                &def,
            )
            .is_none()
        );
    }
}
