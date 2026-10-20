//! LLM-request view of `run_code`. The next turn's `function_call_output` is
//! what the model actually sees.

mod common;

use std::sync::Arc;

use common::fake_deps::{assistant_text_item, function_call_item};
use common::{RecordingProvider, build_runtime_with_provider, test_agent};
use serde_json::{Value, json};

fn output_text(input: &[Value], call_id: &str) -> String {
    for item in input {
        if item.get("call_id").and_then(Value::as_str) != Some(call_id) {
            continue;
        }
        let Some(output) = item.get("output") else {
            continue;
        };
        if let Some(text) = output.as_str() {
            return text.to_string();
        }
        if let Some(text) = output.get("text").and_then(Value::as_str) {
            return text.to_string();
        }
    }
    panic!("no function_call_output for {call_id}");
}

async fn captured_run_code(code: &str) -> String {
    captured_run(json!({ "code": code }), |_| {}).await
}

async fn captured_run(arguments: Value, prepare: impl FnOnce(&std::path::Path)) -> String {
    captured_with(|root| {
        prepare(root);
        arguments
    })
    .await
}

async fn captured_with(build: impl FnOnce(&std::path::Path) -> Value) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let arguments = build(dir.path());
    let provider = RecordingProvider::new(vec![
        vec![function_call_item(
            "call_view",
            "run_code",
            &arguments.to_string(),
            "fc_view",
        )],
        vec![assistant_text_item("done", "msg_done")],
    ]);
    let captured_handle = provider.clone();
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(
            vec![
                "read".into(),
                "grep".into(),
                "glob".into(),
                "edit".into(),
                "write".into(),
            ],
            "workspace",
            6,
        ),
        Arc::new(provider),
    );
    runtime
        .run("show the run")
        .await
        .expect("turn completes");
    let captured = captured_handle.captured();
    assert!(
        captured.len() >= 2,
        "expected the tool result on the next request, got {}",
        captured.len()
    );
    output_text(&captured[1].input, "call_view")
}

#[tokio::test(flavor = "current_thread")]
async fn large_result_is_budgeted_in_the_request() {
    let text = captured_run_code(r#"return "A".repeat(200000) + "END_SENTINEL";"#).await;
    assert!(text.len() <= 65_536, "{}", text.len());
    assert!(!text.starts_with("[blob:"), "{text}");
    assert!(text.contains("END_SENTINEL"), "{text}");
    assert!(text.contains("bytes omitted"), "{text}");
    assert!(text.starts_with("run_code ok"), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn cjk_result_does_not_panic() {
    let text = captured_run_code(r#"return "中".repeat(300000);"#).await;
    assert!(std::str::from_utf8(text.as_bytes()).is_ok());
    assert!(text.len() <= 65_536, "{}", text.len());
    assert!(text.contains('中'), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn small_result_is_verbatim() {
    let text = captured_run_code(r#"return "hello-small";"#).await;
    assert!(text.contains("result:\nhello-small"), "{text}");
    assert!(!text.contains("@@run_code_data"), "{text}");
    assert!(!text.contains("\"hello-small\""), "{text}");
    assert!(text.starts_with("run_code ok"), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_keeps_the_log_tail_and_names_the_budget() {
    let text = captured_run(
        json!({
            "code": "console.log('LOG_TAIL_SENTINEL'); while (true) {}",
            "timeout_ms": 300
        }),
        |_| {},
    )
    .await;
    assert!(text.contains("timeout after 300ms"), "{text}");
    assert!(text.contains("LOG_TAIL_SENTINEL"), "{text}");
    assert!(text.len() <= 65_536, "{}", text.len());
}

#[tokio::test(flavor = "current_thread")]
async fn syntax_error_shows_the_source_line() {
    let text = captured_run(json!({ "code": "const x = 1;\nreturn (;" }), |_| {}).await;
    assert!(text.contains("line 2, column"), "{text}");
    assert!(text.contains("return (;"), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn exception_shows_boom_and_the_adjusted_frame() {
    let text = captured_run(
        json!({
            "code": "const ready = 1;\nfunction boom() {\n  throw new Error(\"boom\");\n}\nreturn boom();\n"
        }),
        |_| {},
    )
    .await;
    assert!(text.contains("boom"), "{text}");
    assert!(text.contains("program.js:3"), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn many_reads_budget_the_ledger_and_mark_a_hidden_session_read() {
    let text = captured_with(|root| {
        std::fs::write(root.join("probe.txt"), "ok").unwrap();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let session_id = {
            let lease = litecode::session::WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
            let data = litecode::session::SessionData::open(&lease, &db).unwrap();
            let id = data
                .create_session(root.to_str().unwrap(), "default", None)
                .unwrap();
            data.insert_items(&id, &[litecode::types::user_text("session body")])
                .unwrap();
            id
        };
        let code = format!(
            r#"
                for (let i = 0; i < 466; i++) {{
                    const path = i === 299 ? ".litecode/sessions/{session_id}.md" : "probe.txt";
                    await tools.read({{ path }});
                }}
                return "done";
            "#
        );
        json!({ "code": code })
    })
    .await;
    assert!(text.contains("of 466"), "{text}");
    assert!(text.contains("read×466"), "{text}");
    assert!(text.contains("· reads session store"), "{text}");
    assert!(litecode::session::transcript_file::ledger_reads_sessions(&text));
    let ledger = text.split_once("calls (").expect("ledger").1;
    let ledger = ledger.split_once("\n\n").map(|(head, _)| head).unwrap_or(ledger);
    assert!(ledger.len() <= 8 * 1024, "{}", ledger.len());
}

#[tokio::test(flavor = "current_thread")]
async fn partial_edit_is_ok_fraction_and_a_diagnostic() {
    let text = captured_run(
        json!({
            "code": r#"
                const edits = [];
                for (let i = 0; i < 9; i++) {
                    edits.push({ old_string: "line-" + i, new_string: "LINE-" + i });
                }
                edits.push({ old_string: "missing-line", new_string: "NOPE" });
                await tools.edit({ path: "many.txt", edits });
                return "done";
            "#
        }),
        |root| {
            let body: String = (0..10).map(|index| format!("line-{index}\n")).collect();
            std::fs::write(root.join("many.txt"), body).unwrap();
        },
    )
    .await;
    assert!(text.contains("ok(9/10)"), "{text}");
    assert!(text.contains("edit_block_failed"), "{text}");
}
