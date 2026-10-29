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
    // Token budget (10k) keeps both ends; the first line names the token cut.
    let result = text.split_once("result:\n").expect("result section").1;
    assert!(
        result
            .lines()
            .next()
            .unwrap_or("")
            .contains("exceeds 10.0k; truncated by token budget"),
        "{text}"
    );
    assert!(result.contains("END_SENTINEL"), "{text}");
    // The 10k-token cut can still be byte-heavy (here it is), so the shared
    // 64KB guard may add a second, explicitly byte-side marker.
    assert!(
        result.contains("tok omitted from the middle") || result.contains("truncated by byte cap"),
        "{text}"
    );
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
        // WP-210 G: not_found is a domain failure and resolves { ok: false }.
        const r = await tools.read({ path: "missing.txt" });
        return [r.ok, r.reason, "read", typeof r.message];
        "#,
    )
    .await;
    assert!(text.contains("false"), "{text}");
    assert!(text.contains("\"not_found\""), "{text}");
    assert!(text.contains("\"read\""), "{text}");
    assert!(text.contains("\"string\""), "{text}");
    assert!(
        text.contains("tool calls: 0 ok / 1 failed"),
        "{text}"
    );
    assert!(!text.contains("rejected"), "{text}");
    assert!(!text.contains("#1 "), "{text}");
    assert!(!text.contains("calls ("), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn write_conflict_resolves_ok_false() {
    // WP-211: create_only conflict is a domain soft-fail, not an uncaught ToolError.
    let text = captured_run(
        json!({ "code": r#"
            const r = await tools.write({ path: "taken.txt", content: "x", create_only: true });
            return [r.ok, r.reason, typeof r.message];
        "# }),
        |root| std::fs::write(root.join("taken.txt"), "old").unwrap(),
    )
    .await;
    assert!(text.contains("false"), "{text}");
    assert!(text.contains("\"conflict\""), "{text}");
    assert!(text.contains("tool calls: 0 ok / 1 failed"), "{text}");
    assert!(!text.contains("uncaught ToolError"), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn uncaught_tool_error_names_the_call() {
    // WP-211: conflict/stale soft-fail now; keep throw-naming coverage on a
    // system code that still rejects (read of an image → unsupported).
    let text = captured_run(
        json!({ "code": r#"await tools.read({ path: "pic.png" });"# }),
        |root| {
            // Minimal PNG signature so read sniffs image/png and rejects.
            let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
            png.extend_from_slice(&[0u8; 16]);
            std::fs::write(root.join("pic.png"), png).unwrap();
        },
    )
    .await;
    assert!(
        text.contains("uncaught ToolError unsupported from call #1 read path=pic.png"),
        "{text}"
    );
    assert!(text.contains("unsupported file:"), "{text}");
    assert!(text.contains("(image/png"), "{text}");
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

