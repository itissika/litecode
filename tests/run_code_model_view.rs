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
    let text = captured_run_code(r#"inspect("START_SENTINEL" + "A".repeat(200000) + "END_SENTINEL");"#).await;
    assert!(text.len() <= 20_480, "{}", text.len());
    assert!(!text.starts_with("[blob:"), "{text}");
    assert!(text.starts_with("run_code ok"), "{text}");
    assert!(text.contains("START_SENTINEL"), "{text}");
    assert!(text.contains("END_SENTINEL"), "{text}");
    assert!(
        text.contains("full output: .litecode/run/run_code_"),
        "{text}"
    );
}

/// A caught failure is counted in the head and stated once above the stream:
/// the call the model would otherwise never hear about.
#[tokio::test(flavor = "current_thread")]
async fn caught_tool_error_is_counted_and_stated_once() {
    let text = captured_run_code(
        r#"
        // WP-210 G: not_found is a domain failure and resolves { ok: false }.
        const r = await tools.read({ path: "missing.txt" });
        inspect([r.ok, r.reason, "read", typeof r.message]);
        "#,
    )
    .await;
    assert!(text.contains("false"), "{text}");
    assert!(text.contains("\"not_found\""), "{text}");
    assert!(text.contains("\"read\""), "{text}");
    assert!(text.contains("\"string\""), "{text}");
    assert!(text.starts_with("run_code ok"), "the run itself succeeded: {text}");
    assert!(!text.contains("rejected"), "{text}");
    assert_eq!(text.matches("#1 ").count(), 1, "{text}");
    let lines: Vec<&str> = text.lines().collect();
    let anchor = lines
        .iter()
        .position(|line| line.starts_with("↳ #1 tools.read: failed(not_found)"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        anchor < lines.len() - 1,
        "the line sits above the returned value: {text}"
    );
    assert_eq!(lines.last().copied(), Some("[false,\"not_found\",\"read\",\"string\"]"), "{text}");
    assert!(!text.contains("calls ("), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn write_conflict_resolves_ok_false() {
    // WP-211: create_only conflict is a domain soft-fail, not an uncaught ToolError.
    let text = captured_run(
        json!({ "code": r#"
            const r = await tools.write({ path: "taken.txt", content: "x", create_only: true });
            inspect([r.ok, r.reason, typeof r.message]);
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
        text.contains("uncaught ToolError unsupported from call #1 tools.read path=pic.png"),
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
                  inspect("resolved");
                } catch (e) {
                  inspect(e.name);
                }
            "#
        }),
        |root| {
            std::fs::write(root.join("a.txt"), "hi\n").unwrap();
        },
    )
    .await;
    assert_eq!(text.lines().last(), Some("TypeError"), "{text}");
    assert!(text.starts_with("run_code ok"), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn run_code_description_only_lists_bound_tools() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = RecordingProvider::new(vec![vec![assistant_text_item("done", "msg_done")]]);
    let captured_handle = provider.clone();
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "workspace", 3),
        Arc::new(provider),
    );
    runtime.run("what tools are available?").await.expect("turn");

    let captured = captured_handle.captured();
    let run_code = captured[0]
        .tools
        .iter()
        .find(|tool| tool["name"] == "run_code")
        .expect("run_code is advertised");
    let description = run_code["description"].as_str().expect("description");
    assert!(
        description.contains("read(args: ReadInput)"),
        "the bound read tool must be in the SDK: {description}"
    );
    assert!(
        !description.contains("write(args:"),
        "an unbound tool leaked into the SDK: {description}"
    );
    assert!(
        !description.contains("GrepInput"),
        "an unbound type leaked into the SDK: {description}"
    );
}
