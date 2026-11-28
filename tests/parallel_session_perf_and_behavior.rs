//! P1-7+ Multi-session concurrency, Tokio non-starvation, and performance
//! regression suite.
//!
//! Structure:
//! - RED-FIRST tests: prove `resume_session` holds `records` lock across I/O
//!   (blocking unrelated sessions) and `writer.rs` spawns scoped OS threads on
//!   async writes. Both must FAIL before optimization and PASS after.
//! - GREEN-FIRST + quantitative benchmarks: run 20 parallel sessions on a
//!   2-worker Tokio runtime with real SQLite WAL + blob spills + `SessionController`
//!   to measure before/after wall time, throughput, writer metrics, and Tokio
//!   heartbeat jitter while asserting strict behavioral invariants.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::SessionDataFixture;
use litecode::client_protocol::controller::SessionController;
use litecode::config::global_db;
use litecode::config::{ConfigManager, SettingsWriter, TurnGuard, WorkspaceState};
use litecode::engines::WorkspaceEngines;
use litecode::optional::EngineManager;
use litecode::runtime::RuntimeHandle;
use litecode::session::data::command::{MutationId, SessionMutation};
use litecode::session::event::{EventDraft, EventType};
use litecode::session::manager::SessionManager;
use litecode::session::task_state::{PlanRef, TaskReminders};
use litecode::types::{assistant_text, user_text};
use tempfile::TempDir;

/// Spawn a 5ms Tokio heartbeat ticker on the current runtime and return
/// `(stop_flag, handle)` yielding `(tick_count, max_jitter_ms, p95_jitter_ms)`.
fn spawn_heartbeat() -> (
    Arc<AtomicBool>,
    tokio::task::JoinHandle<(usize, f64, f64)>,
) {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_bg = Arc::clone(&stop);
    let handle = tokio::spawn(async move {
        let interval = Duration::from_millis(5);
        let mut jitters_ms: Vec<f64> = Vec::with_capacity(512);
        let mut last = Instant::now();
        while !stop_bg.load(Ordering::Relaxed) {
            tokio::time::sleep(interval).await;
            let now = Instant::now();
            let elapsed = now.duration_since(last);
            let jitter = elapsed
                .saturating_sub(interval)
                .as_secs_f64()
                * 1000.0;
            jitters_ms.push(jitter);
            last = now;
        }
        jitters_ms.sort_by(|a, b| a.total_cmp(b));
        let count = jitters_ms.len();
        let max_j = jitters_ms.last().copied().unwrap_or(0.0);
        let p95_j = if count == 0 {
            0.0
        } else {
            jitters_ms[((count - 1) * 95) / 100]
        };
        (count, max_j, p95_j)
    });
    (stop, handle)
}

fn build_controller(
    sessions: Arc<SessionManager>,
    workspace_root: &std::path::Path,
) -> SessionController {
    let db = workspace_root.join("global-litecode.db");
    let mut baseline = common::test_resolved("default", &[]).global().clone();
    common::insert_test_llm_registry(&mut baseline, "test-key");
    common::seed_test_catalog(&db, "http://127.0.0.1:9", 128_000);
    global_db::import_into(&db, &baseline).expect("seed global db");

    let guard = Arc::new(TurnGuard::new());
    let engine_manager = Arc::new(EngineManager::new());
    let mut writer = SettingsWriter::with_path(&db, guard);
    writer.set_engine_manager(Arc::clone(&engine_manager));
    let revision = writer.revision_handle();
    let settings = writer.load_settings().expect("load");
    let workspace = WorkspaceState::new(workspace_root);
    let resolved = ConfigManager::resolve(settings, workspace.clone(), common::catalog_for_db(&db));
    let workspace_engines = Arc::new(WorkspaceEngines::new());
    let ide = litecode::ide_base::IdeBaseHandle::open(
        workspace.workspace_root.clone(),
        Arc::clone(&workspace_engines),
    )
    .expect("ide");
    let runtime = RuntimeHandle::new(
        resolved,
        "default".into(),
        workspace,
        engine_manager,
        workspace_engines,
        ide,
        revision,
        &db,
    );
    SessionController::with_turn_guard(runtime, None, sessions).expect("controller")
}

// ===========================================================================
// GUARD #1 (P0): resume_session must NOT hold global `records` lock across I/O
// ===========================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_session_must_not_hold_records_lock_across_io() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("sessions.db");
    let db_str = db_path.to_string_lossy().to_string();

    // Seed two sessions on disk: sid1 has a stale active plan (triggers SaveTaskState
    // write during resume_session); sid2 is a clean session.
    let (sid1, sid2) = {
        let seed_mgr = SessionManager::new_for_test(Arc::new(TurnGuard::new()), db_str.clone());
        let s1 = seed_mgr
            .open_session("/proj", "default", Some("test/test-primary-model"))
            .await
            .expect("create sid1");
        let s2 = seed_mgr
            .open_session("/proj", "default", Some("test/test-primary-model"))
            .await
            .expect("create sid2");
        let rev1 = seed_mgr.data().revision(&s1).await.expect("rev1");
        seed_mgr
            .data()
            .mutate(SessionMutation::SaveTaskState {
                session_id: s1.clone(),
                expected_revision: rev1,
                operation_id: MutationId::new(),
                state: TaskReminders {
                    todos: vec![],
                    active_plan: Some(PlanRef::new("nonexistent-stale-plan")),
                },
            })
            .await
            .expect("seed stale plan");
        (s1, s2)
    };

    // Open a fresh SessionManager so both sessions are cold (absent from `records`).
    let mgr = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_str,
    ));
    let hooks = mgr.data().hooks();
    hooks.pause();

    let mgr_bg = Arc::clone(&mgr);
    let sid1_bg = sid1.clone();
    let t1 = tokio::spawn(async move { mgr_bg.ensure_entry(&sid1_bg).await });

    // Wait until sid1's SaveTaskState write actually reached the paused writer.
    let hooks_wait = Arc::clone(&hooks);
    tokio::task::spawn_blocking(move || hooks_wait.wait_until_parked())
        .await
        .expect("wait parked");

    // While sid1's SQLite write is in flight, the global `records` mutex MUST be free
    // and an unrelated cold session `sid2` MUST resume without blocking on `sid1`!
    // Use an OS watchdog thread to unpause the writer after 600ms so a buggy
    // `records.lock()` hold across I/O fails fast instead of deadlocking the test.
    let lock_free_during_io = mgr.records_lock_free();
    let hooks_watchdog = Arc::clone(&hooks);
    let sid2_done = Arc::new(AtomicBool::new(false));
    let sid2_done_bg = Arc::clone(&sid2_done);
    let watchdog_fired = Arc::new(AtomicBool::new(false));
    let watchdog_fired_bg = Arc::clone(&watchdog_fired);
    let watchdog = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(600);
        while Instant::now() < deadline {
            if sid2_done_bg.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        watchdog_fired_bg.store(true, Ordering::SeqCst);
        hooks_watchdog.resume();
    });

    let sid2_started = Instant::now();
    let sid2_res = mgr.ensure_entry(&sid2).await;
    let sid2_elapsed = sid2_started.elapsed();
    sid2_done.store(true, Ordering::SeqCst);

    // Always unpause before joining t1.
    hooks.resume();
    let _ = watchdog.join();
    t1.await
        .expect("join sid1")
        .expect("sid1 resume should finish cleanly");

    assert!(
        lock_free_during_io,
        "RED P0: resume_session held the global `records` Mutex while performing SQLite write I/O!"
    );
    assert!(
        !watchdog_fired.load(Ordering::SeqCst) && sid2_res.is_ok(),
        "RED P0: cold ensure_entry(sid2) stalled for {:?} waiting on sid1's SQLite write (watchdog had to unpause writer)!",
        sid2_elapsed
    );
}

// ===========================================================================
// GUARD #2 (P1-7): writer.rs must NOT spawn OS threads per async write
// ===========================================================================

#[test]
fn writer_must_not_spawn_os_thread_per_async_write() {
    let writer_src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/session/data/writer.rs"),
    )
    .expect("read writer.rs");
    // Strip #[cfg(test)] tail (only inspect production portion).
    let prod = writer_src
        .split("#[cfg(test)]")
        .next()
        .unwrap_or(&writer_src);
    assert!(
        !prod.contains("std::thread::scope"),
        "RED P1-7: src/session/data/writer.rs still bridges blocking writes from Tokio via std::thread::scope (1 OS thread spawn + worker stall per write)!"
    );
}

// ===========================================================================
// BENCHMARK + INVARIANT #1: 20 parallel sessions write burst on 2-worker Tokio
// ===========================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn perf_and_behavior_20_parallel_sessions_write_burst() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("sessions.db");
    let mgr = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path.to_string_lossy().to_string(),
    ));

    const SESSIONS: usize = 20;
    const STEPS_PER_SESSION: usize = 8;

    let mut sids = Vec::with_capacity(SESSIONS);
    for _ in 0..SESSIONS {
        let sid = mgr
            .open_session("/proj", "default", Some("test/test-primary-model"))
            .await
            .expect("open session");
        sids.push(sid);
    }

    let (stop_hb, hb_handle) = spawn_heartbeat();
    let started = Instant::now();

    let mut joins = Vec::with_capacity(SESSIONS);
    for (idx, sid) in sids.iter().cloned().enumerate() {
        let m = Arc::clone(&mgr);
        joins.push(tokio::spawn(async move {
            let big_blob = format!("session-{idx}-{}", "X".repeat(38_000));
            for step in 0..STEPS_PER_SESSION {
                // Mix of realistic per-step hot-path writes:
                // 1) item/user + assistant (every 4th step spills a 38KB blob)
                let reply = if step % 4 == 0 {
                    big_blob.clone()
                } else {
                    format!("reply s{idx} step{step}")
                };
                m.insert_detail_rows(
                    &sid,
                    &[
                        user_text(format!("prompt s{idx} step{step}")),
                        assistant_text(reply),
                    ],
                )
                .await
                .expect("insert detail rows");
                // 2) task state update
                m.save_task_state(&sid).await.expect("save task state");
                // 3) thinking tier / metadata write
                let tier = if step % 2 == 0 {
                    litecode::platform_knobs::ThinkingTier::High
                } else {
                    litecode::platform_knobs::ThinkingTier::Medium
                };
                m.set_thinking_tier(&sid, tier)
                    .await
                    .expect("set thinking tier");
            }
        }));
    }

    for j in joins {
        j.await.expect("session task join");
    }
    let wall = started.elapsed();
    stop_hb.store(true, Ordering::Relaxed);
    let (hb_ticks, hb_max_ms, hb_p95_ms) = hb_handle.await.expect("heartbeat join");
    let wm = mgr.data().writer_metrics();

    let total_mutations = SESSIONS * STEPS_PER_SESSION * 3;
    let throughput = total_mutations as f64 / wall.as_secs_f64().max(0.0001);
    println!(
        "[BENCH:write_burst] sessions={SESSIONS} total_writes={total_mutations} wall_ms={:.1} writes_per_sec={:.1} commit_p50_ms={:?} commit_p95_ms={:?} queue_wait_p95_ms={:?} hb_ticks={hb_ticks} hb_max_jitter_ms={:.2} hb_p95_jitter_ms={:.2}",
        wall.as_secs_f64() * 1000.0,
        throughput,
        wm.commit_p50_ms,
        wm.commit_p95_ms,
        wm.queue_wait_p95_ms,
        hb_max_ms,
        hb_p95_ms,
    );

    // Behavioral invariants: every single session must have exact row counts,
    // contiguous seqs, and intact blob roundtrip.
    for (idx, sid) in sids.iter().enumerate() {
        let rows = mgr.data().working_set_blocking(sid).expect("working set");
        assert_eq!(
            rows.len(),
            STEPS_PER_SESSION * 2,
            "session {idx} lost rows"
        );
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(
                row.log_seq,
                Some(i as u64),
                "session {idx} seq gap at index {i}"
            );
        }
        let expected_blob = format!("session-{idx}-{}", "X".repeat(38_000));
        let first_reply = rows[1]
            .item()
            .map(litecode::types::item_text_preview)
            .unwrap_or_default();
        assert_eq!(
            first_reply, expected_blob,
            "session {idx} blob spill roundtrip drifted"
        );
    }
    assert_eq!(wm.failed, 0, "writer reported failed commits");
    assert_eq!(wm.conflicts, 0, "unexpected revision conflicts across disjoint sessions");
}

// ===========================================================================
// BENCHMARK + INVARIANT #2: session/list with 20 populated subagent sessions + blobs
// ===========================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn perf_and_behavior_session_list_20_subagent_sessions_with_blobs() {
    let fixture = SessionDataFixture::new();
    let sessions = Arc::new(fixture.manager());
    let root = fixture.dir.path();

    let parent_sid = sessions
        .open_session("/proj", "default", Some("test/test-primary-model"))
        .await
        .expect("root session");

    const CHILDREN: usize = 20;
    let mut child_ids = Vec::with_capacity(CHILDREN);
    let big_payload = "Z".repeat(42_000);
    for i in 0..CHILDREN {
        let child_sid = sessions
            .open_child_session_with_responsibility(
                "/proj",
                "explore",
                Some("test/test-primary-model"),
                &parent_sid,
                &format!("call_{i}"),
                &format!("audit shard {i}"),
            )
            .await
            .expect("open child");
        // Seed 20 rows including 2 spilled blobs + terminal turn/end with cancelled_by=user.
        let mut items = Vec::with_capacity(20);
        for k in 0..10 {
            items.push(user_text(format!("child {i} step {k}")));
            if k == 2 || k == 7 {
                items.push(assistant_text(format!("blob-{i}-{k}-{big_payload}")));
            } else {
                items.push(assistant_text(format!("answer {i} step {k}")));
            }
        }
        sessions
            .insert_detail_rows(&child_sid, &items)
            .await
            .expect("seed child transcript");
        sessions
            .apply(
                &child_sid,
                litecode::session::store::SessionApply::Append(EventDraft {
                    time: chrono::Utc::now().timestamp_millis(),
                    event_type: EventType::TurnEnd,
                    data: serde_json::json!({
                        "turn": format!("turn-{i}"),
                        "reason": "cancelled",
                        "cancelled_by": "user",
                    }),
                    surface_op: None,
                    source_seqs: None,
                    ignorable: false,
                    state: litecode::session::LogState::Final,
                }),
            )
            .await
            .expect("append turn/end");
        child_ids.push(child_sid);
    }

    let mut ctrl = build_controller(Arc::clone(&sessions), root);

    let (stop_hb, hb_handle) = spawn_heartbeat();
    let started = Instant::now();
    const CALLS: usize = 10;
    let mut last_list = None;
    for _ in 0..CALLS {
        last_list = Some(ctrl.list_sessions().await.expect("list_sessions"));
    }
    let wall = started.elapsed();
    stop_hb.store(true, Ordering::Relaxed);
    let (hb_ticks, hb_max_ms, hb_p95_ms) = hb_handle.await.expect("heartbeat join");

    println!(
        "[BENCH:session_list] children={CHILDREN} calls={CALLS} wall_ms={:.1} per_call_ms={:.2} hb_ticks={hb_ticks} hb_max_jitter_ms={:.2} hb_p95_jitter_ms={:.2}",
        wall.as_secs_f64() * 1000.0,
        wall.as_secs_f64() * 1000.0 / CALLS as f64,
        hb_max_ms,
        hb_p95_ms,
    );

    // Behavioral invariants: every child session is listed with exact turn/end metadata.
    let list = last_list.expect("list");
    for cid in &child_ids {
        let info = list
            .sessions
            .iter()
            .find(|s| &s.id == cid)
            .unwrap_or_else(|| panic!("missing child session {cid} in session/list"));
        assert_eq!(info.last_turn_reason, "cancelled");
        assert_eq!(info.last_turn_cancelled_by.as_deref(), Some("user"));
    }
}
