//! Regression guards for the session storage / wire projection boundary:
//!
//! - `SessionSnapshot` flattened wire identity without redundant `meta` envelope (`P1-4`)
//! - Blob spill roundtrip & user message inline invariant (`P1-5` / storage)
//! - Wire `params.cites` derived from `source_seqs` without write-only DB `cites` (`P1-6`)
//! - Compaction anchor & revision lifecycle (`begin_compaction_blocking`)
//!
//! The `src/session/` -> `crate::tool` layer boundary is guarded permanently in
//! `tests/architecture_layers.rs::session_layer_does_not_import_the_tool_layer`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::SessionDataFixture;
use litecode::client_protocol::project;
use litecode::session::data::command::{MutationId, SessionMutation};
use litecode::session::event::EventType;
use litecode::session::manager::SessionManager;
use litecode::types::{assistant_text, user_text};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn open_db(fixture: &SessionDataFixture) -> rusqlite::Connection {
    rusqlite::Connection::open(fixture.db_path()).expect("open sqlite db")
}

fn first_seq(manager: &SessionManager, sid: &str) -> u64 {
    manager
        .data()
        .working_set_blocking(sid)
        .expect("working set")
        .first()
        .and_then(|row| row.log_seq)
        .unwrap_or(1)
}

fn prior_checkpoint(manager: &SessionManager, sid: &str) -> Option<u64> {
    manager
        .data()
        .working_set_blocking(sid)
        .expect("working set")
        .first()
        .and_then(|row| row.checkpoint().and(row.log_seq))
}

fn arc_manager(fixture: &SessionDataFixture) -> std::sync::Arc<SessionManager> {
    std::sync::Arc::new(SessionManager::from_data(
        std::sync::Arc::new(litecode::config::TurnGuard::new()),
        std::sync::Arc::clone(&fixture.data),
    ))
}

/// Locate an on-disk spilled blob by id (the blob layout is private to
/// `session::data`, so this walks the data root instead of assuming a path).
fn find_blob_file(dir: &Path, id: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_blob_file(&path, id) {
                return Some(found);
            }
        } else if path.file_stem().is_some_and(|stem| stem == id) {
            return Some(path);
        }
    }
    None
}

fn blob_id_of(body_ref: &str) -> &str {
    body_ref
        .strip_prefix("[blob:")
        .and_then(|rest| rest.split_once(']').map(|(id, _)| id))
        .expect("body_ref must be `[blob:<id>]`")
}

#[derive(Debug)]
struct LogRow {
    event_type: String,
    body: Option<String>,
    body_ref: Option<String>,
}

fn log_rows(fixture: &SessionDataFixture, sid: &str) -> Vec<LogRow> {
    let conn = open_db(fixture);
    let mut stmt = conn
        .prepare(
            "SELECT event_type, body, body_ref FROM transcript_items
             WHERE session_id = ?1 ORDER BY seq ASC",
        )
        .expect("prepare row query");
    let rows = stmt
        .query_map(rusqlite::params![sid], |row| {
            Ok(LogRow {
                event_type: row.get(0)?,
                body: row.get(1)?,
                body_ref: row.get(2)?,
            })
        })
        .expect("query rows");
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect rows")
}

/// Flattened `SessionSnapshot` fields carry the complete session identity and
/// state (`session_id`, `project`, `agent_id`, `model_id`, `api_model_id`,
/// `label`, `buffer`, `context_meter`, `compaction`) without a redundant `meta`
/// envelope on the wire.
#[test]
fn session_snapshot_identity_complete_without_meta() {
    let binding = litecode::client_protocol::protocol::SessionBindingProjection {
        agent_id: "default".into(),
        model_id: Some("openai/gpt-5".into()),
        api_model_id: "gpt-5".into(),
        label: "GPT-5".into(),
        context_window: 128_000,
        max_output_tokens: 16_000,
        thinking_tier: "high".into(),
        context_mode: "standard".into(),
    };

    let snapshot = project::buffer_snapshot(
        "s1",
        "/p",
        &binding,
        5,
        6,
        2,
        None,
        None,
        None,
        &litecode::context_pipeline::ContextMeter::default(),
        &litecode::context_pipeline::ContextEnvelope::default(),
        Default::default(),
    );

    let wire = serde_json::to_value(&snapshot).expect("serialize snapshot");
    assert_eq!(wire["session_id"], "s1");
    assert_eq!(wire["project"], "/p");
    assert_eq!(wire["agent_id"], "default");
    assert_eq!(wire["model_id"], "openai/gpt-5");
    assert_eq!(wire["api_model_id"], "gpt-5");
    assert_eq!(wire["label"], "GPT-5");
    assert_eq!(wire["buffer"]["last_seq"], 5);
    assert_eq!(wire["buffer"]["next_seq"], 6);
    assert_eq!(wire["buffer"]["revision"], 2);
    assert_eq!(wire["thinking_tier"], "high");
    assert_eq!(wire["context_mode"], "standard");
}

// ===========================================================================
// SECTION 2 — Invariants: behavior the fixes must never break
// ===========================================================================

/// P1-6 guard: oversized non-user rows spill to a blob and read back
/// byte-identically, while oversized user messages stay inlined because the
/// k-anchor SQL requires their body JSON.
#[test]
fn blob_spill_roundtrip_and_user_message_stays_inlined() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/p", "default", None);

    let big_assistant = "A".repeat(40_000);
    let big_user = "B".repeat(40_000);
    let assistant_item = assistant_text(big_assistant.clone());
    let user_item = user_text(big_user.clone());
    fixture.insert_items(&sid, &[assistant_item.clone(), user_item.clone()]);

    let rows = log_rows(&fixture, &sid);
    assert_eq!(rows.len(), 2, "expected one row per inserted item: {rows:?}");

    let assistant_row = rows
        .iter()
        .find(|row| row.event_type == "item/assistant")
        .expect("assistant row");
    assert!(
        assistant_row.body.is_none(),
        "an oversize non-user row must not keep an inline body"
    );
    let body_ref = assistant_row
        .body_ref
        .as_deref()
        .expect("oversize non-user row must spill to body_ref");
    assert!(
        body_ref.starts_with("[blob:"),
        "spill body_ref must use the blob envelope, got {body_ref:?}"
    );

    let user_row = rows
        .iter()
        .find(|row| row.event_type == "item/user")
        .expect("user row");
    assert!(
        user_row.body.is_some() && user_row.body_ref.is_none(),
        "an oversize USER row must stay inlined (anchors read its body JSON); \
         got body_present={} body_ref={:?}",
        user_row.body.is_some(),
        user_row.body_ref
    );

    // The spill actually landed on disk, and reads back byte-identically.
    let data_root = fixture.db_path().parent().expect("db parent").to_path_buf();
    let blob_id = blob_id_of(body_ref);
    assert!(
        find_blob_file(&data_root, blob_id).is_some(),
        "spilled blob {blob_id} must exist under the session data root"
    );

    let events = fixture.data.events_blocking(&sid).expect("read log back");
    let assistant_event = events
        .iter()
        .find(|event| event.event_type == EventType::ItemAssistant)
        .expect("assistant event read back");
    assert_eq!(
        serde_json::to_value(&assistant_item).expect("original item json"),
        assistant_event.data,
        "a spilled row must read back byte-identically to what was written"
    );

    let user_event = events
        .iter()
        .find(|event| event.event_type == EventType::ItemUser)
        .expect("user event read back");
    assert_eq!(
        serde_json::to_value(&user_item).expect("original item json"),
        user_event.data,
        "an inlined user row must read back byte-identically to what was written"
    );
}

/// P1-6 guard: wire `cites` is derived from `source_seqs`, so dropping the DB
/// `cites` column can never break the wire contract.
#[test]
fn compaction_source_seqs_projects_wire_cites_independently() {
    let fixture = SessionDataFixture::new();
    let manager = arc_manager(&fixture);
    let sid = fixture.create("/p", "default", None);
    fixture.insert_items(&sid, &[user_text("first"), user_text("second")]);

    let expected_from = first_seq(&manager, &sid);
    let prior = prior_checkpoint(&manager, &sid);
    fixture.mutate(SessionMutation::Compact {
        session_id: sid.clone(),
        operation_id: MutationId::new(),
        summary: "summary text".to_string(),
        token_estimate: 10,
        kept_from: None,
        at: None,
        expected_prior_checkpoint: prior,
        expected_from,
        usage: None,
        provider_id: None,
    });

    let events = fixture.data.events_blocking(&sid).expect("read log");
    let compacted = events
        .iter()
        .find(|event| event.event_type == EventType::Compacted)
        .expect("compaction row");
    let source_seqs = compacted
        .source_seqs
        .clone()
        .expect("a compaction row carries the seqs it shadows in `source_seqs`");
    assert!(
        !source_seqs.is_empty(),
        "the shadowed surface seqs must be recorded, got {source_seqs:?}"
    );

    // Wire projection: `params.cites` must equal `source_seqs` exactly.
    let wire = project::buffer_log_row(&sid, compacted, None);
    assert_eq!(
        wire.pointer("/params/cites"),
        Some(&serde_json::to_value(&source_seqs).expect("source seqs json")),
        "wire `params.cites` must project the durable `source_seqs`, never a second column"
    );
}

/// Guard for the `begin_compaction_blocking` -> `anchor_compaction_episode`
/// cleanup: re-anchoring an admitted episode sets `at`, bumps the revision, and
/// never creates a second episode.
#[test]
fn compaction_anchor_updates_at_and_bumps_revision_without_duplicate_episode() {
    use litecode::runtime::observer::CompactionTrigger;

    let manager = std::sync::Arc::new(SessionManager::new_for_test(
        std::sync::Arc::new(litecode::config::TurnGuard::default()),
        String::new(),
    ));
    let sid = manager.open_session_sync("/p", "default", None).expect("open");

    let lease = manager
        .try_begin_compaction_episode(&sid, CompactionTrigger::Auto)
        .expect("admit the session's one compaction episode");
    let lease_id = lease.id().to_string();

    // Admission is exclusive: a second claim is refused, so an anchor call can
    // never mint a duplicate episode record.
    assert!(
        manager
            .try_begin_compaction_episode(&sid, CompactionTrigger::Auto)
            .is_err(),
        "a second episode claim must be refused while one is active"
    );

    let before = manager.compaction_state_blocking(&sid);
    assert_eq!(
        before.active.as_ref().map(|active| active.id.as_str()),
        Some(lease_id.as_str())
    );

    let (revision, returned_id) = manager.begin_compaction_blocking(
        &sid,
        Some(lease_id.as_str()),
        CompactionTrigger::Auto,
        Some(41),
    );
    assert_eq!(
        returned_id, lease_id,
        "the anchor must report the episode it was given, not a new id"
    );
    assert_eq!(
        revision,
        before.revision + 1,
        "anchoring must bump the compaction revision so a stale snapshot is droppable"
    );

    let anchored = manager.compaction_state_blocking(&sid);
    let active = anchored.active.expect("episode still active");
    assert_eq!(active.id, lease_id, "the episode identity must not change");
    assert_eq!(active.at, Some(41), "the anchor seq must be recorded");
    assert_eq!(anchored.revision, revision);

    drop(lease);
    let cleared = manager.compaction_state_blocking(&sid);
    assert!(
        cleared.active.is_none(),
        "dropping the episode lease must clear the active compaction state"
    );
    assert!(
        cleared.revision > revision,
        "clearing must advance the revision past the anchored one"
    );
}
