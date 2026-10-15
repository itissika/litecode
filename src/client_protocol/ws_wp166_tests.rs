//! WP-166 T1: `session/list` is answered off the serial session loop, so it is
//! never queued behind a subscribe/replay; subscribe/replay frames and their
//! order are unchanged.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use crate::config::{
    ConfigManager, SettingsWriter, TurnGuard, WorkspacePaths, WorkspaceState, init_workspace,
};
use crate::engines::WorkspaceEngines;
use crate::optional::EngineManager;
use crate::serve::ServeState;
use crate::session::manager::resume_gate_for_test;

const TOKEN: &str = "wp166-token";
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Env {
    _project: TempDir,
    _db: TempDir,
    state: ServeState,
    url: String,
}

async fn env() -> Env {
    let project = TempDir::new().expect("project");
    let db = TempDir::new().expect("db");
    let root = project.path().to_path_buf();
    let global_db = db.path().join("litecode.db");
    init_workspace(&root).expect("init workspace");
    let workspace_id = crate::config::peek_workspace_id(&root).expect("workspace id");
    let workspace = WorkspaceState {
        workspace_root: root.clone(),
        workspace_id: workspace_id.clone(),
        contract: String::new(),
        paths: WorkspacePaths::for_workspace(&root, &workspace_id),
        workspace_tool_readiness: Default::default(),
        workspace_mcp_servers: Default::default(),
        workspace_custom_tools: Default::default(),
    };
    let settings = ConfigManager::load_global_from(&global_db).expect("global");
    let resolved = ConfigManager::resolve(
        settings,
        workspace.clone(),
        crate::provider_catalog::shared_for_db(&global_db).expect("catalog"),
    );
    let turn_guard = Arc::new(TurnGuard::new());
    let engine_manager = Arc::new(EngineManager::new());
    let mut writer = SettingsWriter::with_path(&global_db, turn_guard.clone());
    writer.set_engine_manager(Arc::clone(&engine_manager));
    let state = ServeState::with_project(
        resolved,
        "default".into(),
        workspace,
        engine_manager,
        Arc::new(WorkspaceEngines::new()),
        None,
        Some(TOKEN.into()),
        root.clone(),
        turn_guard,
        Arc::new(writer),
    )
    .expect("serve state");
    let web_dist = root.join("web/dist");
    std::fs::create_dir_all(&web_dist).expect("web dist");
    std::fs::write(web_dist.join("index.html"), "<!doctype html>").expect("index");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = crate::serve::router::router(state.clone(), web_dist);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Env {
        _project: project,
        _db: db,
        state,
        url: format!("ws://{addr}/ws?token={TOKEN}"),
    }
}

/// A persisted session with one turn's worth of rows (listed: not empty).
fn seeded_session(state: &ServeState, text: &str) -> String {
    let data = state.sessions.data();
    let sid = data
        .create_session("/proj", "default", None)
        .expect("create session");
    data.insert_items(&sid, &[crate::types::user_text(text)])
        .expect("insert items");
    sid
}

async fn connect(url: &str) -> Socket {
    tokio_tungstenite::connect_async(url).await.expect("connect").0
}

async fn send(ws: &mut Socket, id: u64, method: &str, params: Value) {
    ws.send(Message::Text(
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
            .to_string()
            .into(),
    ))
    .await
    .expect("send");
}

async fn next_frame(ws: &mut Socket) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(15), ws.next())
            .await
            .expect("frame timeout")
            .expect("socket open")
            .expect("ws message");
        if let Message::Text(text) = msg {
            return serde_json::from_str(&text).expect("json frame");
        }
    }
}

/// Read frames until the reply with `id`; returns (reply, frames before it).
async fn until_reply(ws: &mut Socket, id: u64) -> (Value, Vec<Value>) {
    let mut seen = Vec::new();
    loop {
        let frame = next_frame(ws).await;
        if frame.get("id").and_then(Value::as_u64) == Some(id) {
            return (frame, seen);
        }
        seen.push(frame);
    }
}

fn reply_id(frame: &Value) -> Option<u64> {
    if frame.get("method").is_some() {
        return None;
    }
    frame.get("id").and_then(Value::as_u64)
}

fn snapshot_for<'a>(frames: &'a [Value], sid: &str) -> Vec<&'a Value> {
    frames
        .iter()
        .filter(|f| f.get("method").and_then(Value::as_str) == Some("session/snapshot"))
        .filter(|f| f.pointer("/params/session_id").and_then(Value::as_str) == Some(sid))
        .collect()
}

fn listed_ids(reply: &Value) -> Vec<String> {
    reply
        .pointer("/result/sessions")
        .and_then(Value::as_array)
        .expect("sessions array")
        .iter()
        .filter_map(|s| s.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

async fn gate_entered(gate: &resume_gate_for_test::Gate) {
    tokio::time::timeout(Duration::from_secs(10), gate.entered.notified())
        .await
        .expect("subscribe never reached resume_session");
}

/// T1-1: while a subscribe is parked inside resume_session, session/list on the
/// same connection is answered, with the full list; the subscribe reply only
/// arrives once the replay is released, and its Snapshot precedes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_list_not_blocked_by_inflight_subscribe() {
    let env = env().await;
    let sid = seeded_session(&env.state, "hello");
    let gate = resume_gate_for_test::install(&sid);
    let mut ws = connect(&env.url).await;

    send(&mut ws, 1, "session/subscribe", json!({"session_id": sid})).await;
    gate_entered(&gate).await;
    send(&mut ws, 2, "session/list", json!({})).await;

    let (list, before_list) = until_reply(&mut ws, 2).await;
    assert!(
        before_list.iter().all(|f| reply_id(f) != Some(1)),
        "subscribe reply must not precede the list reply while replay is parked"
    );
    assert!(listed_ids(&list).contains(&sid), "list must include the session");
    assert!(env.state.sessions.is_startup_settled(), "list reply marks startup settled");

    gate.release.add_permits(1);
    let (sub, before_sub) = until_reply(&mut ws, 1).await;
    assert!(sub.get("error").is_none(), "subscribe ok: {sub}");
    assert_eq!(snapshot_for(&before_sub, &sid).len(), 1, "one Snapshot before the reply");
}

/// T1-2: the fast-path list equals the controller's list row for row, including
/// the WP-091 rules (empty primary hidden, empty child kept).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_list_fastlane_matches_controller_list() {
    let env = env().await;
    let full = seeded_session(&env.state, "has rows");
    let data = env.state.sessions.data();
    let empty_primary = data
        .create_session("/proj", "default", None)
        .expect("empty primary");
    let mut ws = connect(&env.url).await;
    send(&mut ws, 1, "session/list", json!({})).await;
    let (reply, _) = until_reply(&mut ws, 1).await;

    let resolved = env.state.runtime.read().unwrap().resolved.clone();
    let direct = crate::client_protocol::controller::build_session_list(&env.state.sessions, &resolved)
        .await
        .expect("direct list");
    assert_eq!(
        reply.pointer("/result/sessions").cloned().unwrap(),
        serde_json::to_value(&direct).unwrap(),
        "ws fast path and controller list must return identical rows"
    );
    let ids = listed_ids(&reply);
    assert!(ids.contains(&full));
    assert!(!ids.contains(&empty_primary), "empty primary stays hidden");
}

/// T1-3 / T1-4: the Snapshot a subscribe produces, and the frames around it,
/// are identical whether or not a list was answered during the replay. The
/// Snapshot carries the seq cursor deltas continue from, so identical content
/// means identical seq continuity.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_replay_frames_unchanged_with_concurrent_list() {
    let env = env().await;
    let sid = seeded_session(&env.state, "replay me");

    // B: replay parked, list answered in the middle.
    let gate = resume_gate_for_test::install(&sid);
    let mut b = connect(&env.url).await;
    send(&mut b, 1, "session/subscribe", json!({"session_id": sid})).await;
    gate_entered(&gate).await;
    send(&mut b, 2, "session/list", json!({})).await;
    let _ = until_reply(&mut b, 2).await;
    gate.release.add_permits(1);
    let (_, b_frames) = until_reply(&mut b, 1).await;

    // A: plain subscribe on a fresh connection, no list.
    let mut a = connect(&env.url).await;
    send(&mut a, 1, "session/subscribe", json!({"session_id": sid})).await;
    let (_, a_frames) = until_reply(&mut a, 1).await;

    let a_snap = snapshot_for(&a_frames, &sid);
    let b_snap = snapshot_for(&b_frames, &sid);
    assert_eq!(a_snap.len(), 1);
    assert_eq!(b_snap.len(), 1);
    assert_eq!(a_snap[0], b_snap[0], "Snapshot content (incl. seq cursor) unchanged");
    let next_seq = b_snap[0].pointer("/params/buffer/next_seq");
    assert!(next_seq.is_some(), "Snapshot exposes the seq cursor: {}", b_snap[0]);
}

/// T1-5: nothing a subscribe produced is held back by the fast list: after the
/// subscribe reply, no further Snapshot for the session turns up later.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_projection_frames_stranded_after_fastlane_list() {
    let env = env().await;
    let sid = seeded_session(&env.state, "x");
    let mut ws = connect(&env.url).await;
    send(&mut ws, 1, "session/subscribe", json!({"session_id": sid})).await;
    send(&mut ws, 2, "session/list", json!({})).await;
    let mut frames = Vec::new();
    let mut got = (false, false);
    while !(got.0 && got.1) {
        let f = next_frame(&mut ws).await;
        match reply_id(&f) {
            Some(1) => got.0 = true,
            Some(2) => got.1 = true,
            _ => {}
        }
        frames.push(f);
    }
    let sub_at = frames.iter().position(|f| reply_id(f) == Some(1)).unwrap();
    assert_eq!(snapshot_for(&frames[..sub_at], &sid).len(), 1);
    let late = tokio::time::timeout(Duration::from_millis(300), next_frame(&mut ws)).await;
    if let Ok(frame) = late {
        assert!(
            snapshot_for(std::slice::from_ref(&frame), &sid).is_empty(),
            "stranded Snapshot flushed late: {frame}"
        );
    }
}

/// T1-6: loop-owned requests keep arrival order on one connection:
/// subscribe → unsubscribe → subscribe replies in order, each Snapshot before
/// its own reply; the list reply may land anywhere.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_unsubscribe_order_preserved_with_list() {
    let env = env().await;
    let sid = seeded_session(&env.state, "order");
    let mut ws = connect(&env.url).await;
    send(&mut ws, 1, "session/subscribe", json!({"session_id": sid})).await;
    send(&mut ws, 2, "session/unsubscribe", json!({"session_id": sid})).await;
    send(&mut ws, 3, "session/list", json!({})).await;
    send(&mut ws, 4, "session/subscribe", json!({"session_id": sid})).await;
    let mut frames = Vec::new();
    while !(1..=4).all(|id| frames.iter().any(|f| reply_id(f) == Some(id))) {
        frames.push(next_frame(&mut ws).await);
    }
    let pos = |id| frames.iter().position(|f| reply_id(f) == Some(id)).unwrap();
    assert!(pos(1) < pos(2) && pos(2) < pos(4), "loop order kept: {frames:?}");
    let snaps: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|(_, f)| !snapshot_for(std::slice::from_ref(*f), &sid).is_empty())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(snaps.len(), 2, "one Snapshot per subscribe");
    assert!(snaps[0] < pos(1) && pos(2) < snaps[1] && snaps[1] < pos(4));
}

/// T1-7: when the list read fails, the request falls back to the loop branch,
/// which still emits the operation error frame and then the -32000 reply.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_list_error_emits_operation_error_and_rpc_error() {
    let env = env().await;
    let db = env
        .state
        .runtime
        .read()
        .unwrap()
        .workspace
        .paths
        .sessions_db
        .clone();
    let conn = rusqlite::Connection::open(&db).expect("open db");
    conn.busy_timeout(Duration::from_secs(5)).unwrap();
    conn.execute_batch("ALTER TABLE sessions RENAME TO sessions_wp166_gone;")
        .expect("break sessions table");
    let mut ws = connect(&env.url).await;
    send(&mut ws, 1, "session/list", json!({})).await;
    let (reply, before) = until_reply(&mut ws, 1).await;
    assert_eq!(reply.pointer("/error/code").and_then(Value::as_i64), Some(-32000));
    assert!(
        before
            .iter()
            .any(|f| f.pointer("/params/op").is_some()
                && f.pointer("/params/ok") == Some(&Value::Bool(false))),
        "operation error frame precedes the rpc error: {before:?}"
    );
}

/// WP-166 T2-6: building ServeState no longer prunes snapshots inline; the
/// P1 upkeep task (warm, then maintenance under the lock) does, with the same
/// orphan classification as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_state_does_not_run_snapshot_maintenance_inline() {
    let env = env().await;
    let (root, snaps) = {
        let rt = env.state.runtime.read().unwrap();
        (rt.workspace.workspace_root.clone(), rt.workspace.paths.snapshots_dir.clone())
    };
    std::fs::write(root.join("a.txt"), "v0").unwrap();
    crate::session::snapshot::snapshot_track(&root, &snaps, "orphan-wp166", 0).unwrap();
    // A second ServeState over the same workspace would have pruned inline
    // before WP-166; construction alone must now leave the orphan alone.
    assert!(crate::session::snapshot::snapshot_exists(&snaps, "orphan-wp166", 0));

    crate::serve::router::spawn_snapshot_upkeep(&env.state)
        .await
        .expect("upkeep task");
    assert!(
        !crate::session::snapshot::snapshot_exists(&snaps, "orphan-wp166", 0),
        "P1 upkeep removes the orphan"
    );
}

/// WP-166 T3-1: the startup-settled signal fires on the first session/new
/// reply (or list reply, see T1-1) and only then; there is no timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_settled_fires_on_first_new_reply_only() {
    let env = env().await;
    assert!(!env.state.sessions.is_startup_settled());
    let sessions = env.state.sessions.clone();
    let waiter = tokio::spawn(async move { sessions.wait_startup_settled().await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!waiter.is_finished(), "nothing settles startup before a list/new reply");

    let mut ws = connect(&env.url).await;
    send(&mut ws, 1, "session/new", json!({})).await;
    let (reply, _) = until_reply(&mut ws, 1).await;
    assert!(reply.pointer("/result/session_id").is_some(), "new ok: {reply}");
    tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("settled after new reply")
        .unwrap();
    assert!(env.state.sessions.is_startup_settled());
}

/// WP-166 T3-3: the first empty-session GC pass is deferred until the first
/// list/new reply, then removes exactly what it removed before (stale, empty,
/// unsubscribed primary sessions).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc_stale_empty_sessions_deferred_until_settled() {
    let env = env().await;
    let data = env.state.sessions.data().clone();
    let stale_empty = data
        .create_session("/proj", "default", None)
        .expect("stale empty");
    let kept = seeded_session(&env.state, "has rows");
    let db = env.state.runtime.read().unwrap().workspace.paths.sessions_db.clone();
    let conn = rusqlite::Connection::open(&db).expect("open db");
    conn.busy_timeout(Duration::from_secs(5)).unwrap();
    conn.execute(
        "UPDATE sessions SET updated_at = 0 WHERE id IN (?1, ?2)",
        rusqlite::params![stale_empty, kept],
    )
    .expect("age sessions");

    let gc = crate::serve::router::spawn_empty_session_gc(env.state.sessions.clone());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(data.meta_blocking(&stale_empty).is_ok(), "no GC before settled");

    let mut ws = connect(&env.url).await;
    send(&mut ws, 1, "session/list", json!({})).await;
    let _ = until_reply(&mut ws, 1).await;
    let gone = tokio::time::timeout(Duration::from_secs(5), async {
        while data.meta_blocking(&stale_empty).is_ok() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(gone.is_ok(), "GC runs once startup has settled");
    assert!(data.meta_blocking(&kept).is_ok(), "non-empty session kept");
    gc.abort();
}

/// WP-166 §9: clients that never send session/list or session/new still
/// release P2 work — via the first agent/run (WS) or the first HTTP
/// retrieval search.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_settled_fires_without_list_or_new() {
    // agent/run on a `?session=` connection (no list/new ever sent).
    let env = env().await;
    let sid = seeded_session(&env.state, "hinted");
    let url = format!("{}&session={sid}", env.url);
    let mut ws = connect(&url).await;
    assert!(!env.state.sessions.is_startup_settled());
    send(&mut ws, 1, "agent/run", json!({"session_id": sid, "input": ""})).await;
    let _ = until_reply(&mut ws, 1).await;
    assert!(env.state.sessions.is_startup_settled(), "first agent/run settles startup");

    // HTTP-only retrieval search, no WS at all.
    let env = env().await;
    assert!(!env.state.sessions.is_startup_settled());
    let http = env.url.replace("ws://", "http://").replace("/ws?token=", "/api/workspace/retrieval/search?token=");
    let _ = reqwest::Client::new()
        .post(&http)
        .bearer_auth(TOKEN)
        .json(&json!({"query": "anything"}))
        .send()
        .await
        .expect("http search");
    assert!(env.state.sessions.is_startup_settled(), "first retrieval search settles startup");
}
