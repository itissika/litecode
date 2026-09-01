use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use super::WorkspaceError;
use super::git::{self, GitError};
use super::service::WorkspaceService;
use super::tree::TreeEntry;
use crate::config::workspace::{
    clear_lsp_servers, enable_code_search_engine, set_workspace_engine_desired, write_lsp_init,
};
use crate::lsp::deps::{LspInitFailure, ensure_servers, probe_workspace_servers};
use crate::serve::state::ServeState;

static INSTALL_TASKS: std::sync::LazyLock<Mutex<HashMap<String, InstallTask>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize)]
struct InstallTask {
    task_id: String,
    server_id: String,
    status: String,
    error: Option<String>,
    progress: Option<InstallProgress>,
}

#[derive(Debug, Clone, Serialize)]
struct InstallProgress {
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
}

#[derive(Clone)]
pub struct WorkspaceState {
    pub workspace: Arc<WorkspaceService>,
}

#[derive(Serialize)]
struct ApiOk<T: Serialize> {
    ok: bool,
    data: T,
}

#[derive(Serialize)]
struct ApiErr {
    ok: bool,
    error: String,
}

#[derive(Debug, Deserialize)]
pub struct TreeQuery {
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_depth")]
    pub depth: usize,
    /// `1` / `true` → ancestor listing (`by_dir`) instead of one directory's `entries`.
    #[serde(default)]
    pub reveal: String,
}

fn default_depth() -> usize {
    1
}

fn reveal_requested(flag: &str) -> bool {
    flag == "1" || flag.eq_ignore_ascii_case("true")
}

#[derive(Debug, Deserialize)]
pub struct PathQuery {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Deserialize)]
pub struct FileBody {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateFileBody {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct MkdirBody {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameBody {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Deserialize)]
pub struct CopyBody {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Serialize)]
struct RenameData {
    from: String,
    to: String,
}

#[derive(Serialize)]
struct TreeData {
    entries: Vec<TreeEntry>,
}

#[derive(Serialize)]
struct TreeRevealData {
    by_dir: std::collections::BTreeMap<String, Vec<TreeEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct GlobQuery {
    #[serde(default)]
    pub pattern: String,
}

#[derive(Serialize)]
struct GlobData {
    entries: Vec<TreeEntry>,
    truncated: bool,
}

#[derive(Serialize)]
struct FileData {
    path: String,
    content: String,
}

#[derive(Serialize)]
struct PathData {
    path: String,
}

pub fn router() -> Router<ServeState> {
    Router::new()
        .route("/tree", get(get_tree))
        .route("/glob", get(get_glob))
        .route("/lsp/probe", get(get_lsp_probe))
        .route("/lsp/init", post(post_lsp_init))
        .route("/lsp/stop", post(post_lsp_stop))
        .route("/lsp/clear", post(post_lsp_clear))
        .route("/retrieval/init", post(post_retrieval_init))
        .route("/retrieval/stop", post(post_retrieval_stop))
        .route("/retrieval/refresh", post(post_retrieval_refresh))
        .route("/retrieval/search", post(post_retrieval_search))
        .route("/lsp/install", post(post_lsp_install))
        .route("/lsp/install/status", get(get_lsp_install_status))
        .route("/engines", get(get_engines))
        .route("/engines/detail", get(get_engines_detail))
        .route(
            "/file",
            get(get_file)
                .put(put_file)
                .post(post_file)
                .delete(delete_file),
        )
        .route("/mkdir", post(post_mkdir))
        .route("/rename", post(post_rename))
        .route("/copy", post(post_copy))
        .route("/blob", post(post_blob).put(put_blob))
        .route("/git/status", get(get_git_status))
        .route("/git/log", get(get_git_log))
        .route("/git/stage", post(post_git_stage))
        .route("/git/unstage", post(post_git_unstage))
        .route("/git/restore", post(post_git_restore))
        .route("/git/commit", post(post_git_commit))
        .route("/git/pull", post(post_git_pull))
        .route("/git/push", post(post_git_push))
}

#[derive(Debug, Deserialize)]
pub struct LspInitBody {
    pub servers: Vec<String>,
}

#[derive(Serialize)]
struct LspProbeData {
    servers: Vec<crate::lsp::deps::LspServerProbe>,
}

#[derive(Serialize)]
struct LspInitData {
    servers: Vec<String>,
}

#[derive(Serialize)]
struct EngineMutationData {
    desired: bool,
}

#[derive(Serialize)]
struct LspInitFailureBody {
    id: String,
    error: String,
}

#[derive(Serialize)]
struct EnginesData {
    engines: std::collections::HashMap<String, crate::engines::EngineStatus>,
    lsp_servers: Vec<crate::lsp::LspInstanceStatus>,
}

#[derive(Serialize)]
struct EnginesDetailData {
    retrieval: serde_json::Value,
    lsp: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct LspInstallBody {
    server_id: String,
}

#[derive(Serialize)]
struct LspInstallData {
    task_id: String,
    status: String,
    progress: Option<InstallProgress>,
}

#[derive(Debug, Deserialize)]
struct LspInstallStatusQuery {
    task_id: String,
}

async fn get_lsp_probe(State(state): State<ServeState>) -> Response {
    let root = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace_root()
        .to_path_buf();
    let servers = match tokio::task::spawn_blocking(move || probe_workspace_servers(&root)).await {
        Ok(servers) => servers,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErr {
                    ok: false,
                    error: format!("LSP probe task failed: {err}"),
                }),
            )
                .into_response();
        }
    };
    Json(ApiOk {
        ok: true,
        data: LspProbeData { servers },
    })
    .into_response()
}

async fn get_engines(State(state): State<ServeState>) -> Response {
    let root = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace_root()
        .to_path_buf();
    Json(ApiOk {
        ok: true,
        data: EnginesData {
            engines: state.ide.engines.workspace_engine_statuses(&root),
            lsp_servers: state.ide.engines.lsp_hub().instance_statuses(),
        },
    })
    .into_response()
}

async fn get_engines_detail(State(state): State<ServeState>) -> Response {
    let root = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace_root()
        .to_path_buf();
    let engines = state.ide.engines.clone();
    let detail = match tokio::task::spawn_blocking(move || engines.engines_detail_view(&root)).await
    {
        Ok(detail) => detail,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErr {
                    ok: false,
                    error: format!("engine detail task failed: {err}"),
                }),
            )
                .into_response();
        }
    };
    Json(ApiOk {
        ok: true,
        data: EnginesDetailData {
            retrieval: detail
                .get("retrieval")
                .cloned()
                .unwrap_or(serde_json::json!({})),
            lsp: detail.get("lsp").cloned().unwrap_or(serde_json::json!({})),
        },
    })
    .into_response()
}

async fn post_lsp_init(State(state): State<ServeState>, Json(body): Json<LspInitBody>) -> Response {
    if state.turn_guard.is_turn_in_progress() {
        return (
            StatusCode::CONFLICT,
            Json(ApiErr {
                ok: false,
                error: "turn_in_progress".into(),
            }),
        )
            .into_response();
    }

    let root = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace_root()
        .to_path_buf();

    if body.servers.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiErr {
                ok: false,
                error: "select at least one language server".into(),
            }),
        )
            .into_response();
    }

    let (ready_ids, failures) = ensure_servers(&body.servers, true).await;
    if !failures.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "ok": false,
                "error": "language server setup failed",
                "failed": failures.into_iter().map(|LspInitFailure { id, error }| LspInitFailureBody { id, error }).collect::<Vec<_>>(),
            })),
        )
            .into_response();
    }

    if let Err(e) = write_lsp_init(&root, ready_ids.clone()) {
        return open_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }

    {
        let mut runtime = state.runtime.write().expect("runtime lock");
        runtime.sync_workspace_tool_readiness();
        state.workspace_engines.reconcile(&runtime.resolved);
    }

    Json(ApiOk {
        ok: true,
        data: LspInitData { servers: ready_ids },
    })
    .into_response()
}

async fn post_lsp_stop(State(state): State<ServeState>) -> Response {
    if state.turn_guard.is_turn_in_progress() {
        return (
            StatusCode::CONFLICT,
            Json(ApiErr {
                ok: false,
                error: "turn_in_progress".into(),
            }),
        )
            .into_response();
    }

    let root = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace_root()
        .to_path_buf();
    if let Err(error) = set_workspace_engine_desired(&root, "lsp", false) {
        return open_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
    }

    let mut runtime = state.runtime.write().expect("runtime lock");
    runtime.sync_workspace_tool_readiness();
    state.workspace_engines.reconcile(&runtime.resolved);
    Json(ApiOk {
        ok: true,
        data: EngineMutationData { desired: false },
    })
    .into_response()
}

/// Clear enabled language-server selection and stop the engine.
/// Used when the UI turns Off the last LSP card (unlike `/stop`, which keeps servers).
async fn post_lsp_clear(State(state): State<ServeState>) -> Response {
    if state.turn_guard.is_turn_in_progress() {
        return (
            StatusCode::CONFLICT,
            Json(ApiErr {
                ok: false,
                error: "turn_in_progress".into(),
            }),
        )
            .into_response();
    }

    let root = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace_root()
        .to_path_buf();
    if let Err(error) = clear_lsp_servers(&root) {
        return open_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
    }

    let mut runtime = state.runtime.write().expect("runtime lock");
    runtime.sync_workspace_tool_readiness();
    state.workspace_engines.reconcile(&runtime.resolved);
    Json(ApiOk {
        ok: true,
        data: EngineMutationData { desired: false },
    })
    .into_response()
}

async fn post_retrieval_init(State(state): State<ServeState>) -> Response {
    if state.turn_guard.is_turn_in_progress() {
        return (
            StatusCode::CONFLICT,
            Json(ApiErr {
                ok: false,
                error: "turn_in_progress".into(),
            }),
        )
            .into_response();
    }

    let root = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace_root()
        .to_path_buf();
    if let Err(error) = enable_code_search_engine(&root) {
        return open_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
    }

    let mut runtime = state.runtime.write().expect("runtime lock");
    runtime.sync_workspace_tool_readiness();
    state.workspace_engines.reconcile(&runtime.resolved);
    Json(ApiOk {
        ok: true,
        data: EngineMutationData { desired: true },
    })
    .into_response()
}

async fn post_retrieval_stop(State(state): State<ServeState>) -> Response {
    if state.turn_guard.is_turn_in_progress() {
        return (
            StatusCode::CONFLICT,
            Json(ApiErr {
                ok: false,
                error: "turn_in_progress".into(),
            }),
        )
            .into_response();
    }

    let root = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace_root()
        .to_path_buf();
    if let Err(error) = set_workspace_engine_desired(&root, "code_search", false) {
        return open_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
    }

    let mut runtime = state.runtime.write().expect("runtime lock");
    runtime.sync_workspace_tool_readiness();
    state.workspace_engines.reconcile(&runtime.resolved);
    Json(ApiOk {
        ok: true,
        data: EngineMutationData { desired: false },
    })
    .into_response()
}

#[derive(Serialize)]
struct RefreshData {
    desired: bool,
    mode: crate::engines::RefreshAcceptedMode,
}

async fn post_retrieval_refresh(State(state): State<ServeState>) -> Response {
    if state.turn_guard.is_turn_in_progress() {
        return (
            StatusCode::CONFLICT,
            Json(ApiErr {
                ok: false,
                error: "turn_in_progress".into(),
            }),
        )
            .into_response();
    }

    let accepted = {
        let mut runtime = state.runtime.write().expect("runtime lock");
        match state.workspace_engines.request_refresh(&runtime.resolved) {
            Ok(accepted) => {
                runtime.sync_workspace_tool_readiness();
                accepted
            }
            Err(error) => {
                return open_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
            }
        }
    };
    Json(ApiOk {
        ok: true,
        data: RefreshData {
            desired: accepted.desired,
            mode: accepted.mode,
        },
    })
    .into_response()
}

async fn post_retrieval_search(
    State(state): State<ServeState>,
    Json(body): Json<crate::engines::code_search::HumanSearchRequest>,
) -> Response {
    let root = state
        .runtime
        .read()
        .expect("runtime lock")
        .workspace_root()
        .to_path_buf();
    if body.query.trim().is_empty() {
        return open_error(StatusCode::BAD_REQUEST, "query is required".into());
    }
    match state.workspace_engines.human_search(&root, &body) {
        Ok(data) => Json(ApiOk { ok: true, data }).into_response(),
        Err(e) => open_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn post_lsp_install(
    State(state): State<ServeState>,
    Json(body): Json<LspInstallBody>,
) -> Response {
    if state.turn_guard.is_turn_in_progress() {
        return (
            StatusCode::CONFLICT,
            Json(ApiErr {
                ok: false,
                error: "turn_in_progress".into(),
            }),
        )
            .into_response();
    }

    let task_id = uuid::Uuid::new_v4().to_string();
    let server_id = body.server_id.clone();

    {
        let mut tasks = INSTALL_TASKS.lock().unwrap();
        if let Some(task) = tasks
            .values()
            .find(|task| task.server_id == server_id && task.status == "installing")
        {
            return Json(ApiOk {
                ok: true,
                data: LspInstallData {
                    task_id: task.task_id.clone(),
                    status: task.status.clone(),
                    progress: task.progress.clone(),
                },
            })
            .into_response();
        }
        tasks.insert(
            task_id.clone(),
            InstallTask {
                task_id: task_id.clone(),
                server_id: server_id.clone(),
                status: "installing".to_string(),
                error: None,
                progress: None,
            },
        );
    }

    let tid = task_id.clone();
    let tid_for_progress = tid.clone();
    let sid = server_id.clone();
    tokio::spawn(async move {
        let progress = std::sync::Arc::new(move |downloaded_bytes, total_bytes| {
            if let Ok(mut tasks) = INSTALL_TASKS.lock()
                && let Some(task) = tasks.get_mut(&tid_for_progress)
            {
                task.progress = Some(InstallProgress {
                    downloaded_bytes,
                    total_bytes,
                });
            }
        });
        let result = crate::lsp::install::install_server_to_lsp_dir(&sid, Some(progress)).await;
        let mut tasks = INSTALL_TASKS.lock().unwrap();
        if let Some(task) = tasks.get_mut(&tid) {
            match result {
                Ok(()) => task.status = "done".to_string(),
                Err(e) => {
                    task.status = "failed".to_string();
                    task.error = Some(e.to_string());
                }
            }
        }
    });

    Json(ApiOk {
        ok: true,
        data: LspInstallData {
            task_id,
            status: "installing".to_string(),
            progress: None,
        },
    })
    .into_response()
}

async fn get_lsp_install_status(Query(query): Query<LspInstallStatusQuery>) -> Response {
    let tasks = INSTALL_TASKS.lock().unwrap();
    match tasks.get(&query.task_id) {
        Some(task) => Json(serde_json::json!({
            "ok": true,
            "data": {
                "task_id": task.task_id,
                "server_id": task.server_id,
                "status": task.status,
                "error": task.error,
                "progress": task.progress,
            }
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiErr {
                ok: false,
                error: format!("task not found: {}", query.task_id),
            }),
        )
            .into_response(),
    }
}

fn open_error(status: StatusCode, msg: String) -> Response {
    (
        status,
        Json(ApiErr {
            ok: false,
            error: msg,
        }),
    )
        .into_response()
}

async fn get_tree(State(state): State<ServeState>, Query(query): Query<TreeQuery>) -> Response {
    let workspace = state.workspace.clone();
    let path = query.path;
    let depth = query.depth;
    let reveal = reveal_requested(&query.reveal);
    match tokio::task::spawn_blocking(move || {
        if reveal {
            workspace
                .tree_reveal(&path)
                .map(std::ops::ControlFlow::Break)
        } else {
            workspace
                .tree(&path, depth)
                .map(std::ops::ControlFlow::Continue)
        }
    })
    .await
    {
        Ok(Ok(std::ops::ControlFlow::Continue(entries))) => Json(ApiOk {
            ok: true,
            data: TreeData { entries },
        })
        .into_response(),
        Ok(Ok(std::ops::ControlFlow::Break(layers))) => {
            let by_dir = layers.into_iter().collect();
            Json(ApiOk {
                ok: true,
                data: TreeRevealData { by_dir },
            })
            .into_response()
        }
        Ok(Err(e)) => workspace_error(e),
        Err(e) => open_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("tree task join: {e}"),
        ),
    }
}

async fn get_glob(State(state): State<ServeState>, Query(query): Query<GlobQuery>) -> Response {
    let workspace = state.workspace.clone();
    let pattern = query.pattern;
    match tokio::task::spawn_blocking(move || workspace.glob(&pattern)).await {
        Ok(Ok(listing)) => Json(ApiOk {
            ok: true,
            data: GlobData {
                entries: listing.entries,
                truncated: listing.truncated,
            },
        })
        .into_response(),
        Ok(Err(e)) => workspace_error(e),
        Err(e) => open_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("glob task join: {e}"),
        ),
    }
}

async fn get_file(State(state): State<ServeState>, Query(query): Query<PathQuery>) -> Response {
    let workspace = state.workspace.clone();
    let path = query.path;
    match tokio::task::spawn_blocking(move || workspace.read_file(&path)).await {
        Ok(Ok((path, content))) => Json(ApiOk {
            ok: true,
            data: FileData { path, content },
        })
        .into_response(),
        Ok(Err(e)) => workspace_error(e),
        Err(e) => open_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("file task join: {e}"),
        ),
    }
}

async fn put_file(
    State(state): State<ServeState>,
    Query(query): Query<PathQuery>,
    Json(body): Json<FileBody>,
) -> Response {
    match state.workspace.write_file(&query.path, &body.content) {
        Ok(path) => {
            if let Ok(abs) = state.workspace.sandbox().resolve(&path) {
                state.ide.apply_document_if_ready(&abs, &body.content).await;
            }
            Json(ApiOk {
                ok: true,
                data: PathData { path },
            })
            .into_response()
        }
        Err(e) => workspace_error(e),
    }
}

async fn post_file(State(state): State<ServeState>, Json(body): Json<CreateFileBody>) -> Response {
    match state.workspace.create_file(&body.path, &body.content) {
        Ok(path) => {
            if let Ok(abs) = state.workspace.sandbox().resolve(&path) {
                state.ide.apply_document_if_ready(&abs, &body.content).await;
            }
            (
                StatusCode::CREATED,
                Json(ApiOk {
                    ok: true,
                    data: PathData { path },
                }),
            )
                .into_response()
        }
        Err(e) => workspace_error(e),
    }
}

async fn delete_file(State(state): State<ServeState>, Query(query): Query<PathQuery>) -> Response {
    let abs = state.workspace.sandbox().resolve(&query.path).ok();
    match state.workspace.delete_path(&query.path, query.recursive) {
        Ok(path) => {
            if let Some(abs) = abs {
                state.ide.sync_document_if_ready(&abs).await;
            }
            Json(ApiOk {
                ok: true,
                data: PathData { path },
            })
            .into_response()
        }
        Err(e) => workspace_error(e),
    }
}

async fn post_mkdir(State(state): State<ServeState>, Json(body): Json<MkdirBody>) -> Response {
    match state.workspace.mkdir(&body.path) {
        Ok(path) => (
            StatusCode::CREATED,
            Json(ApiOk {
                ok: true,
                data: PathData { path },
            }),
        )
            .into_response(),
        Err(e) => workspace_error(e),
    }
}

async fn post_rename(State(state): State<ServeState>, Json(body): Json<RenameBody>) -> Response {
    match state
        .workspace
        .rename_path(&body.from, &body.to, body.overwrite)
    {
        Ok((from, to)) => {
            if let Ok(abs) = state.workspace.sandbox().resolve(&from) {
                state.ide.sync_document_if_ready(&abs).await;
            }
            if let Ok(abs) = state.workspace.sandbox().resolve(&to) {
                state.ide.sync_document_if_ready(&abs).await;
            }
            Json(ApiOk {
                ok: true,
                data: RenameData { from, to },
            })
            .into_response()
        }
        Err(e) => workspace_error(e),
    }
}

async fn post_copy(State(state): State<ServeState>, Json(body): Json<CopyBody>) -> Response {
    match state
        .workspace
        .copy_path(&body.from, &body.to, body.overwrite)
    {
        Ok(path) => {
            if let Ok(abs) = state.workspace.sandbox().resolve(&path) {
                state.ide.sync_document_if_ready(&abs).await;
            }
            (
                StatusCode::CREATED,
                Json(ApiOk {
                    ok: true,
                    data: PathData { path },
                }),
            )
                .into_response()
        }
        Err(e) => workspace_error(e),
    }
}

async fn post_blob(
    State(state): State<ServeState>,
    Query(query): Query<PathQuery>,
    body: Bytes,
) -> Response {
    write_blob(state, &query.path, &body, false).await
}

async fn put_blob(
    State(state): State<ServeState>,
    Query(query): Query<PathQuery>,
    body: Bytes,
) -> Response {
    write_blob(state, &query.path, &body, true).await
}

#[derive(Debug, Deserialize)]
struct GitPathsBody {
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GitCommitBody {
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct GitLogQuery {
    pub limit: Option<usize>,
}

async fn git_blocking<T: Serialize + Send + 'static>(
    f: impl FnOnce() -> Result<T, GitError> + Send + 'static,
) -> Response {
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(data)) => Json(ApiOk { ok: true, data }).into_response(),
        Ok(Err(e)) => git_error_response(e),
        Err(e) => open_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("git task join: {e}"),
        ),
    }
}

fn git_error_response(err: GitError) -> Response {
    (
        git::git_error_status(&err),
        Json(ApiErr {
            ok: false,
            error: err.to_string(),
        }),
    )
        .into_response()
}

async fn get_git_status(State(state): State<ServeState>) -> Response {
    let root = state.workspace.sandbox().root().to_path_buf();
    git_blocking(move || git::status(&root)).await
}

async fn get_git_log(
    State(state): State<ServeState>,
    Query(query): Query<GitLogQuery>,
) -> Response {
    let root = state.workspace.sandbox().root().to_path_buf();
    git_blocking(move || git::log(&root, query.limit)).await
}

async fn post_git_stage(
    State(state): State<ServeState>,
    Json(body): Json<GitPathsBody>,
) -> Response {
    let sandbox = state.workspace.sandbox().clone();
    git_blocking(move || git::stage(&sandbox, &body.paths)).await
}

async fn post_git_unstage(
    State(state): State<ServeState>,
    Json(body): Json<GitPathsBody>,
) -> Response {
    let sandbox = state.workspace.sandbox().clone();
    git_blocking(move || git::unstage(&sandbox, &body.paths)).await
}

async fn post_git_restore(
    State(state): State<ServeState>,
    Json(body): Json<GitPathsBody>,
) -> Response {
    let sandbox = state.workspace.sandbox().clone();
    git_blocking(move || git::restore(&sandbox, &body.paths)).await
}

async fn post_git_commit(
    State(state): State<ServeState>,
    Json(body): Json<GitCommitBody>,
) -> Response {
    let sandbox = state.workspace.sandbox().clone();
    git_blocking(move || git::commit(&sandbox, &body.message)).await
}

async fn post_git_pull(State(state): State<ServeState>) -> Response {
    let root = state.workspace.sandbox().root().to_path_buf();
    git_blocking(move || git::pull(&root)).await
}

async fn post_git_push(State(state): State<ServeState>) -> Response {
    let root = state.workspace.sandbox().root().to_path_buf();
    git_blocking(move || git::push(&root)).await
}

async fn write_blob(state: ServeState, path: &str, body: &Bytes, overwrite: bool) -> Response {
    match state.workspace.write_file_bytes(path, body, overwrite) {
        Ok(path) => {
            if let Ok(abs) = state.workspace.sandbox().resolve(&path) {
                if let Ok(text) = std::str::from_utf8(body) {
                    state.ide.apply_document_if_ready(&abs, text).await;
                } else {
                    state.ide.sync_document_if_ready(&abs).await;
                }
            }
            let status = if overwrite {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (
                status,
                Json(ApiOk {
                    ok: true,
                    data: PathData { path },
                }),
            )
                .into_response()
        }
        Err(e) => workspace_error(e),
    }
}

fn workspace_error(err: WorkspaceError) -> Response {
    let (status, msg) = match &err {
        WorkspaceError::Sandbox(super::sandbox::SandboxError::Escape) => {
            (StatusCode::FORBIDDEN, err.to_string())
        }
        WorkspaceError::Sandbox(super::sandbox::SandboxError::Invalid(_)) => {
            (StatusCode::BAD_REQUEST, err.to_string())
        }
        WorkspaceError::NotFound(_)
        | WorkspaceError::Sandbox(super::sandbox::SandboxError::NotFound(_)) => {
            (StatusCode::NOT_FOUND, err.to_string())
        }
        WorkspaceError::AlreadyExists(_) => (StatusCode::CONFLICT, err.to_string()),
        WorkspaceError::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, err.to_string()),
        WorkspaceError::NotFile(_)
        | WorkspaceError::IsDir(_)
        | WorkspaceError::Tree(_)
        | WorkspaceError::InvalidMove(_) => (StatusCode::BAD_REQUEST, err.to_string()),
        WorkspaceError::NotUtf8 => (StatusCode::UNSUPPORTED_MEDIA_TYPE, err.to_string()),
        WorkspaceError::Sandbox(super::sandbox::SandboxError::Io(e)) | WorkspaceError::Io(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    };
    (
        status,
        Json(ApiErr {
            ok: false,
            error: msg,
        }),
    )
        .into_response()
}
