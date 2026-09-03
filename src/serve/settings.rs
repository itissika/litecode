//! Settings REST API (`/api/settings/*`).

use std::collections::HashMap;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use crate::config::schema::{
    AgentProfile, AvailableTool, CustomToolDefinition, LogSettings, McpServerDefinition,
    ModelDefinition, ProviderAuth, ProviderDefinition, ToolOrigin, ToolPreset,
};
use crate::config::workspace::WorkspaceEnginesFile;
use crate::config::{CommitAck, DocId};
use crate::llm::{
    chat_models_url, has_remote_model_catalog, list_adapters, parse_chat_model_catalog,
};
use crate::mcp::{McpConnectionPool, McpRunState, McpServerSnapshot, McpToolSchema};
use crate::serve::state::ServeState;
use crate::tool::availability::available_tools;
use crate::types::LitecodeError;
use crate::workspace::filter::{
    WorkspaceExcludesFile, WorkspaceExcludesLists, WorkspaceExcludesView,
};

#[derive(Serialize)]
struct ApiOk<T: Serialize> {
    ok: bool,
    #[serde(flatten)]
    data: T,
}

#[derive(Serialize)]
struct ApiErr {
    ok: bool,
    error: String,
}

#[derive(Serialize)]
struct RevisionBody {
    revision: u64,
    docs: Vec<DocId>,
}

#[derive(Serialize)]
struct ProviderWriteResponse {
    revision: u64,
    docs: Vec<DocId>,
    restart_required: bool,
}

#[derive(Deserialize)]
struct ProvidersBody {
    providers: HashMap<String, ProviderDefinition>,
}

#[derive(Deserialize)]
struct WebSearchBody {
    api_key: Option<String>,
}

#[derive(Deserialize)]
struct ModelsBody {
    models: HashMap<String, ModelDefinition>,
}

#[derive(Deserialize)]
struct AgentBody {
    #[serde(flatten)]
    profile: AgentProfile,
}

#[derive(Deserialize, Default)]
struct ScopeQuery {
    #[serde(default)]
    scope: Option<String>,
}

fn workspace_scope(query: &ScopeQuery) -> bool {
    matches!(query.scope.as_deref(), Some("workspace"))
}

#[derive(Deserialize)]
struct LogBody {
    level: Option<String>,
}

#[derive(Serialize)]
struct AgentListItem {
    id: String,
    role: crate::config::schema::AgentRole,
    description: String,
    allowed_subagents: Vec<String>,
}

#[derive(Serialize)]
struct AgentsListBody {
    agents: Vec<AgentListItem>,
}

pub fn router() -> Router<ServeState> {
    Router::new()
        .route("/", get(get_settings))
        .route("/adapters", get(get_adapters))
        .route("/providers", get(get_providers).put(put_providers))
        .route("/providers/{id}/models", get(get_provider_models))
        .route("/websearch", get(get_websearch).put(put_websearch))
        .route("/models", get(get_models).put(put_models))
        .route("/agents", get(list_agents))
        .route(
            "/agents/{id}",
            get(get_agent).put(put_agent).delete(delete_agent),
        )
        .route(
            "/agents/{id}/tools/apply-preset",
            post(apply_agent_tool_preset),
        )
        .route("/available-tools", get(get_available_tools))
        .route("/custom-tools", get(list_custom_tools))
        .route(
            "/custom-tools/{id}",
            get(get_custom_tool)
                .put(put_custom_tool)
                .delete(delete_custom_tool),
        )
        .route("/mcp-servers", get(list_mcp_servers))
        .route(
            "/mcp-servers/{id}",
            get(get_mcp_server)
                .put(put_mcp_server)
                .delete(delete_mcp_server),
        )
        .route("/mcp-servers/{id}/probe", post(start_mcp_server))
        .route("/mcp-servers/{id}/start", post(start_mcp_server))
        .route("/mcp-servers/{id}/stop", post(stop_mcp_server))
        .route("/mcp-servers/{id}/restart", post(restart_mcp_server))
        .route("/log", get(get_log).put(put_log))
        .route("/excludes", get(get_excludes).put(put_excludes))
        .route("/engines", get(get_engines).put(put_engines))
}

async fn get_settings(State(state): State<ServeState>) -> Response {
    match state.settings_writer.summary() {
        Ok(summary) => ok_json(summary),
        Err(e) => settings_error(e),
    }
}

async fn get_adapters(State(_state): State<ServeState>) -> Response {
    ok_json(serde_json::json!({ "adapters": list_adapters() }))
}

async fn get_providers(State(state): State<ServeState>) -> Response {
    match state.settings_writer.providers_view() {
        Ok(providers) => ok_json(serde_json::json!({ "providers": providers })),
        Err(e) => settings_error(e),
    }
}

async fn put_providers(
    State(state): State<ServeState>,
    Json(body): Json<ProvidersBody>,
) -> Response {
    match state.settings_writer.write_providers(body.providers) {
        Ok(ack) => {
            reload_runtime_after_settings_write(&state, &ack, "provider write");
            ok_json(ProviderWriteResponse {
                revision: ack.generation,
                docs: ack.docs,
                restart_required: ack.restart_required,
            })
        }
        Err(e) => settings_write_error(e),
    }
}

async fn get_provider_models(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    let settings = match state.settings_writer.load_settings() {
        Ok(s) => s,
        Err(e) => return settings_error(e),
    };
    let Some(provider) = settings.providers.get(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiErr {
                ok: false,
                error: format!("provider not found: {id}"),
            }),
        )
            .into_response();
    };
    if !has_remote_model_catalog(&provider.adapter_id) {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiErr {
                ok: false,
                error: format!("provider '{id}' has no remote model catalog"),
            }),
        )
            .into_response();
    }
    let endpoint = crate::llm::closed_default_endpoint(&provider.adapter_id)
        .filter(|_| provider.config.endpoint.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| provider.config.endpoint.clone());
    // Strips a trailing `/responses` so Ark Coding Plan catalog stays `{v3}/models`.
    let url = chat_models_url(&endpoint);
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => return catalog_fetch_error(format!("catalog client: {e}")),
    };
    let mut req = client.get(&url);
    let key = provider.config.api_key.trim();
    if !key.is_empty() {
        req = match provider.config.auth {
            ProviderAuth::Bearer => req.header("Authorization", format!("Bearer {key}")),
            ProviderAuth::ApiKey => req.header("api-key", key),
        };
    }
    let response = match req.send().await {
        Ok(r) => r,
        Err(e) => return catalog_fetch_error(format!("catalog fetch failed: {e}")),
    };
    let status = response.status();
    let body = match response.text().await {
        Ok(t) => t,
        Err(e) => return catalog_fetch_error(format!("catalog body: {e}")),
    };
    if !status.is_success() {
        return catalog_fetch_error(format!("HTTP {status}: {body}"));
    }
    match parse_chat_model_catalog(&body) {
        Ok(ids) => ok_json(serde_json::json!({ "ids": ids })),
        Err(e) => catalog_fetch_error(e.to_string()),
    }
}

fn catalog_fetch_error(error: String) -> Response {
    (StatusCode::BAD_GATEWAY, Json(ApiErr { ok: false, error })).into_response()
}

async fn get_websearch(State(state): State<ServeState>) -> Response {
    match state.settings_writer.websearch_view() {
        Ok(view) => ok_json(view),
        Err(e) => settings_error(e),
    }
}

async fn put_websearch(
    State(state): State<ServeState>,
    Json(body): Json<WebSearchBody>,
) -> Response {
    let mut current = match state.settings_writer.load_settings() {
        Ok(s) => s.websearch,
        Err(e) => return settings_error(e),
    };
    current.api_key = crate::config::settings_writer::SettingsWriter::apply_api_key_patch(
        current.api_key,
        body.api_key.as_deref(),
    );
    match state.settings_writer.write_websearch(current) {
        Ok(ack) => {
            reload_runtime_after_settings_write(&state, &ack, "websearch write");
            ok_json(revision_body(ack))
        }
        Err(e) => settings_write_error(e),
    }
}

async fn get_models(State(state): State<ServeState>) -> Response {
    match state.settings_writer.load_settings() {
        Ok(settings) => ok_json(serde_json::json!({ "models": settings.models })),
        Err(e) => settings_error(e),
    }
}

async fn put_models(State(state): State<ServeState>, Json(body): Json<ModelsBody>) -> Response {
    let valid_ids: std::collections::HashSet<String> = body.models.keys().cloned().collect();
    match state.settings_writer.write_models(body.models) {
        Ok(ack) => {
            reload_runtime_after_settings_write(&state, &ack, "model write");
            if let Err(e) = state.sessions.clear_orphaned_model_ids(&valid_ids) {
                tracing::warn!(error = %e, "failed to clear orphaned session model_id bindings");
            }
            ok_json(revision_body(ack))
        }
        Err(e) => settings_write_error(e),
    }
}

async fn list_agents(State(state): State<ServeState>) -> Response {
    match state.settings_writer.load_settings() {
        Ok(settings) => {
            let mut agents: Vec<AgentListItem> = settings
                .agents
                .iter()
                .map(|(id, profile)| AgentListItem {
                    id: id.clone(),
                    role: profile.role,
                    description: profile.description.clone(),
                    allowed_subagents: profile.allowed_subagents.clone(),
                })
                .collect();
            agents.sort_by(|a, b| a.id.cmp(&b.id));
            ok_json(AgentsListBody { agents })
        }
        Err(e) => settings_error(e),
    }
}

async fn get_agent(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    match state.settings_writer.load_settings() {
        Ok(settings) => match settings.agents.get(&id) {
            Some(profile) => ok_json(profile),
            None => (
                StatusCode::NOT_FOUND,
                Json(ApiErr {
                    ok: false,
                    error: format!("agent not found: {id}"),
                }),
            )
                .into_response(),
        },
        Err(e) => settings_error(e),
    }
}

async fn put_agent(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Json(body): Json<AgentBody>,
) -> Response {
    let workspace = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace
        .clone();
    match state
        .settings_writer
        .write_agent(&id, body.profile, &workspace)
    {
        Ok(ack) => {
            reload_runtime_after_settings_write(&state, &ack, "agent write");
            ok_json(revision_body(ack))
        }
        Err(e) => settings_write_error(e),
    }
}

#[derive(Deserialize)]
struct ApplyPresetBody {
    preset: ToolPreset,
}

async fn apply_agent_tool_preset(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Json(body): Json<ApplyPresetBody>,
) -> Response {
    let workspace = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace
        .clone();
    match state
        .settings_writer
        .apply_agent_tool_preset(&id, body.preset, &workspace)
    {
        Ok(ack) => {
            reload_runtime_after_settings_write(&state, &ack, "apply tool preset");
            ok_json(revision_body(ack))
        }
        Err(e) => settings_write_error(e),
    }
}

async fn delete_agent(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    match state.settings_writer.delete_agent(&id) {
        Ok(ack) => {
            let mut runtime = state.runtime.write().expect("runtime lock");
            if runtime.desired_primary_agent() == id {
                let _ = runtime.set_desired_primary_agent("default".to_string());
            }
            drop(runtime);
            reload_runtime_after_settings_write(&state, &ack, "delete agent");
            ok_json(revision_body(ack))
        }
        Err(e) => settings_write_error(e),
    }
}

async fn get_available_tools(State(state): State<ServeState>) -> Response {
    let runtime = state.runtime.read().expect("runtime lock");
    let tools: Vec<AvailableTool> = available_tools(&runtime.resolved);
    ok_json(serde_json::json!({ "tools": tools }))
}

async fn list_custom_tools(State(state): State<ServeState>) -> Response {
    let root = workspace_root(&state);
    match (
        state.settings_writer.list_custom_tools(),
        state.settings_writer.list_workspace_custom_tools(&root),
    ) {
        (Ok(global), Ok(workspace)) => ok_json(serde_json::json!({
            "global": global,
            "workspace": workspace,
        })),
        (Err(e), _) | (_, Err(e)) => settings_error(e),
    }
}

async fn get_custom_tool(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> Response {
    if workspace_scope(&query) {
        let root = workspace_root(&state);
        return match state.settings_writer.get_workspace_custom_tool(&root, &id) {
            Ok(Some(tool)) => ok_json(tool),
            Ok(None) => (
                StatusCode::NOT_FOUND,
                Json(ApiErr {
                    ok: false,
                    error: format!("custom tool not found: {id}"),
                }),
            )
                .into_response(),
            Err(e) => settings_error(e),
        };
    }
    match state.settings_writer.get_custom_tool(&id) {
        Ok(Some(tool)) => ok_json(tool),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiErr {
                ok: false,
                error: format!("custom tool not found: {id}"),
            }),
        )
            .into_response(),
        Err(e) => settings_error(e),
    }
}

async fn put_custom_tool(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Query(query): Query<ScopeQuery>,
    Json(body): Json<CustomToolDefinition>,
) -> Response {
    let workspace = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace
        .clone();
    let result = if workspace_scope(&query) {
        state
            .settings_writer
            .write_workspace_custom_tool(&workspace.workspace_root, &id, body)
    } else {
        state.settings_writer.write_custom_tool(&id, body)
    };
    match result {
        Ok(ack) => {
            reload_runtime_after_settings_write(&state, &ack, "custom tool write");
            ok_json(revision_body(ack))
        }
        Err(e) => settings_write_error(e),
    }
}

async fn delete_custom_tool(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> Response {
    let workspace = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace
        .clone();
    let result = if workspace_scope(&query) {
        state
            .settings_writer
            .delete_workspace_custom_tool(&workspace.workspace_root, &id)
    } else {
        state.settings_writer.delete_custom_tool(&id, &workspace)
    };
    match result {
        Ok(ack) => {
            reload_runtime_after_settings_write(&state, &ack, "custom tool delete");
            ok_json(revision_body(ack))
        }
        Err(e) => settings_write_error(e),
    }
}

#[derive(Serialize)]
struct McpDefItem {
    id: String,
    origin: ToolOrigin,
    #[serde(flatten)]
    def: McpServerDefinition,
}

#[derive(Serialize)]
struct McpRuntimeView {
    status: McpRunState,
    tools: Vec<McpToolSchema>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct McpServerItem {
    id: String,
    origin: ToolOrigin,
    #[serde(flatten)]
    def: McpServerDefinition,
    status: McpRunState,
    tools: Vec<McpToolSchema>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn mcp_pool(state: &ServeState) -> std::sync::Arc<McpConnectionPool> {
    state.runtime.read().expect("runtime lock").mcp_pool.clone()
}

fn mcp_pool_key(workspace: bool, id: &str) -> String {
    if workspace {
        format!("workspace:{id}")
    } else {
        format!("global:{id}")
    }
}

fn mcp_item(
    id: String,
    origin: ToolOrigin,
    def: McpServerDefinition,
    snap: McpServerSnapshot,
) -> McpServerItem {
    McpServerItem {
        id,
        origin,
        def,
        status: snap.status,
        tools: snap.tools,
        error: snap.error,
    }
}

#[derive(Serialize)]
struct McpProbeResult {
    ready: bool,
    status: McpRunState,
    tools: Vec<McpToolSchema>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn mcp_cwd(state: &ServeState) -> std::path::PathBuf {
    state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace_root()
        .to_path_buf()
}

fn workspace_root(state: &ServeState) -> std::path::PathBuf {
    state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace_root()
        .to_path_buf()
}

fn mcp_runtime_view(snap: McpServerSnapshot) -> McpRuntimeView {
    McpRuntimeView {
        status: snap.status,
        tools: snap.tools,
        error: snap.error,
    }
}

async fn list_mcp_servers(State(state): State<ServeState>) -> Response {
    let root = workspace_root(&state);
    let global = match state.settings_writer.list_mcp_servers() {
        Ok(v) => v,
        Err(e) => return settings_error(e),
    };
    let workspace_defs = match state.settings_writer.list_workspace_mcp_servers(&root) {
        Ok(v) => v,
        Err(e) => return settings_error(e),
    };
    let pool = mcp_pool(&state);
    let snaps = pool.snapshots().await;
    let mut global_runtime = HashMap::new();
    let global_items: Vec<_> = global
        .into_iter()
        .map(|(id, def)| {
            let snap = snaps
                .get(&mcp_pool_key(false, &id))
                .cloned()
                .unwrap_or_default();
            global_runtime.insert(id.clone(), mcp_runtime_view(snap));
            McpDefItem {
                id,
                origin: ToolOrigin::Global,
                def,
            }
        })
        .collect();
    let mut workspace_runtime = HashMap::new();
    let workspace: Vec<_> = workspace_defs
        .into_iter()
        .map(|(id, def)| {
            let snap = snaps
                .get(&mcp_pool_key(true, &id))
                .cloned()
                .unwrap_or_default();
            workspace_runtime.insert(id.clone(), mcp_runtime_view(snap));
            McpDefItem {
                id,
                origin: ToolOrigin::Workspace,
                def,
            }
        })
        .collect();
    ok_json(serde_json::json!({
        "global": global_items,
        "workspace": workspace,
        "runtime": {
            "global": global_runtime,
            "workspace": workspace_runtime,
        },
    }))
}

async fn get_mcp_server(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> Response {
    let workspace = workspace_scope(&query);
    let key = mcp_pool_key(workspace, &id);
    if workspace {
        let root = workspace_root(&state);
        let def = match state.settings_writer.get_workspace_mcp_server(&root, &id) {
            Ok(v) => v,
            Err(e) => return settings_error(e),
        };
        return match def {
            Some(def) => {
                let snap = mcp_pool(&state).snapshot(&key).await;
                ok_json(mcp_item(id, ToolOrigin::Workspace, def, snap))
            }
            None => (
                StatusCode::NOT_FOUND,
                Json(ApiErr {
                    ok: false,
                    error: format!("MCP server not found: {id}"),
                }),
            )
                .into_response(),
        };
    }
    match state.settings_writer.get_mcp_server(&id) {
        Ok(Some(def)) => {
            let snap = mcp_pool(&state).snapshot(&key).await;
            ok_json(mcp_item(id, ToolOrigin::Global, def, snap))
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiErr {
                ok: false,
                error: format!("MCP server not found: {id}"),
            }),
        )
            .into_response(),
        Err(e) => settings_error(e),
    }
}

async fn put_mcp_server(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Query(query): Query<ScopeQuery>,
    Json(body): Json<McpServerDefinition>,
) -> Response {
    let workspace = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace
        .clone();
    let result = if workspace_scope(&query) {
        state
            .settings_writer
            .write_workspace_mcp_server(&workspace.workspace_root, &id, body)
    } else {
        state.settings_writer.write_mcp_server(&id, body)
    };
    match result {
        Ok(ack) => {
            reload_runtime_after_settings_write(&state, &ack, "MCP server write");
            ok_json(revision_body(ack))
        }
        Err(e) => settings_write_error(e),
    }
}

async fn delete_mcp_server(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> Response {
    let workspace = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace
        .clone();
    let ws_scope = workspace_scope(&query);
    mcp_pool(&state).stop(&mcp_pool_key(ws_scope, &id)).await;
    let result = if ws_scope {
        state
            .settings_writer
            .delete_workspace_mcp_server(&workspace.workspace_root, &id)
    } else {
        state.settings_writer.delete_mcp_server(&id, &workspace)
    };
    match result {
        Ok(ack) => {
            reload_runtime_after_settings_write(&state, &ack, "MCP server delete");
            ok_json(revision_body(ack))
        }
        Err(e) => settings_write_error(e),
    }
}

async fn load_mcp_definition(
    state: &ServeState,
    id: &str,
    workspace: bool,
) -> Result<McpServerDefinition, Response> {
    if workspace {
        let root = workspace_root(state);
        match state.settings_writer.get_workspace_mcp_server(&root, id) {
            Ok(Some(def)) => Ok(def),
            Ok(None) => Err((
                StatusCode::NOT_FOUND,
                Json(ApiErr {
                    ok: false,
                    error: format!("MCP server not found: {id}"),
                }),
            )
                .into_response()),
            Err(e) => Err(settings_error(e)),
        }
    } else {
        match state.settings_writer.get_mcp_server(id) {
            Ok(Some(def)) => Ok(def),
            Ok(None) => Err((
                StatusCode::NOT_FOUND,
                Json(ApiErr {
                    ok: false,
                    error: format!("MCP server not found: {id}"),
                }),
            )
                .into_response()),
            Err(e) => Err(settings_error(e)),
        }
    }
}

async fn start_mcp_server(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> Response {
    let workspace = workspace_scope(&query);
    let def = match load_mcp_definition(&state, &id, workspace).await {
        Ok(def) => def,
        Err(resp) => return resp,
    };
    let key = mcp_pool_key(workspace, &id);
    let cwd = Some(mcp_cwd(&state));
    mcp_lifecycle_result(mcp_pool(&state).start(&key, &def, cwd).await, key, &state).await
}

async fn restart_mcp_server(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> Response {
    let workspace = workspace_scope(&query);
    let def = match load_mcp_definition(&state, &id, workspace).await {
        Ok(def) => def,
        Err(resp) => return resp,
    };
    let key = mcp_pool_key(workspace, &id);
    let cwd = Some(mcp_cwd(&state));
    mcp_lifecycle_result(
        mcp_pool(&state).restart(&key, &def, cwd).await,
        key,
        &state,
    )
    .await
}

async fn stop_mcp_server(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> Response {
    let key = mcp_pool_key(workspace_scope(&query), &id);
    mcp_pool(&state).stop(&key).await;
    let snap = mcp_pool(&state).snapshot(&key).await;
    ok_json(McpProbeResult {
        ready: false,
        status: snap.status,
        tools: snap.tools,
        error: snap.error,
    })
}

async fn mcp_lifecycle_result(
    result: crate::types::Result<Vec<McpToolSchema>>,
    id: String,
    state: &ServeState,
) -> Response {
    let snap = mcp_pool(state).snapshot(&id).await;
    match result {
        Ok(schemas) => ok_json(McpProbeResult {
            ready: true,
            status: snap.status,
            tools: schemas,
            error: None,
        }),
        Err(e) => ok_json(McpProbeResult {
            ready: false,
            status: snap.status,
            tools: vec![],
            error: Some(e.to_string()),
        }),
    }
}

fn revision_body(ack: CommitAck) -> RevisionBody {
    RevisionBody {
        revision: ack.generation,
        docs: ack.docs,
    }
}

fn reload_runtime_after_settings_write(state: &ServeState, ack: &CommitAck, what: &str) {
    let mut runtime = state.runtime.write().expect("runtime lock");
    if let Err(e) = runtime.apply(&ack.docs) {
        tracing::warn!(error = %e, "{what}: runtime apply failed");
    }
    runtime.sync_workspace_tool_readiness();
}

async fn get_excludes(State(state): State<ServeState>) -> Response {
    let root = workspace_root(&state);
    match state.settings_writer.get_excludes(&root) {
        Ok(file) => ok_json(WorkspaceExcludesView::from_file(&file)),
        Err(e) => settings_error(e),
    }
}

async fn put_excludes(
    State(state): State<ServeState>,
    Json(body): Json<WorkspaceExcludesLists>,
) -> Response {
    let root = workspace_root(&state);
    let file = WorkspaceExcludesFile {
        version: 1,
        files_exclude: body.files_exclude,
        search_exclude: body.search_exclude,
        watcher_exclude: body.watcher_exclude,
        git_ignore: body.git_ignore,
        explorer_git_ignore: body.explorer_git_ignore,
    };
    match state.settings_writer.write_excludes(&root, file) {
        Ok(ack) => {
            reload_runtime_after_settings_write(&state, &ack, "excludes write");
            state.workspace_engines.request_index_sync(&root);
            match state.settings_writer.get_excludes(&root) {
                Ok(saved) => ok_json(WorkspaceExcludesView::from_file(&saved)),
                Err(e) => settings_error(e),
            }
        }
        Err(e) => settings_write_error(e),
    }
}

async fn get_engines(State(state): State<ServeState>) -> Response {
    let root = workspace_root(&state);
    match state.settings_writer.get_engines(&root) {
        Ok(file) => ok_json(file),
        Err(e) => settings_error(e),
    }
}

async fn put_engines(
    State(state): State<ServeState>,
    Json(body): Json<WorkspaceEnginesFile>,
) -> Response {
    let root = workspace_root(&state);
    match state.settings_writer.write_engines(&root, body) {
        Ok(ack) => {
            reload_runtime_after_settings_write(&state, &ack, "engines write");
            match state.settings_writer.get_engines(&root) {
                Ok(file) => ok_json(serde_json::json!({
                    "revision": ack.generation,
                    "docs": ack.docs,
                    "engines": file,
                })),
                Err(e) => settings_error(e),
            }
        }
        Err(e) => settings_write_error(e),
    }
}

async fn get_log(State(state): State<ServeState>) -> Response {
    match state.settings_writer.load_settings() {
        Ok(settings) => ok_json(settings.log),
        Err(e) => settings_error(e),
    }
}

async fn put_log(State(state): State<ServeState>, Json(body): Json<LogBody>) -> Response {
    match state
        .settings_writer
        .write_log(LogSettings { level: body.level })
    {
        Ok(ack) => {
            reload_runtime_after_settings_write(&state, &ack, "log write");
            ok_json(revision_body(ack))
        }
        Err(e) => settings_write_error(e),
    }
}

fn ok_json<T: Serialize>(data: T) -> Response {
    Json(ApiOk { ok: true, data }).into_response()
}

fn turn_blocked() -> Response {
    (
        StatusCode::CONFLICT,
        Json(ApiErr {
            ok: false,
            error: "turn_in_progress".into(),
        }),
    )
        .into_response()
}

fn settings_write_error(err: LitecodeError) -> Response {
    if matches!(&err, LitecodeError::Config(msg) if msg == "turn_in_progress") {
        return turn_blocked();
    }
    if matches!(&err, LitecodeError::Config(_)) {
        return validation_error(err);
    }
    settings_error(err)
}

fn validation_error(err: LitecodeError) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiErr {
            ok: false,
            error: err.to_string(),
        }),
    )
        .into_response()
}

fn settings_error(err: LitecodeError) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErr {
            ok: false,
            error: err.to_string(),
        }),
    )
        .into_response()
}
