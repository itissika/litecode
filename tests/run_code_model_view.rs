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
                const r = await tools.edit({ path: "many.txt", edits });
                return { failed: r.failed, has_status: Object.prototype.hasOwnProperty.call(r, "status") };
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
    assert!(text.contains("\"failed\": 1"), "{text}");
    assert!(text.contains("\"has_status\": false"), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn caught_tool_error_is_counted_and_not_listed() {
    let text = captured_run_code(
        r#"
        try {
          await tools.read({ path: "missing.txt" });
        } catch (e) {
          return [e instanceof ToolError, e.code, e.tool, typeof e.seq];
        }
        "#,
    )
    .await;
    assert!(text.contains("true"), "{text}");
    assert!(text.contains("\"not_found\""), "{text}");
    assert!(text.contains("\"read\""), "{text}");
    assert!(text.contains("\"number\""), "{text}");
    assert!(
        text.contains("1 rejected (not_found×1), handled by the program"),
        "{text}"
    );
    assert!(!text.contains("#1 "), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn uncaught_tool_error_names_the_call() {
    let text = captured_run_code(r#"await tools.read({ path: "missing.txt" });"#).await;
    assert!(
        text.contains("uncaught ToolError not_found from call #1 read path=missing.txt"),
        "{text}"
    );
    assert!(text.contains("not_found:"), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn a_second_argument_is_a_catchable_type_error() {
    let text = captured_run(
        json!({
            "code": r#"
                try {
                  await tools.read({ path: "a.txt" }, {});
                  return "resolved";
                } catch (e) {
                  return e.name;
                }
            "#
        }),
        |root| {
            std::fs::write(root.join("a.txt"), "hi\n").unwrap();
        },
    )
    .await;
    assert!(text.contains("result:\nTypeError") || text.contains("\"TypeError\""), "{text}");
    assert!(text.starts_with("run_code ok"), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn start_line_zero_is_invalid_input() {
    let text = captured_run(
        json!({
            "code": r#"
                try {
                  await tools.read({ path: "a.txt", start_line: 0 });
                  return "resolved";
                } catch (e) {
                  return [e instanceof ToolError, e.code, e.message];
                }
            "#
        }),
        |root| {
            std::fs::write(root.join("a.txt"), "alpha\n").unwrap();
        },
    )
    .await;
    assert!(text.contains("\"invalid_input\""), "{text}");
    assert!(text.contains("start_line"), "{text}");
    assert!(text.contains("error:invalid_input"), "{text}");
    assert!(!text.contains("resolved"), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn file_path_alias_is_rejected_without_reading() {
    let text = captured_run(
        json!({
            "code": r#"
                try {
                  const page = await tools.read({ path: "a.txt", file_path: "a.txt" });
                  return JSON.stringify(page);
                } catch (e) {
                  return [e instanceof ToolError, e.code, e.message];
                }
            "#
        }),
        |root| {
            std::fs::write(root.join("a.txt"), "SENTINEL_NOT_READ\n").unwrap();
        },
    )
    .await;
    assert!(text.contains("\"invalid_input\""), "{text}");
    assert!(text.contains("file_path"), "{text}");
    assert!(text.contains("error:invalid_input"), "{text}");
    assert!(!text.contains("SENTINEL_NOT_READ"), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn parent_glob_and_blank_grep_glob_are_invalid_input() {
    let text = captured_run_code(
        r#"
        let glob_error = null;
        let grep_error = null;
        try {
          await tools.glob({ pattern: "../*" });
        } catch (e) {
          glob_error = [e instanceof ToolError, e.code, e.message];
        }
        try {
          await tools.grep({ pattern: "x", glob: "  " });
        } catch (e) {
          grep_error = [e instanceof ToolError, e.code, e.message];
        }
        return { glob_error, grep_error };
        "#,
    )
    .await;
    assert!(text.contains("invalid_input"), "{text}");
    assert!(text.contains("reaches outside the search directory"), "{text}");
    assert!(text.contains("parameter 'glob' must not be blank"), "{text}");
    assert!(text.contains("error:invalid_input"), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn missing_file_message_has_no_discovery_hint() {
    let text = captured_run_code(r#"await tools.read({ path: "nope.rs" });"#).await;
    assert!(text.contains("file not found:"), "{text}");
    assert!(!text.contains("Similar files"), "{text}");
    assert!(!text.contains("verify"), "{text}");
}
