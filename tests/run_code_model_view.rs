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
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = RecordingProvider::new(vec![
        vec![function_call_item(
            "call_view",
            "run_code",
            &json!({ "code": code }).to_string(),
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
