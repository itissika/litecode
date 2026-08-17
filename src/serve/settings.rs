//! Settings REST API (`/api/settings/*`).

use std::collections::HashMap;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use crate::config::schema::{
    ADAPTER_OPENCODE, AgentProfile, CustomToolDefinition, InitScope, LogSettings,
    McpServerDefinition, ModelDefinition, ProviderAuth, ProviderDefinition, ToolCatalogEntry,
    ToolPreset, ToolReadiness, ToolTier,
};
use crate::config::workspace;
use crate::llm::{list_adapters, opencode_models_url, parse_opencode_model_catalog};
use crate::mcp::{McpConnectionPool, McpRunState, McpServerSnapshot};
use crate::serve::state::ServeState;
use crate::tool::catalog::{effective_readiness, prune_non_catalog_agent_bindings};
use crate::types::LitecodeError;

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
}

#[derive(Serialize)]
struct ProviderWriteResponse {
    revision: u64,
    restart_required: bool,
}

#[derive(Deserialize)]
struct ProvidersBody {
    providers: HashMap<String, ProviderDefinition>,
}

#[derive(Deserialize)]
struct WebSearchBody {
    search_endpoint: Option<String>,
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

#[derive(Deserialize)]
struct CatalogBody {
    tool_catalog: HashMap<String, ToolCatalogEntry>,
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
        .route("/tool-catalog", get(get_catalog).put(put_catalog))
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
        Ok(revision) => {
            reload_runtime_after_settings_write(&state, "provider write");
            ok_json(ProviderWriteResponse {
                revision,
                restart_required: false,
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
    if provider.adapter_id != ADAPTER_OPENCODE {
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
    let url = opencode_models_url(&endpoint);
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
    match parse_opencode_model_catalog(&body) {
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
    if let Some(endpoint) = body.search_endpoint {
        let trimmed = endpoint.trim();
        current.search_endpoint = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    match state.settings_writer.write_websearch(current) {
        Ok(revision) => ok_json(RevisionBody { revision }),
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
        Ok(revision) => {
            reload_runtime_after_settings_write(&state, "model write");
            if let Err(e) = state.sessions.clear_orphaned_model_ids(&valid_ids) {
                tracing::warn!(error = %e, "failed to clear orphaned session model_id bindings");
            }
            ok_json(RevisionBody { revision })
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
            Some(profile) => {
                let mut profile = profile.clone();
                let runtime = state.runtime.read().expect("runtime lock");
                let workspace_readiness =
                    workspace::workspace_readiness_from_engines(runtime.workspace_root());
                let runtime_catalog_state = runtime.resolved.runtime_catalog_state().clone();
                prune_non_catalog_agent_bindings(
                    &settings.tool_catalog,
                    &workspace_readiness,
                    &runtime_catalog_state,
                    &mut profile.tools,
                );
                ok_json(profile)
            }
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
        Ok(revision) => {
            reload_runtime_after_settings_write(&state, "agent write");
            ok_json(RevisionBody { revision })
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
        Ok(revision) => {
            reload_runtime_after_settings_write(&state, "apply tool preset");
            ok_json(RevisionBody { revision })
        }
        Err(e) => settings_write_error(e),
    }
}

async fn delete_agent(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    match state.settings_writer.delete_agent(&id) {
        Ok(revision) => {
            let mut runtime = state.runtime.write().expect("runtime lock");
            if runtime.desired_primary_agent() == id {
                let _ = runtime.set_desired_primary_agent("default".to_string());
            }
            drop(runtime);
            // 2.12: reload so the runtime config takes effect immediately.
            reload_runtime_after_settings_write(&state, "delete agent");
            ok_json(RevisionBody { revision })
        }
        Err(e) => settings_write_error(e),
    }
}

/// Catalog entry view: persisted catalog fields plus the *effective* readiness,
/// which is computed at request time from process-memory (global) and workspace
/// readiness state. Readiness is not persisted (see CONFIG §2.4), so it is injected here.
#[derive(Serialize)]
struct CatalogEntryView {
    id: String,
    tier: ToolTier,
    init_scope: InitScope,
    catalog_enabled: bool,
    readiness: ToolReadiness,
}

async fn get_catalog(State(state): State<ServeState>) -> Response {
    match state.settings_writer.load_settings() {
        Ok(settings) => {
            let runtime = state.runtime.read().expect("runtime lock");
            let workspace_readiness =
                workspace::workspace_readiness_from_engines(runtime.workspace_root());
            let runtime_catalog_state = runtime.resolved.runtime_catalog_state().clone();
            let tool_catalog: HashMap<String, CatalogEntryView> = settings
                .tool_catalog
                .into_iter()
                .map(|(id, entry)| {
                    let readiness =
                        effective_readiness(&entry, &workspace_readiness, &runtime_catalog_state);
                    let view = CatalogEntryView {
                        id: entry.id,
                        tier: entry.tier,
                        init_scope: entry.init_scope,
                        catalog_enabled: entry.catalog_enabled,
                        readiness,
                    };
                    (id, view)
                })
                .collect();
            ok_json(serde_json::json!({
                "tool_catalog": tool_catalog,
                "workspace_readiness": workspace_readiness,
                "engines": state
                    .workspace_engines
                    .workspace_engine_statuses(runtime.workspace_root()),
            }))
        }
        Err(e) => settings_error(e),
    }
}

async fn put_catalog(State(state): State<ServeState>, Json(body): Json<CatalogBody>) -> Response {
    let workspace = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace
        .clone();
    match state
        .settings_writer
        .write_tool_catalog(body.tool_catalog, &workspace)
    {
        Ok(revision) => {
            // 2.12: reload so the runtime config takes effect immediately.
            reload_runtime_after_settings_write(&state, "catalog write");
            ok_json(RevisionBody { revision })
        }
        Err(e) => settings_write_error(e),
    }
}

async fn list_custom_tools(State(state): State<ServeState>) -> Response {
    match state.settings_writer.list_custom_tools() {
        Ok(custom_tools) => ok_json(serde_json::json!({ "custom_tools": custom_tools })),
        Err(e) => settings_error(e),
    }
}

async fn get_custom_tool(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
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
    Json(body): Json<CustomToolDefinition>,
) -> Response {
    match state.settings_writer.write_custom_tool(&id, body) {
        Ok(revision) => {
            reload_runtime_after_settings_write(&state, "custom tool write");
            ok_json(RevisionBody { revision })
        }
        Err(e) => settings_write_error(e),
    }
}

async fn delete_custom_tool(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    match state.settings_writer.delete_custom_tool(&id) {
        Ok(revision) => {
            reload_runtime_after_settings_write(&state, "custom tool delete");
            ok_json(RevisionBody { revision })
        }
        Err(e) => settings_write_error(e),
    }
}

#[derive(Serialize)]
struct McpServerItem {
    id: String,
    #[serde(flatten)]
    def: McpServerDefinition,
    status: McpRunState,
    tools: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn mcp_pool(state: &ServeState) -> std::sync::Arc<McpConnectionPool> {
    state.runtime.read().expect("runtime lock").mcp_pool.clone()
}

fn mcp_item(id: String, def: McpServerDefinition, snap: McpServerSnapshot) -> McpServerItem {
    McpServerItem {
        id,
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
    tools: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

async fn list_mcp_servers(State(state): State<ServeState>) -> Response {
    match state.settings_writer.list_mcp_servers() {
        Ok(servers) => {
            let pool = mcp_pool(&state);
            let snaps = pool.snapshots().await;
            ok_json(serde_json::json!({
                "mcp_servers": servers
                    .into_iter()
                    .map(|(id, def)| {
                        let snap = snaps.get(&id).cloned().unwrap_or_default();
                        mcp_item(id, def, snap)
                    })
                    .collect::<Vec<_>>(),
            }))
        }
        Err(e) => settings_error(e),
    }
}

async fn get_mcp_server(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    match state.settings_writer.get_mcp_server(&id) {
        Ok(Some(def)) => {
            let snap = mcp_pool(&state).snapshot(&id).await;
            ok_json(mcp_item(id, def, snap))
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
    Json(body): Json<McpServerDefinition>,
) -> Response {
    match state.settings_writer.write_mcp_server(&id, body) {
        Ok(revision) => {
            reload_runtime_after_settings_write(&state, "MCP server write");
            ok_json(RevisionBody { revision })
        }
        Err(e) => settings_write_error(e),
    }
}

async fn delete_mcp_server(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    mcp_pool(&state).stop(&id).await;
    match state.settings_writer.delete_mcp_server(&id) {
        Ok(revision) => {
            reload_runtime_after_settings_write(&state, "MCP server delete");
            ok_json(RevisionBody { revision })
        }
        Err(e) => settings_write_error(e),
    }
}

async fn start_mcp_server(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Json(def): Json<McpServerDefinition>,
) -> Response {
    mcp_lifecycle_result(mcp_pool(&state).start(&id, &def).await, id, &state).await
}

async fn restart_mcp_server(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Json(def): Json<McpServerDefinition>,
) -> Response {
    mcp_lifecycle_result(mcp_pool(&state).restart(&id, &def).await, id, &state).await
}

async fn stop_mcp_server(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    mcp_pool(&state).stop(&id).await;
    let snap = mcp_pool(&state).snapshot(&id).await;
    ok_json(McpProbeResult {
        ready: false,
        status: snap.status,
        tools: snap.tools,
        error: snap.error,
    })
}

async fn mcp_lifecycle_result(
    result: crate::types::Result<Vec<(String, serde_json::Value)>>,
    id: String,
    state: &ServeState,
) -> Response {
    let snap = mcp_pool(state).snapshot(&id).await;
    match result {
        Ok(schemas) => ok_json(McpProbeResult {
            ready: true,
            status: snap.status,
            tools: schemas.into_iter().map(|(name, _)| name).collect(),
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

/// Custom-tool definition changes must land in live `resolved` immediately so
/// catalog views / next turn use the same snapshot as SQLite (no stale defs).
/// `reload_if_needed` already runs global catalog init — do not double-init.
fn reload_runtime_after_settings_write(state: &ServeState, what: &str) {
    let mut runtime = state.runtime.write().expect("runtime lock");
    if let Err(e) = runtime.reload_if_needed() {
        tracing::warn!(error = %e, "{what}: runtime reload failed");
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
        Ok(revision) => ok_json(RevisionBody { revision }),
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
