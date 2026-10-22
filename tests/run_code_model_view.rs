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
                "apply_edits".into(),
                "write".into(),
            ],
            "workspace",
            6,
        ),
        Arc::new(provider),
    );
    runtime.run("show the run").await.expect("turn completes");
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
        text.contains("0 succeeded, 1 failed"),
        "{text}"
    );
    assert!(!text.contains("rejected"), "{text}");
    assert!(!text.contains("#1 "), "{text}");
    assert!(!text.contains("calls ("), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn uncaught_tool_error_names_the_call() {
    let text = captured_run_code(r#"await tools.read({ path: "missing.txt" });"#).await;
    assert!(
        text.contains("uncaught ToolError not_found from call #1 read path=missing.txt"),
        "{text}"
    );
    assert!(!text.contains("calls ("), "{text}");
    assert!(!text.contains("diagnostics:"), "{text}");
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
    assert!(
        text.contains("result:\nTypeError") || text.contains("\"TypeError\""),
        "{text}"
    );
    assert!(text.starts_with("run_code ok"), "{text}");
}

