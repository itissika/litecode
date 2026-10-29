use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::get;
use serde_json::json;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use crate::client_protocol::ws;
use crate::serve::auth;
use crate::serve::settings;
use crate::serve::shutdown::{self, ShutdownWatch};
use crate::serve::state::ServeState;
use crate::session::manager::EMPTY_SESSION_TTL;
use crate::workspace::{restart_watcher, workspace_router};

/// How often the idle tick checks the session corpus for drift.
///
/// The tick is the **only** thing that refreshes the session semantic index: a
/// session corpus moves on every turn, so an on-demand refresh would put a
/// reconcile in front of the search that asked for it. The tick runs only while
/// no turn is in progress (the same gate a settings write uses), so 30s costs
/// one cheap watermark read per half-minute and bounds staleness to roughly the
/// tail of the previous turn.
const SESSION_INDEX_TICK: std::time::Duration = std::time::Duration::from_secs(30);
/// Silent incremental code-index pass (WP-079 `code_index_tick`). Sync and
/// non-blocking: the engine decides and spawns any refresh itself.
const CODE_INDEX_TICK: std::time::Duration = std::time::Duration::from_secs(30);

pub fn router(state: ServeState, web_dist: PathBuf) -> Router {
    let cors = if state.auth_token.is_some() {
        CorsLayer::new()
    } else {
        // G1: without a token the CORS policy is tightened to localhost-only
        // origins (the desktop/dev case); previously it was wide open.
        CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(
                |origin: &axum::http::HeaderValue, _parts: &axum::http::request::Parts| {
                    origin
                        .to_str()
                        .ok()
                        .map(crate::serve::auth::origin_host)
                        .map(|host| host == "localhost" || host == "127.0.0.1" || host == "::1")
                        .unwrap_or(false)
                },
            ))
            .allow_methods(Any)
            .allow_headers(Any)
    };

    let auth_layer = middleware::from_fn_with_state(state.clone(), auth::middleware);

    // G3: never record the full URI (which may carry the /ws token query) in
    // access logs — span carries method + path only.
    let trace_layer = TraceLayer::new_for_http().make_span_with(
        |request: &axum::http::Request<axum::body::Body>| {
            let path = request.uri().path().to_string();
            tracing::info_span!("http", method = %request.method(), path)
        },
    );

    Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws::ws_handler))
        .nest("/api/workspace", workspace_router())
        .nest("/api/workspace/apps", crate::apps::routes::router())
        .nest("/api/settings", settings::router())
        .nest("/api/media", crate::serve::media::router())
        .fallback_service(ServeDir::new(web_dist))
        .layer(auth_layer)
        .layer(cors)
        .layer(trace_layer)
        .with_state(state)
}

async fn health(State(state): State<ServeState>) -> impl IntoResponse {
    let (workspace_root, workspace_id) = {
        let runtime = state.runtime.read().expect("runtime lock");
        (
            runtime
                .workspace
                .workspace_root
                .to_string_lossy()
                .to_string(),
            runtime.workspace.workspace_id.clone(),
        )
    };
    Json(json!({
        "ok": true,
        "workspace_root": workspace_root,
        "workspace_id": workspace_id,
    }))
}

pub async fn listen(
    state: ServeState,
    addr: SocketAddr,
    web_dist: PathBuf,
    shutdown_watch: ShutdownWatch,
) -> anyhow::Result<()> {
    let engine_manager = state.engine_manager.clone();
    let workspace_engines = state.workspace_engines.clone();
    let sessions = state.sessions.clone();
    let mcp_pool = state.runtime.read().expect("runtime lock").mcp_pool.clone();
    let session_gc = sessions.clone();
    restart_watcher(&state.watcher, state.workspace.clone()).await?;
    // Resident knowledge index: builds in the background (never blocks the
    // first turn) and follows the same change bus as the engines below.
    let knowledge = crate::knowledge::index::start(state.workspace.sandbox().root());
    // Sole OS watcher → code_search worker Index dirty queue (DESIGN §2.9).
    {
        let workspace = state.workspace.clone();
        let engines = state.workspace_engines.clone();
        let runtime = state.runtime.clone();
        let turn_guard = state.turn_guard.clone();
        tokio::spawn(async move {
            let mut rx = workspace.subscribe_changes();
            loop {
                match rx.recv().await {
                    Ok(change) => {
                        knowledge.notify_paths(&change.paths);
                        // Sole ServeState reload of mcp.json / custom_tools.json.
                        // Skip during a turn (same as Settings 409); start_turn reads disk.
                        if !turn_guard.is_turn_in_progress()
                            && change
                                .paths
                                .iter()
                                .any(|p| crate::config::workspace::is_workspace_tool_defs_rel(p))
                        {
                            runtime
                                .write()
                                .expect("runtime lock")
                                .sync_workspace_tool_readiness();
                        }
                        let deleted = change.kind == "deleted";
                        engines
                            .code_search()
                            .notify_fs_changes(&change.paths, deleted);
                        engines
                            .path_index()
                            .notify_fs_changes(&change.paths, deleted);
                        if change
                            .paths
                            .iter()
                            .any(|p| crate::workspace::filter::path_triggers_code_index_sync(p))
                        {
                            engines.request_index_sync(workspace.sandbox().root());
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        // Missed discrete events — force a full disk↔index reconcile
                        // so the semantic index cannot stay silently stale.
                        tracing::warn!(
                            skipped,
                            "workspace change subscriber lagged; requesting index reconcile"
                        );
                        engines.code_search().request_reconcile();
                        engines.path_index().request_rescan();
                        knowledge.notify_lagged();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
    drop(spawn_empty_session_gc(session_gc));
    // Idle maintenance of both session indexes. Never on the search path: a search
    // only reads whatever the index already holds (stale is fine, wrong is not —
    // `build_agent_view` re-checks each hit against the store).
    {
        let engines = state.workspace_engines.clone();
        let turn_guard = state.turn_guard.clone();
        let settled = state.sessions.clone();
        tokio::spawn(async move {
            // WP-166 (P2): start the maintenance cadence only after the first
            // session/list or session/new reply. The loop body is unchanged.
            settled.wait_startup_settled().await;
            let mut tick = tokio::time::interval(SESSION_INDEX_TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                // The first tick completes immediately, so a workspace opens with a
                // maintenance pass behind it: an index that is missing, unreadable,
                // or from an older schema is repaired at startup rather than at the
                // first search.
                tick.tick().await;
                // The sparse pass runs every tick, turn or not, and decides for
                // itself: reconcile when the file is usable, rebuild when it is
                // not. In the steady state that is a key diff over its own file —
                // a search never waits for it, and a corpus written by parallel
                // sessions would otherwise only ever move in front of a query.
                // Frequent passes are the cheap ones: each covers a smaller delta.
                if let Some(reader) = engines.session_reader() {
                    crate::engines::session_search::spawn_sparse_maintenance(&reader);
                }
                // The dense lane is still turn-gated: its pass embeds, and its
                // worker call holds the client lock.
                if turn_guard.is_turn_in_progress() {
                    continue;
                }
                // No-op unless the engine is desired, warm, and the store moved.
                // Real failures are logged inside; nothing to surface here.
                let _ = engines.consume_session_index_work();
            }
        });
    }
    // Periodic incremental code-index pass; never rebuilds, safe during turns.
    {
        let engines = state.workspace_engines.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(CODE_INDEX_TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                // WP-141: the tick now runs the stat-only disk scan (worker IPC),
                // so it leaves the async runtime; the next tick waits for it.
                let engines = engines.clone();
                let _ = tokio::task::spawn_blocking(move || engines.code_index_tick()).await;
            }
        });
    }
    // WP-141: watcher error / Rescan / overflow → full disk reconcile, same
    // entry as the Lagged arm above.
    {
        let workspace = state.workspace.clone();
        let engines = state.workspace_engines.clone();
        tokio::spawn(async move {
            let mut rx = workspace.subscribe_rescan();
            loop {
                match rx.recv().await {
                    Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        tracing::warn!("workspace watcher lost events; requesting index reconcile");
                        let engines = engines.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            engines.code_search().request_reconcile();
                            engines.path_index().request_rescan();
                        })
                        .await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
    let snapshot_upkeep_state = state.clone();
    let apps = state.apps.clone();
    let app = router(state, web_dist);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // WP-166 (P1): snapshot-repo warm, then snapshot maintenance, in the
    // background right after bind. Both hold workspace_snapshot_lock.
    drop(spawn_snapshot_upkeep(&snapshot_upkeep_state));
    let local_addr = listener.local_addr()?;
    let ready_line = format!("LITECODE_READY http://{local_addr}/");
    writeln!(std::io::stdout(), "{ready_line}")?;
    std::io::stdout().flush()?;
    tracing::info!("{ready_line}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown::wait_for_shutdown(shutdown_watch).await;
            sessions.shutdown_cleanup().await;
            mcp_pool.stop_all().await;
            engine_manager.stop_all();
            workspace_engines.stop_all();
            apps.stop_all();
        })
        .await?;
    Ok(())
}

/// WP-166 (P1): warm the shadow snapshot repo and then prune orphan/stale
/// snapshot refs, off the startup path. Same order as before WP-166 (warm ran
/// in `load_workspace_state`, maintenance in `ServeState::with_project`); both
/// take `workspace_snapshot_lock`, so they serialise with turn snapshots.
pub(crate) fn spawn_snapshot_upkeep(state: &ServeState) -> tokio::task::JoinHandle<()> {
    let (workspace_root, snapshots_dir) = {
        let runtime = state.runtime.read().expect("runtime lock");
        (
            runtime.workspace.workspace_root.clone(),
            runtime.workspace.paths.snapshots_dir.clone(),
        )
    };
    let sessions = state.sessions.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(e) =
            crate::session::snapshot::warm_snapshot_repo(&workspace_root, &snapshots_dir)
        {
            tracing::warn!(error = %e, "snapshot warm failed");
        }
        match crate::session::snapshot::maintain_snapshots(&workspace_root, &snapshots_dir, || {
            sessions
                .data()
                .list_session_ids_blocking()
                .unwrap_or_default()
        }) {
            Ok(report) if report.orphans_removed > 0 || report.stale_removed > 0 => {
                tracing::info!(
                    orphans = report.orphans_removed,
                    stale = report.stale_removed,
                    "snapshot maintenance"
                );
            }
            Err(e) => tracing::warn!(error = %e, "snapshot maintenance failed"),
            _ => {}
        }
    })
}

/// Empty-session GC: hourly, TTL `EMPTY_SESSION_TTL`. WP-166 (P2): the first
/// pass waits until the first session/list or session/new reply has gone out;
/// cadence and TTL are unchanged.
pub(crate) fn spawn_empty_session_gc(
    session_gc: std::sync::Arc<crate::session::manager::SessionManager>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        session_gc.wait_startup_settled().await;
        session_gc.gc_stale_empty_sessions(EMPTY_SESSION_TTL).await;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60 * 60)).await;
            session_gc.gc_stale_empty_sessions(EMPTY_SESSION_TTL).await;
        }
    })
}
