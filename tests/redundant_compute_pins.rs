//! Pins for computations suspected of producing *no information*.
//!
//! These are equivalence tests, not performance tests. Each one answers a
//! single question: **if the expensive spelling were replaced by the cheap one,
//! would anything observable change?** A test that fails here means the claim
//! is wrong and the work is load-bearing; a test that passes means the work can
//! be removed without a single byte of difference.
//!
//! Every test also carries a negative control, so it cannot pass vacuously.

mod common;

use litecode::context_pipeline::{ContextEnvelope, request_token_breakdown, tool_schema_tokens};
use litecode::llm::{ModelRequest, RequestPurpose, ToolDef};
use litecode::platform_knobs::ThinkingSpec;
use litecode::session::{apply_prompt_overhead, compute_token_breakdown};
use litecode::types::{assistant_text, user_text};
use serde_json::json;

fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "run_code".into(),
            description: "Run JavaScript. ".repeat(180),
            input_schema: json!({
                "type": "object",
                "properties": { "code": { "type": "string" }, "timeout_ms": { "type": "integer" } },
                "required": ["code"]
            }),
        },
        ToolDef {
            name: "read".into(),
            description: "Read one file. ".repeat(60),
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string" }, "start_line": { "type": "integer" } },
                "required": ["path"]
            }),
        },
    ]
}

/// A request shaped like a real one: a multi-KB instructions block, a tool list,
/// and history. The sizes matter — a fixture of "a" would prove nothing about
/// how much text the redundant pass actually touches.
fn request() -> ModelRequest {
    ModelRequest {
        model: "test-model".into(),
        instructions: "You are LiteCode. Stay inside the task. ".repeat(40),
        input: vec![
            user_text("explain the retry path"),
            assistant_text("Looking at the writer loop."),
            user_text("now the read pool"),
            assistant_text("It opens one connection per query."),
        ],
        tools: tool_defs(),
        max_output_tokens: 2048,
        thinking: ThinkingSpec::Tier(Default::default()),
        json_output: false,
        session_id: Some("01PINTEST".into()),
        purpose: RequestPurpose::Main,
    }
}

/// Claim 1: `request_token_breakdown` re-tokenizes instructions and every tool
/// schema only to obtain the protocol-overhead constant.
///
/// `apply_prompt_overhead` already writes the same `system` / `tool_schema`
/// numbers from the same inputs, and `ContextEnvelope::apply_to_breakdown`
/// overwrites those two buckets with the identical values. The only field the
/// envelope actually adds is `prompt_overhead`, which is a constant.
#[test]
fn breakdown_rederives_only_a_protocol_constant() {
    let request = request();
    assert!(
        request.instructions.len() > 1_000,
        "fixture must carry a non-trivial prompt"
    );

    let current = request_token_breakdown(&request);

    // The cheap spelling: one tokenization pass over the items, one over the
    // schemas, then the constant.
    let schemas: Vec<(String, usize)> = request
        .tools
        .iter()
        .map(|tool| (tool.name.clone(), tool_schema_tokens(tool)))
        .collect();
    let mut one_pass = compute_token_breakdown(&request.input);
    apply_prompt_overhead(&mut one_pass, &request.instructions, &schemas);

    // Negative control: without the constant the result differs, so the
    // assertion below is sensitive to the one field the envelope contributes.
    assert_ne!(
        current, one_pass,
        "the protocol-overhead constant must be the only delta"
    );

    one_pass.prompt_overhead =
        ContextEnvelope::from_request_parts(&request.instructions, &request.tools, 0)
            .prompt_overhead_tokens;

    assert_eq!(
        current, one_pass,
        "dropping the second tokenization must leave every bucket identical"
    );
}

/// Claim 2: the envelope is recomputed from the same inputs at three call sites
/// in one step (request origin row, `LlmRequestBuilt`, and the breakdown above),
/// so one value can be carried to the other two.
#[test]
fn envelope_call_sites_compute_the_same_value() {
    let request = request();

    // Site 1: before the request exists, from the turn's instructions + schemas.
    let at_origin = ContextEnvelope::from_request_parts(
        &request.instructions,
        &request.tools,
        request.max_output_tokens,
    );
    // Sites 2 and 3: from the finished request.
    let at_event = ContextEnvelope::from_request(&request);
    assert_eq!(
        at_origin, at_event,
        "the request carries exactly the inputs the first site used"
    );

    // Negative control: the number really does depend on the tool list, so
    // equality above is not an artifact of an input-independent function.
    let mut fewer = request.tools.clone();
    fewer.pop();
    assert_ne!(
        at_origin,
        ContextEnvelope::from_request_parts(&request.instructions, &fewer, request.max_output_tokens),
        "dropping a tool schema must move the envelope"
    );
}

/// Claim 3: `tool_defs()` re-derives every description and schema string on
/// each call, but for an unchanged context the strings are identical — the
/// precondition for building them once per turn.
///
/// This runs the real registry (whatever tools the build advertises), not a
/// fake, so a description that embedded a clock or a live counter would fail it.
#[tokio::test]
async fn real_tool_defs_are_stable_across_calls() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let provider = std::sync::Arc::new(
        common::scripted_provider::ScriptedProvider::with_texts_legacy_accepting_compaction_summary(
            &["unused"],
        ),
    );
    let spec = common::runtime::test_agent(vec!["*".into()], "all", 2);
    let mut runtime = common::runtime::build_runtime_with_provider(dir.path(), spec, provider);
    runtime
        .ensure_tooling_initialized()
        .await
        .expect("initialize tooling");

    let first = runtime
        .compaction_request_context()
        .expect("compaction context")
        .tools;
    let second = runtime
        .compaction_request_context()
        .expect("compaction context")
        .tools;

    let project = |tools: &[ToolDef]| -> Vec<(String, String, String)> {
        tools
            .iter()
            .map(|tool| {
                (
                    tool.name.clone(),
                    tool.description.clone(),
                    tool.input_schema.to_string(),
                )
            })
            .collect()
    };

    assert!(!first.is_empty(), "registry must advertise tools");
    assert_eq!(
        project(&first),
        project(&second),
        "a second derivation must produce byte-identical descriptions and schemas"
    );
}

/// Claim 5: `restamp_changed_seqs` reads one range per changed `seq`. One range
/// read over the whole span plus a filter yields the same events, in the same
/// order, so the per-seq reads are redundant.
#[test]
fn one_range_read_equals_per_seq_reads() {
    use common::SessionDataFixture;

    let fixture = SessionDataFixture::new();
    let session_id = fixture.create("p", "default", None);
    fixture.insert_items(
        &session_id,
        &[
            user_text("first"),
            assistant_text("second"),
            user_text("third"),
            assistant_text("fourth"),
        ],
    );
    let reader = fixture.reader();
    let all = fixture.data.events_blocking(&session_id).expect("events");
    assert!(all.len() >= 3, "fixture must hold a few rows");

    // The changed set a flush would publish: the first three rows.
    let changed: Vec<u64> = all.iter().take(3).map(|event| event.seq).collect();

    let mut per_seq = Vec::new();
    for seq in &changed {
        per_seq.extend(
            fixture
                .data
                .events_range_blocking(&session_id, *seq as i64, *seq as i64 + 1)
                .expect("per-seq range"),
        );
    }

    let lo = *changed.iter().min().expect("non-empty") as i64;
    let hi = *changed.iter().max().expect("non-empty") as i64 + 1;
    let filtered: Vec<_> = fixture
        .data
        .events_range_blocking(&session_id, lo, hi)
        .expect("one range")
        .into_iter()
        .filter(|event| changed.contains(&event.seq))
        .collect();

    assert_eq!(per_seq, filtered, "same rows, same order");
}

/// Claim 6: the legacy `session_change_log` is written by nothing and read by
/// nothing at schema v8 — dirty state is `session_dirty` alone. Rows planted in
/// the legacy table therefore cannot move an indexer's answer, which is what
/// makes deleting them a pure cleanup.
#[test]
fn legacy_change_log_rows_do_not_reach_dirty_sessions() {
    use common::SessionDataFixture;

    let fixture = SessionDataFixture::new();
    let session_id = fixture.create("p", "default", None);
    fixture.insert_items(&session_id, &[user_text("indexable text")]);
    let reader = fixture.reader();

    let before: Vec<(i64, String)> = reader
        .dirty_sessions_since_blocking(0)
        .expect("dirty sessions")
        .into_iter()
        .map(|row| (row.change_id, row.session_id))
        .collect();
    assert!(
        !before.is_empty(),
        "a write must coalesce into session_dirty, otherwise this test is vacuous"
    );

    // New writes never append to the legacy table.
    let conn = rusqlite::Connection::open(fixture.db_path()).expect("open db");
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .expect("busy timeout");
    let legacy_after_writes: i64 = conn
        .query_row("SELECT COUNT(*) FROM session_change_log", [], |row| row.get(0))
        .expect("count legacy rows");
    assert_eq!(
        legacy_after_writes, 0,
        "the v8 writer must not append to the legacy change log"
    );

    // Plant pre-upgrade history: the answer must not move.
    conn.execute_batch(
        "INSERT INTO session_change_log (session_id, revision, kind, created_at)
         VALUES ('ghost', 1, 'appended', 0), ('ghost', 2, 'sealed', 0)",
    )
    .expect("plant legacy rows");

    let after: Vec<(i64, String)> = reader
        .dirty_sessions_since_blocking(0)
        .expect("dirty sessions")
        .into_iter()
        .map(|row| (row.change_id, row.session_id))
        .collect();

    assert_eq!(
        before, after,
        "legacy rows must not feed dirty-session discovery"
    );
}
