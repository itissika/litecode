use std::fmt::Write as _;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::{
    CallState, ExecuteRequest, FailureKind, HostBinding, HostError, HostFn, Limits, Namespace,
    Outcome, ResourceKind, execute,
};

fn limits() -> Limits {
    Limits {
        memory_bytes: 32 * 1024 * 1024,
        stack_bytes: 256 * 1024,
        output_bytes: 64 * 1024,
        max_in_flight: 8,
    }
}

fn tools(functions: Vec<(&str, HostFn)>) -> Vec<Namespace> {
    vec![Namespace {
        name: "tools".into(),
        functions: functions
            .into_iter()
            .map(|(name, function)| HostBinding {
                name: name.into(),
                function,
                contract: None,
            })
            .collect(),
    }]
}

/// Bindings whose result is read through this contract. The engine does not
/// own field lists: in production they come from the tool's output schema.
fn tools_with_contract(
    functions: Vec<(&str, HostFn)>,
    fields: &[&str],
) -> Vec<Namespace> {
    let contract = Arc::new(super::BindingContract {
        fields: fields
            .iter()
            .map(|field| (*field).to_string())
            .collect::<Vec<_>>()
            .into(),
        failure_fields: ["ok", "reason", "message"]
            .iter()
            .map(|field| (*field).to_string())
            .collect::<Vec<_>>()
            .into(),
    });
    vec![Namespace {
        name: "tools".into(),
        functions: functions
            .into_iter()
            .map(|(name, function)| HostBinding {
                name: name.into(),
                function,
                contract: Some(Arc::clone(&contract)),
            })
            .collect(),
    }]
}

async fn run_with(
    source: &str,
    namespaces: Vec<Namespace>,
    limits: Limits,
    budget: Duration,
    grace: Duration,
    cancel: CancellationToken,
) -> Outcome {
    execute(ExecuteRequest {
        source: source.into(),
        namespaces,
        limits,
        budget,
        grace,
        cancel,
        fs_root: None,
    })
    .await
}

async fn run(source: &str, namespaces: Vec<Namespace>) -> Outcome {
    run_with(
        source,
        namespaces,
        limits(),
        Duration::from_secs(2),
        Duration::from_millis(200),
        CancellationToken::new(),
    )
    .await
}

fn value_of(outcome: &Outcome) -> &Value {
    assert!(
        outcome.failure.is_none(),
        "expected success, got {outcome:?}"
    );
    outcome
        .value()
        .as_ref()
        .expect("success carries a value")
        .as_value()
}

/// `fs` has no reminder channel, so `truncated` is the only way the model
/// learns that entries were left out. The program must see it on the value.
#[tokio::test]
async fn fs_ls_exposes_truncation_as_a_field() {
    let dir = tempfile::tempdir().unwrap();
    for index in 0..(crate::ptc::fs::LS_CAP + 1) {
        std::fs::write(dir.path().join(format!("f{index:04}.txt")), "x").unwrap();
    }
    let outcome = execute(ExecuteRequest {
        source: "return fs.ls(\".\");".into(),
        namespaces: Vec::new(),
        limits: limits(),
        budget: Duration::from_secs(5),
        grace: Duration::from_millis(200),
        cancel: CancellationToken::new(),
        fs_root: Some(dir.path().to_path_buf()),
    })
    .await;
    let value = value_of(&outcome);
    assert_eq!(value["truncated"].as_bool(), Some(true), "{value}");
    assert_eq!(value["entries"].as_array().unwrap().len(), crate::ptc::fs::LS_CAP);
    assert!(value.get("message").is_none(), "the prose field is gone: {value}");
}
/// Console lines of the run, in order.
fn texts(outcome: &Outcome) -> Vec<String> {
    outcome
        .entries
        .iter()
        .filter_map(|entry| match &entry.body {
            crate::ptc::EntryBody::Text { text, .. } => Some(text.clone()),
            crate::ptc::EntryBody::Value(_) => None,
        })
        .collect()
}

fn failure_kind(outcome: &Outcome) -> FailureKind {
    outcome
        .failure
        .as_ref()
        .unwrap_or_else(|| panic!("expected a failure, got {outcome:?}"))
        .kind
}

#[tokio::test]
async fn dotted_binding_installs_a_nested_namespace() {
    let echo = HostFn::new(|_, _, _, _| async move { Ok(json!({ "nested": true })) });
    let outcome = run(
        "return await tools.deck.only({ n: 1 });",
        tools(vec![("deck.only", echo)]),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!({ "nested": true }));
}

#[tokio::test]
async fn budget_interrupts_infinite_loop() {
    let started = Instant::now();
    let outcome = run_with(
        "while (true) {}",
        Vec::new(),
        limits(),
        Duration::from_millis(80),
        Duration::from_millis(30),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(failure_kind(&outcome), FailureKind::Timeout);
    assert!(outcome.value().is_none());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn cancel_while_js_running_cancels_inflight() {
    let started = Arc::new(Notify::new());
    let saw = Arc::new(AtomicBool::new(false));
    let started_host = Arc::clone(&started);
    let saw_host = Arc::clone(&saw);
    let watch = HostFn::new(move |_, cancel, _, _| {
        let started_host = Arc::clone(&started_host);
        let saw_host = Arc::clone(&saw_host);
        async move {
            started_host.notify_one();
            cancel.cancelled().await;
            saw_host.store(true, Ordering::SeqCst);
            Ok(json!(null))
        }
    });
    let cancel = CancellationToken::new();
    let cancel_task = cancel.clone();
    let waiter = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .expect("host did not start");
        // Let the program leave the yield and enter the loop before cancelling.
        tokio::time::sleep(Duration::from_millis(40)).await;
        cancel_task.cancel();
    });
    let outcome = tokio::time::timeout(
        Duration::from_secs(3),
        run_with(
            "const pending = tools.watch(null); await Promise.resolve(); while (true) {}",
            tools(vec![("watch", watch)]),
            limits(),
            Duration::from_secs(5),
            Duration::from_millis(400),
            cancel,
        ),
    )
    .await
    .expect("cancel while running timed out");
    waiter.await.unwrap();
    assert_eq!(failure_kind(&outcome), FailureKind::Cancelled);
    assert!(outcome.value().is_none());
    assert!(saw.load(Ordering::SeqCst));
    assert_eq!(outcome.calls.len(), 1);
    assert_eq!(outcome.calls[0].state, CallState::CancelledFinished);
    assert!(outcome.calls[0].ended.is_some());
}

#[tokio::test]
async fn cancel_while_awaiting_host() {
    let started = Arc::new(Notify::new());
    let saw = Arc::new(AtomicBool::new(false));
    let started_host = Arc::clone(&started);
    let saw_host = Arc::clone(&saw);
    let watch = HostFn::new(move |_, cancel, _, _| {
        let started_host = Arc::clone(&started_host);
        let saw_host = Arc::clone(&saw_host);
        async move {
            started_host.notify_one();
            cancel.cancelled().await;
            saw_host.store(true, Ordering::SeqCst);
            Ok(json!({"continued": true}))
        }
    });
    let cancel = CancellationToken::new();
    let cancel_task = cancel.clone();
    let waiter = tokio::spawn(async move {
        started.notified().await;
        cancel_task.cancel();
    });
    let outcome = run_with(
        "return await tools.watch(null);",
        tools(vec![("watch", watch)]),
        limits(),
        Duration::from_secs(5),
        Duration::from_millis(400),
        cancel,
    )
    .await;
    waiter.await.unwrap();
    assert_eq!(failure_kind(&outcome), FailureKind::Cancelled);
    assert!(outcome.value().is_none());
    assert!(saw.load(Ordering::SeqCst));
    assert_eq!(outcome.calls[0].state, CallState::CancelledFinished);
}

#[tokio::test]
async fn cancel_grace_records_unfinished() {
    let started = Arc::new(Notify::new());
    let token_slot: Arc<Mutex<Option<CancellationToken>>> = Arc::new(Mutex::new(None));
    let started_host = Arc::clone(&started);
    let token_host = Arc::clone(&token_slot);
    let hang = HostFn::new(move |_, cancel, _, _| {
        let started_host = Arc::clone(&started_host);
        let token_host = Arc::clone(&token_host);
        async move {
            *token_host.lock().unwrap() = Some(cancel.clone());
            started_host.notify_one();
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(json!(null))
        }
    });
    let cancel = CancellationToken::new();
    let cancel_task = cancel.clone();
    tokio::spawn(async move {
        started.notified().await;
        cancel_task.cancel();
    });
    let started_at = Instant::now();
    let outcome = run_with(
        "return await tools.hang(null);",
        tools(vec![("hang", hang)]),
        limits(),
        Duration::from_secs(5),
        Duration::from_millis(60),
        cancel,
    )
    .await;
    assert!(started_at.elapsed() < Duration::from_millis(800));
    assert_eq!(failure_kind(&outcome), FailureKind::Cancelled);
    assert_eq!(outcome.calls[0].state, CallState::CancelledUnfinished);
    assert!(outcome.calls[0].ended.is_none());
    let stored = token_slot.lock().unwrap().clone().unwrap();
    assert!(stored.is_cancelled());
}

#[tokio::test]
async fn budget_pause_is_not_charged() {
    let pause_host = HostFn::new(|_, _, pause, _| async move {
        let _guard = pause.pause();
        tokio::time::sleep(Duration::from_millis(300)).await;
        Ok(json!(7))
    });
    let outcome = run_with(
        "return await tools.wait(null);",
        tools(vec![("wait", pause_host)]),
        limits(),
        Duration::from_millis(80),
        Duration::from_millis(40),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!(7));
    assert_eq!(outcome.calls[0].state, CallState::Completed);

    let plain = HostFn::new(|_, cancel, _, _| async move {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(300)) => Ok(json!(1)),
            _ = cancel.cancelled() => Ok(json!(0)),
        }
    });
    let outcome = run_with(
        "return await tools.wait(null);",
        tools(vec![("wait", plain)]),
        limits(),
        Duration::from_millis(80),
        Duration::from_millis(200),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(failure_kind(&outcome), FailureKind::Timeout);
    assert_eq!(outcome.calls[0].state, CallState::CancelledFinished);
}

#[tokio::test]
async fn memory_limit() {
    let mut limits = limits();
    limits.memory_bytes = 3 * 1024 * 1024;
    let outcome = run_with(
        "const blob = new Uint8Array(16 * 1024 * 1024); return blob[0];",
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    let failure = outcome.failure.expect("memory limit");
    assert_eq!(failure.kind, FailureKind::ResourceLimit);
    assert_eq!(failure.resource, Some(ResourceKind::Memory));
    assert!(
        failure.message.contains("out of memory"),
        "{}",
        failure.message
    );
}

#[tokio::test]
async fn memory_exhaustion_stays_sealed_when_the_program_catches_it() {
    let mut limits = limits();
    limits.memory_bytes = 3 * 1024 * 1024;
    let outcome = run_with(
        r#"
        try {
          const blob = new Uint8Array(16 * 1024 * 1024);
          return "caught " + blob[0];
        } catch (e) {
          return "caught";
        }
        "#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    assert!(
        outcome.value().is_none(),
        "program returned {:?}",
        outcome.value()
    );
    let failure = outcome.failure.expect("out of memory is not catchable");
    assert_eq!(failure.kind, FailureKind::ResourceLimit, "{failure:?}");
    assert_eq!(failure.resource, Some(ResourceKind::Memory), "{failure:?}");
}

#[tokio::test]
async fn caught_allocation_failure_is_memory_when_the_stack_watch_is_disarmed() {
    let mut limits = limits();
    limits.memory_bytes = 3 * 1024 * 1024;
    limits.stack_bytes = 0;
    let outcome = run_with(
        r#"
        try {
          const blob = new Uint8Array(16 * 1024 * 1024);
          return "caught " + blob[0];
        } catch (e) {
          return "caught";
        }
        "#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    assert!(
        outcome.value().is_none(),
        "program returned {:?}",
        outcome.value()
    );
    let failure = outcome.failure.expect("memory fault overrides success");
    assert_eq!(failure.kind, FailureKind::ResourceLimit, "{failure:?}");
    assert_eq!(failure.resource, Some(ResourceKind::Memory), "{failure:?}");
}

#[tokio::test]
async fn stack_limit() {
    let mut limits = limits();
    limits.stack_bytes = 64 * 1024;
    let outcome = run_with(
        "function f(){ return 1 + f(); } return f();",
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    let failure = outcome.failure.expect("stack limit");
    assert_eq!(failure.kind, FailureKind::ResourceLimit, "{failure:?}");
    assert_eq!(failure.resource, Some(ResourceKind::Stack));
    assert!(
        failure.message.contains("stack overflow")
            || failure.message.contains("Maximum call stack size exceeded"),
        "{}",
        failure.message
    );
}

#[tokio::test]
async fn stack_overflow_caught_then_immediate_return_is_still_sealed() {
    let mut limits = limits();
    limits.stack_bytes = 64 * 1024;
    let outcome = run_with(
        r#"
        function f(){ return 1 + f(); }
        try { f(); } catch (e) { return "caught"; }
        "#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    assert!(
        outcome.value().is_none(),
        "program returned {:?}",
        outcome.value()
    );
    let failure = outcome.failure.expect("stack overflow is not catchable");
    assert_eq!(failure.kind, FailureKind::ResourceLimit, "{failure:?}");
    assert_eq!(failure.resource, Some(ResourceKind::Stack), "{failure:?}");
}

#[tokio::test]
async fn stack_overflow_stays_sealed_when_the_program_catches_it() {
    let mut limits = limits();
    limits.stack_bytes = 64 * 1024;
    let outcome = run_with(
        r#"
        try {
          function f(){ return 1 + f(); }
          return f();
        } catch (e) {
          return "caught";
        }
        "#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    assert!(
        outcome.value().is_none(),
        "program returned {:?}",
        outcome.value()
    );
    let failure = outcome.failure.expect("stack overflow is not catchable");
    assert_eq!(failure.kind, FailureKind::ResourceLimit, "{failure:?}");
    assert_eq!(failure.resource, Some(ResourceKind::Stack), "{failure:?}");
}

#[tokio::test]
async fn success_survives_budget_expiry_at_return() {
    // The interrupt aborts JavaScript once the clock is exhausted, so a live
    // program cannot return after the deadline. The expiry-after-return policy
    // is locked in `engine::settle_tests`. This run checks that a normal return
    // is still a success.
    let outcome = run("return \"kept\";", Vec::new()).await;
    assert_eq!(value_of(&outcome), &json!("kept"), "{outcome:?}");
    assert!(outcome.failure.is_none(), "{outcome:?}");
}

#[tokio::test]
async fn output_limit_keeps_log_prefix() {
    let mut limits = limits();
    limits.output_bytes = 12;
    let outcome = run_with(
        r#"
        console.log("abcdefghij");
        try { console.log("XYZ"); return "caught"; }
        catch (e) { return "caught"; }
        "#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(failure_kind(&outcome), FailureKind::OutputLimit);
    assert!(outcome.value().is_none());
    assert_eq!(texts(&outcome), vec!["abcdefghij", "XY"]);
}

#[tokio::test]
async fn output_limit_counts_final_value() {
    let mut limits = limits();
    limits.output_bytes = 8;
    let outcome = run_with(
        r#"console.log("ab"); return "hello";"#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(failure_kind(&outcome), FailureKind::OutputLimit);
    assert_eq!(texts(&outcome), vec!["ab"]);
    assert!(outcome.value().is_none());

    limits.output_bytes = 3;
    let outcome = run_with(
        r#"console.log("ab"); return 1;"#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!(1));
    assert_eq!(texts(&outcome), vec!["ab"]);
}

#[tokio::test]
async fn illegal_output_values() {
    let cases = [
        ("return 1n;", "bigint at $"),
        ("return { a: { b: 1n } };", "bigint at $.a.b"),
        ("return [1, Symbol('s')];", "symbol at $[1]"),
        ("return { f() {} };", "function at $.f"),
        ("return function () {};", "function"),
        ("return Symbol('s');", "symbol"),
        ("const a = {}; a.self = a; return a;", "circular reference"),
        ("return new Date();", "Date"),
        ("return [1,,2];", "sparse array"),
        (
            "const marked = { a: 1 }; marked[Symbol('s')] = 2; return marked;",
            "symbol key",
        ),
    ];
    for (source, needle) in cases {
        let outcome = run(source, Vec::new()).await;
        let failure = outcome
            .failure
            .as_ref()
            .unwrap_or_else(|| panic!("{source} succeeded: {outcome:?}"));
        assert_eq!(failure.kind, FailureKind::IllegalOutput, "{source}");
        assert!(
            failure.message.contains(needle),
            "{source}: {}",
            failure.message
        );
        assert!(outcome.value().is_none());
    }
}

#[tokio::test]
async fn non_finite_output_is_nulled_with_paths() {
    let outcome = run(
        "return { n: NaN, nested: [Infinity, -Infinity] };",
        Vec::new(),
    )
    .await;
    assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
    assert_eq!(
        value_of(&outcome),
        &json!({"n": null, "nested": [null, null]})
    );
    let line = outcome
        .dropped_undefined
        .non_finite_head_line()
        .expect("coercion warning");
    assert!(line.contains("n"), "{line}");
    assert!(line.contains("nested[0]"), "{line}");
    assert!(line.contains("nested[1]"), "{line}");
    assert!(outcome.dropped_undefined.head_line().is_none());
}

#[tokio::test]
async fn unknown_tool_is_a_contract_failure() {
    // B3: naming a tool that does not exist is a contract failure, not a
    // catchable TypeError with a bare "not a function".
    let outcome = run("return await tools.nope({});", tools(vec![])).await;
    let failure = outcome.failure.as_ref().expect("unknown tool must fail");
    assert_eq!(failure.kind, FailureKind::Exception);
    assert!(!failure.uncaught_type_error, "{failure:?}");
    let info = failure
        .tool_error
        .as_ref()
        .expect("a contract failure keeps its ToolError identity");
    assert_eq!(info.code, "not_found");
    assert!(
        failure.message.contains("tools.nope"),
        "the message names the call: {failure:?}"
    );
}

#[tokio::test]
async fn illegal_arguments_skip_host() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_host = Arc::clone(&calls);
    let host = HostFn::new(move |_, _, _, _| {
        calls_host.fetch_add(1, Ordering::SeqCst);
        async move { Ok(json!(null)) }
    });
    let cases = [
        ("await tools.f(NaN);", "non-finite number"),
        ("await tools.f(Infinity);", "non-finite number"),
        ("await tools.f(1n);", "bigint at $"),
        ("await tools.f({ n: [1n] });", "bigint at $.n[0]"),
        ("await tools.f(function () {});", "function"),
        ("await tools.f(Symbol('s'));", "symbol"),
        (
            "const a = {}; a.self = a; await tools.f(a);",
            "circular reference",
        ),
        ("await tools.f(new Date());", "Date"),
        ("await tools.f([1,,2]);", "sparse array"),
        (
            "const marked = { a: 1 }; marked[Symbol('s')] = 2; await tools.f(marked);",
            "symbol key",
        ),
    ];
    for (body, needle) in cases {
        calls.store(0, Ordering::SeqCst);
        let source =
            format!("try {{ {body} return 'called'; }} catch (e) {{ return String(e.message); }}");
        let outcome = run(&source, tools(vec![("f", host.clone())])).await;
        let message = value_of(&outcome).as_str().unwrap_or("");
        assert!(message.contains(needle), "{body}: {message}");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "{body}");
        assert!(outcome.calls.is_empty(), "{body}");
    }
}

#[tokio::test]
async fn host_integer_above_safe_range_is_rejected() {
    let cases = [
        json!(9_007_199_254_740_992_i64),
        json!(-9_007_199_254_740_992_i64),
        json!(1_u64 << 60),
    ];
    for value in cases {
        let host_value = value.clone();
        let host = HostFn::new(move |_, _, _, _| {
            let host_value = host_value.clone();
            async move { Ok(host_value) }
        });
        let outcome = run(
            r#"
            try { return { ok: true, value: await tools.n(null) }; }
            catch (e) { return { ok: false, message: String(e.message) }; }
            "#,
            tools(vec![("n", host)]),
        )
        .await;
        let body = value_of(&outcome);
        assert_eq!(body["ok"], json!(false), "{value}");
        assert!(
            body["message"].as_str().unwrap().contains("2^53-1"),
            "{value}: {body}"
        );
        assert_eq!(outcome.calls[0].state, CallState::Completed, "{value}");
    }

    let fraction = HostFn::new(|_, _, _, _| async move { Ok(json!(1.25)) });
    let outcome = run("return await tools.n(null);", tools(vec![("n", fraction)])).await;
    assert_eq!(value_of(&outcome), &json!(1.25));
}

#[tokio::test]
async fn no_ambient_capabilities() {
    let outcome = run(
        r#"
        const names = ["fetch", "require", "process", "setTimeout", "std", "os", "Atomics", "SharedArrayBuffer"];
        const present = names.filter((name) => typeof globalThis[name] !== "undefined");
        let imported = false;
        try { await import("fs"); imported = true; } catch (e) { imported = false; }
        return {
          present,
          imported,
          evalOk: eval("1+1") === 2,
          fnOk: (new Function("return 3"))() === 3
        };
        "#,
        Vec::new(),
    )
    .await;
    assert_eq!(
        value_of(&outcome),
        &json!({"present": [], "imported": false, "evalOk": true, "fnOk": true})
    );
}

#[tokio::test]
async fn unhandled_promise_rejection_warns_without_failing() {
    let outcome = run(
        r#"
        Promise.reject("boom-unhandled");
        return "ok";
        "#,
        tools(vec![]),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!("ok"));
    assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
    assert!(
        texts(&outcome)
            .iter()
            .any(|text| text.contains("unhandled promise rejection")
                && text.contains("boom-unhandled")),
        "{:?}",
        texts(&outcome)
    );
}

#[tokio::test]
async fn handled_promise_rejection_does_not_warn() {
    let outcome = run(
        r#"
        Promise.reject("caught").catch(() => {});
        return "ok";
        "#,
        tools(vec![]),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!("ok"));
    assert!(
        texts(&outcome)
            .iter()
            .all(|text| !text.contains("unhandled promise rejection")),
        "{:?}",
        texts(&outcome)
    );
}

#[tokio::test]
async fn unawaited_calls_are_waited_and_recorded() {
    let finished = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&finished);
    let echo = HostFn::new(move |args, _, _, _| {
        let flag = Arc::clone(&flag);
        async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            flag.store(true, Ordering::SeqCst);
            Ok(args.into_value())
        }
    });
    let outcome = run(
        "tools.echo({ n: 1 }); return 'done';",
        tools(vec![("echo", echo)]),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!("done"));
    assert!(finished.load(Ordering::SeqCst));
    assert_eq!(outcome.calls.len(), 1);
    assert_eq!(outcome.calls[0].name, "echo");
    assert_eq!(outcome.calls[0].state, CallState::CancelledFinished);
    assert!(outcome.calls[0].ended.is_some());
    assert!(
        texts(&outcome)
            .iter()
            .any(|text| text.contains("1 host call was still running")),
        "{:?}",
        texts(&outcome)
    );
}

#[tokio::test]
async fn tool_error_is_catchable_and_instanceof() {
    let host = HostFn::new(|_, _, _, _| async move {
        Err(HostError::tool("not_found", "missing", None, false))
    });
    let outcome = run(
        r#"
        try {
          await tools.read(null);
          return "resolved";
        } catch (e) {
          return [e instanceof ToolError, e.name, e.code, e.tool, typeof e.seq, e.message];
        }
        "#,
        tools(vec![("read", host)]),
    )
    .await;
    assert_eq!(
        value_of(&outcome),
        &json!([true, "ToolError", "not_found", "read", "number", "missing"])
    );
}

#[tokio::test]
async fn uncaught_tool_error_keeps_code() {
    let host = HostFn::new(|_, _, _, _| async move {
        Err(HostError::tool("not_found", "missing", None, false))
    });
    let outcome = run("await tools.read(null);", tools(vec![("read", host)])).await;
    let failure = outcome.failure.expect("uncaught tool error");
    let info = failure.tool_error.expect("tool error identity");
    assert_eq!(info.code, "not_found");
    assert_eq!(info.tool, "read");
    assert_eq!(info.seq, 1);
    assert_eq!(failure.message, "missing");
}

fn nested_arrays(depth: usize) -> Value {
    let mut value = json!(0);
    for _ in 0..depth {
        value = Value::Array(vec![value]);
    }
    value
}

fn array_depth(value: &Value) -> usize {
    let mut depth = 0;
    let mut cursor = value;
    while let Some(items) = cursor.as_array() {
        let Some(child) = items.first() else {
            break;
        };
        if items.len() != 1 {
            break;
        }
        depth += 1;
        cursor = child;
    }
    depth
}

fn failure_text(outcome: &Outcome) -> String {
    match &outcome.failure {
        Some(failure) => format!("{:?} {}", failure.kind, failure.message),
        None => "none".into(),
    }
}

/// Run `test` in a child process so a stack overflow fails the assertion
/// instead of killing the test runner. Returns true when this process is the
/// child and should execute the body.
fn isolated(test: &str) -> bool {
    if std::env::var_os("PTC_ISOLATED_CHILD").is_some() {
        return true;
    }
    let output = Command::new(std::env::current_exe().expect("test executable"))
        .arg(test)
        .arg("--exact")
        .env("PTC_ISOLATED_CHILD", "1")
        .output()
        .expect("spawn isolated child");
    assert!(
        output.status.success(),
        "child status {:?}\nstdout {}\nstderr {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

#[tokio::test]
async fn success_cancels_inflight_calls() {
    let slow = HostFn::new(|_, _, _, _| async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok(json!(null))
    });
    let started = Instant::now();
    let outcome = run_with(
        "tools.slow(null); return 'ok';",
        tools(vec![("slow", slow)]),
        limits(),
        Duration::from_millis(80),
        Duration::from_millis(40),
        CancellationToken::new(),
    )
    .await;
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "slow call turned success into a wait: {:?}",
        started.elapsed()
    );
    assert_eq!(value_of(&outcome), &json!("ok"));
    assert_ne!(failure_kind_if_any(&outcome), Some(FailureKind::Timeout));
    assert_eq!(outcome.calls.len(), 1);
    assert_eq!(outcome.calls[0].state, CallState::CancelledUnfinished);
    assert!(
        texts(&outcome)
            .iter()
            .any(|text| text.contains("1 host call was still running")),
        "{:?}",
        texts(&outcome)
    );

    let paused = HostFn::new(|_, _, pause, _| async move {
        let _guard = pause.pause();
        tokio::time::sleep(Duration::from_secs(2)).await;
        Ok(json!(null))
    });
    let started = Instant::now();
    let outcome = run_with(
        "tools.hold(null); return 1;",
        tools(vec![("hold", paused)]),
        limits(),
        Duration::from_millis(50),
        Duration::from_millis(80),
        CancellationToken::new(),
    )
    .await;
    assert!(
        started.elapsed() < Duration::from_millis(600),
        "paused call did not return: {:?}",
        started.elapsed()
    );
    assert_eq!(value_of(&outcome), &json!(1));
    assert_eq!(outcome.calls.len(), 1);
    assert_eq!(outcome.calls[0].state, CallState::CancelledUnfinished);
}

fn failure_kind_if_any(outcome: &Outcome) -> Option<FailureKind> {
    outcome.failure.as_ref().map(|failure| failure.kind)
}

#[tokio::test]
async fn user_exceptions_are_not_classified_by_message() {
    let cases = [
        r#"throw new Error("interrupted");"#,
        r#"throw "out of memory";"#,
        r#"throw "Maximum call stack size exceeded";"#,
        r#"throw new RangeError("Maximum call stack size exceeded");"#,
        r#"throw new InternalError("out of memory");"#,
    ];
    for source in cases {
        let outcome = run(source, Vec::new()).await;
        let failure = outcome
            .failure
            .as_ref()
            .unwrap_or_else(|| panic!("{source} succeeded: {outcome:?}"));
        assert_eq!(
            failure.kind,
            FailureKind::Exception,
            "{source}: {failure:?}"
        );
        assert!(outcome.value().is_none());
    }
}

#[tokio::test]
async fn dense_array_within_output_budget() {
    let mut wide = limits();
    wide.output_bytes = 1024 * 1024;
    let outcome = run_with(
        r#"
        const values = [];
        for (let i = 0; i < 40000; i++) values.push(i);
        return values;
        "#,
        Vec::new(),
        wide,
        Duration::from_secs(5),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    assert!(outcome.failure.is_none(), "{}", failure_text(&outcome));
    let values = value_of(&outcome).as_array().expect("dense array");
    assert_eq!(values.len(), 40_000);
    assert_eq!(values[0], json!(0));
    assert_eq!(values[39_999], json!(39_999));
}

#[tokio::test]
async fn sparse_huge_array_rejected_before_allocation() {
    if !isolated(concat!(
        module_path!(),
        "::sparse_huge_array_rejected_before_allocation"
    )) {
        return;
    }
    let mut wide = limits();
    wide.memory_bytes = 64 * 1024 * 1024;
    wide.output_bytes = 1024 * 1024;
    for source in [
        "const sparse = []; sparse.length = 1e8; return sparse;",
        "return new Array(1e8);",
        "const sparse = []; sparse[0] = 1; sparse[3] = 2; sparse.length = 1e8; return sparse;",
    ] {
        let started = Instant::now();
        let outcome = run_with(
            source,
            Vec::new(),
            wide,
            Duration::from_secs(2),
            Duration::from_millis(50),
            CancellationToken::new(),
        )
        .await;
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{source} took {:?}",
            started.elapsed()
        );
        let failure = outcome
            .failure
            .as_ref()
            .unwrap_or_else(|| panic!("{source} succeeded"));
        assert_eq!(
            failure.kind,
            FailureKind::IllegalOutput,
            "{source}: {failure:?}"
        );
        assert!(
            failure.message.contains("sparse array"),
            "{source}: {}",
            failure.message
        );
        assert!(
            !failure.message.contains("nesting depth"),
            "{source}: {}",
            failure.message
        );
    }
}

#[tokio::test]
async fn deep_nesting_does_not_crash() {
    if !isolated(concat!(module_path!(), "::deep_nesting_does_not_crash")) {
        return;
    }
    deep_nest_child().await;
}

fn exercise_json(value: &super::Json) {
    let cloned = value.clone();
    assert_eq!(value, &cloned);
    let mut sink = Discard;
    write!(&mut sink, "{value:?}{cloned:?}").expect("debug");
}

struct Discard;

impl std::fmt::Write for Discard {
    fn write_str(&mut self, _text: &str) -> std::fmt::Result {
        Ok(())
    }
}

fn deep_array_source() -> &'static str {
    r#"
    let value = 0;
    for (let i = 0; i < 20000; i++) value = [value];
    return value;
    "#
}

async fn deep_nest_child() {
    {
        let returned = run(deep_array_source(), Vec::new()).await;
        assert!(
            returned.failure.is_none(),
            "deep return: {}",
            failure_text(&returned)
        );
        let value = returned.value().expect("deep value");
        assert_eq!(array_depth(value.as_value()), 20_000);
        exercise_json(value);
    }

    {
        let ignored = HostFn::new(|_value, _, _, _| async move { Ok(json!(1)) });
        let ignored_argument = run(
            r#"
            let value = 0;
            for (let i = 0; i < 20000; i++) value = [value];
            return await tools.f(value);
            "#,
            tools(vec![("f", ignored)]),
        )
        .await;
        assert_eq!(value_of(&ignored_argument), &json!(1));
        assert_eq!(ignored_argument.calls[0].state, CallState::Completed);
    }

    {
        let injected = HostFn::new(|_, _, _, _| async move { Ok(nested_arrays(20_000)) });
        let host_to_js = run(
            "return await tools.deep(null);",
            tools(vec![("deep", injected)]),
        )
        .await;
        assert!(
            host_to_js.failure.is_none(),
            "deep host result: {}",
            failure_text(&host_to_js)
        );
        let value = host_to_js.value().expect("injected value");
        assert_eq!(array_depth(value.as_value()), 20_000);
        exercise_json(value);
    }

    {
        let mut tight = limits();
        tight.output_bytes = 100;
        let over_budget = run_with(
            deep_array_source(),
            Vec::new(),
            tight,
            Duration::from_secs(2),
            Duration::from_millis(50),
            CancellationToken::new(),
        )
        .await;
        let failure = over_budget.failure.as_ref().expect("output budget");
        assert_eq!(failure.kind, FailureKind::OutputLimit, "{failure:?}");
        assert!(over_budget.value().is_none());
    }

    {
        let illegal = run(
            r#"
            let value = 0;
            for (let i = 0; i < 20000; i++) value = [value];
            return [value, 1n];
            "#,
            Vec::new(),
        )
        .await;
        let failure = illegal.failure.as_ref().expect("illegal sibling");
        assert_eq!(failure.kind, FailureKind::IllegalOutput, "{failure:?}");
        assert!(
            failure.message.contains("bigint at $[1]"),
            "{}",
            failure.message
        );
        assert!(illegal.value().is_none());
    }

    let mut wide = limits();
    wide.memory_bytes = 64 * 1024 * 1024;
    wide.output_bytes = 64 * 1024;
    let started = Instant::now();
    let sparse = run_with(
        "const sparse = []; sparse.length = 1e8; return sparse;",
        Vec::new(),
        wide,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "sparse array took {:?}",
        started.elapsed()
    );
    let failure = sparse.failure.as_ref().expect("sparse array");
    assert_eq!(failure.kind, FailureKind::IllegalOutput, "{failure:?}");
    assert!(
        failure.message.contains("sparse array"),
        "{}",
        failure.message
    );
}

#[tokio::test]
async fn host_receives_extra_arguments_as_a_json_array() {
    let seen = Arc::new(Mutex::new(Value::Null));
    let record = Arc::clone(&seen);
    let echo = HostFn::new(move |args, _, _, _| {
        let value = args.as_value().clone();
        let record = Arc::clone(&record);
        async move {
            *record.lock().expect("seen") = value;
            Ok(json!(null))
        }
    });
    let outcome = run(
        "await tools.echo({ path: 'a' }, { timeout_ms: 5 }); return null;",
        tools(vec![("echo", echo)]),
    )
    .await;
    assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
    assert_eq!(
        *seen.lock().expect("seen"),
        json!([{"path": "a"}, {"timeout_ms": 5}])
    );
}

// WP-206 #7: undefined tolerance.
#[tokio::test]
async fn undefined_in_result_is_stripped_or_nulled_and_reported() {
    let cases: [(&str, Value, &[&str], usize); 5] = [
        ("return undefined;", json!(null), &["$"], 1),
        ("return { a: 1, b: undefined };", json!({"a": 1}), &["b"], 1),
        (
            "return { a: { b: undefined } };",
            json!({"a": {}}),
            &["a.b"],
            1,
        ),
        (
            "return { list: [1, undefined] };",
            json!({"list": [1, null]}),
            &["list[1]"],
            1,
        ),
        (
            "return { 'a-b': undefined };",
            json!({}),
            &[r#"["a-b"]"#],
            1,
        ),
    ];
    for (source, want, paths, total) in cases {
        let outcome = run(source, Vec::new()).await;
        assert_eq!(value_of(&outcome), &want, "{source}");
        assert_eq!(outcome.dropped_undefined.paths, paths, "{source}");
        assert_eq!(outcome.dropped_undefined.total, total, "{source}");
    }
    let outcome = run("return [undefined, undefined, 3];", Vec::new()).await;
    assert_eq!(value_of(&outcome), &json!([null, null, 3]));
    assert_eq!(
        outcome.dropped_undefined.head_line().as_deref(),
        Some("dropped undefined at [0], [1]")
    );
    let clean = run("return { a: null };", Vec::new()).await;
    assert!(clean.dropped_undefined.is_empty());
    assert!(clean.dropped_undefined.head_line().is_none());
}

#[tokio::test]
async fn dropped_undefined_list_is_capped_with_more_count() {
    let outcome = run(
        "const o = {}; for (let i = 0; i < 25; i++) o['k' + i] = undefined; o.keep = 1; return o;",
        Vec::new(),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!({"keep": 1}));
    assert_eq!(outcome.dropped_undefined.total, 25);
    assert_eq!(
        outcome.dropped_undefined.paths.len(),
        crate::ptc::DROPPED_CAP
    );
    let line = outcome.dropped_undefined.head_line().unwrap();
    assert!(line.starts_with("dropped undefined at k0, k1"), "{line}");
    assert!(line.ends_with(" +15 more"), "{line}");
}

#[tokio::test]
async fn undefined_in_arguments_is_dropped_silently() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_host = Arc::clone(&seen);
    let host = HostFn::new(move |args, _, _, _| {
        seen_host.lock().unwrap().push(args.as_value().clone());
        async move { Ok(json!("ok")) }
    });
    let outcome = run(
        "await tools.f({ a: 1, b: undefined, c: [undefined, 2] }); await tools.f(undefined); return 'done';",
        tools(vec![("f", host)]),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!("done"));
    assert!(
        outcome.dropped_undefined.is_empty(),
        "args are not reported"
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0], json!({"a": 1, "c": [null, 2]}));
    assert_eq!(seen[1], json!(null));
}

#[tokio::test]
async fn runtime_exception_reports_line_column_and_stack() {
    let outcome = run(
        "const a = 1;\nfunction boom() {\n  throw new Error('bad');\n}\nboom();",
        Vec::new(),
    )
    .await;
    let failure = outcome.failure.as_ref().expect("exception");
    assert_eq!(failure.kind, FailureKind::Exception);
    let location = failure
        .location
        .expect("runtime exceptions carry a location");
    assert_eq!((location.line, location.column), (3, 13), "{failure:?}");
    let stack = failure.stack.as_deref().unwrap_or("");
    assert!(stack.contains("program.js:"), "{stack}");
}

#[tokio::test]
async fn object_keys_keep_agent_order_across_the_bridge() {
    let echo = HostFn::new(|args, _, _, _| async move { Ok(args.into_value()) });
    let outcome = run(
        "const back = await tools.echo({zeta: 1, alpha: 2, mid: {y: 1, b: 2}});\nreturn {z: 1, a: back, m: 3};",
        tools(vec![("echo", echo)]),
    )
    .await;
    let text = serde_json::to_string(value_of(&outcome)).unwrap();
    assert_eq!(
        text,
        r#"{"z":1,"a":{"zeta":1,"alpha":2,"mid":{"y":1,"b":2}},"m":3}"#
    );
}

// ── PROBE P2–P9: contract failures and the read proxy (design v2 §3) ──
//
// Probes are split by layer, because only the production path can say what the
// contract is:
//   - judgement (S2/S3) live at the bridge, `src/tools/code/bridge_tests.rs`:
//     a real CodeSurface decides, and the run must end there;
//   - mechanism (a host `HostError::Contract` crossing try/catch and
//     Promise.all) is B2's gate and lands with B2's first change point, which
//     introduces that variant; B1 keeps product code at zero;
//   - everything else (unknown suite/member, read proxy, passthrough, benchmark)
//     is engine-level and lives here.
//
// Each probe asserts the behaviour the design requires, not the behaviour the
// code has today; "Expected red before <step>" is written on the test.

// ── MECHANISM M1–M3: is a contract error uncatchable? (B2 gate) ──
//
// These drive the engine directly with the error the bridge will hand it, so
// they measure the channel and nothing else. They must be red until the
// second cut seals the contract failure as terminal.

/// M1: a contract error cannot be swallowed by try/catch. (Expected red before
/// B2 second cut.)
#[tokio::test]
async fn m1_contract_error_is_not_catchable() {
    let host = HostFn::new(|_, _, _, _| async move {
        Err(HostError::contract(
            "invalid_args",
            "invalid input for 'read': unknown parameter 'nope'",
            "read",
        ))
    });
    let outcome = run(
        r#"
        try { await tools.read({ nope: 1 }); } catch (e) { return "swallowed"; }
        return "done";
        "#,
        tools(vec![("read", host)]),
    )
    .await;
    let failure = outcome
        .failure
        .as_ref()
        .unwrap_or_else(|| panic!("a contract error must end the run, got {outcome:?}"));
    assert!(
        failure.message.contains("unknown parameter 'nope'"),
        "{failure:?}"
    );
}

/// M2: a contract error inside Promise.all still ends the run. (Expected red
/// before B2 second cut; this is the await boundary the whole channel depends on.)
#[tokio::test]
async fn m2_contract_error_inside_promise_all_ends_run() {
    let host = HostFn::new(|_, _, _, _| async move {
        Err(HostError::contract(
            "invalid_args",
            "invalid input for 'read': unknown parameter 'nope'",
            "read",
        ))
    });
    let outcome = run(
        r#"
        try { await Promise.all([tools.read({ nope: 1 })]); } catch (e) { return "swallowed"; }
        return "done";
        "#,
        tools(vec![("read", host)]),
    )
    .await;
    assert!(
        outcome.failure.is_some(),
        "a contract error must not resolve: {outcome:?}"
    );
}

/// M3: after a contract error nothing else is dispatched. (Expected red before
/// B2 second cut.)
#[tokio::test]
async fn m3_no_host_call_after_contract_error() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let host = HostFn::new(move |_, _, _, _| {
        let counter = Arc::clone(&counter);
        async move {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(HostError::contract(
                    "invalid_args",
                    "invalid input for 'read': unknown parameter 'nope'",
                    "read",
                ))
            } else {
                Ok(json!({ "ok": true }))
            }
        }
    });
    let outcome = run(
        r#"
        try { await tools.read({ nope: 1 }); } catch (e) {}
        try { await tools.read({ path: "ok" }); } catch (e) {}
        return "finished";
        "#,
        tools(vec![("read", host)]),
    )
    .await;
    assert!(outcome.failure.is_some(), "{outcome:?}");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "no host call may be dispatched after a contract error"
    );
}

/// P2: an unknown member throws at property access, not on call.
/// (Expected red before B3.)
#[tokio::test]
async fn p2_unknown_member_throws_at_property_access() {
    let echo = HostFn::new(|_, _, _, _| async move { Ok(json!({ "ok": true })) });
    let outcome = run(
        r#"const f = tools.file.readx; return typeof f;"#,
        tools(vec![("file.write", echo)]),
    )
    .await;
    let failure = outcome
        .failure
        .as_ref()
        .unwrap_or_else(|| panic!("reading an unknown member must fail, got {outcome:?}"));
    assert!(failure.message.contains("file.readx"), "{failure:?}");
}

/// P6: an unknown suite names the available suites. (Expected red before B3.)
#[tokio::test]
async fn p6_unknown_suite_names_the_candidates() {
    let echo = HostFn::new(|_, _, _, _| async move { Ok(json!({ "ok": true })) });
    let outcome = run(
        r#"
        try { tools.zzz.one({ path: "a" }); } catch (e) { return "swallowed"; }
        return "done";
        "#,
        tools(vec![("alpha.one", echo)]),
    )
    .await;
    let failure = outcome
        .failure
        .as_ref()
        .unwrap_or_else(|| panic!("unknown suite must fail the run, got {outcome:?}"));
    assert!(failure.message.contains("zzz"), "{failure:?}");
    assert!(failure.message.contains("alpha"), "{failure:?}");
}

/// P7: an unknown member names the member and its suite's candidates.
/// (Expected red before B3.)
#[tokio::test]
async fn p7_unknown_member_names_the_candidates() {
    let echo = HostFn::new(|_, _, _, _| async move { Ok(json!({ "ok": true })) });
    let outcome = run(
        r#"
        try { await tools.file.readx({ path: "a" }); } catch (e) { return "swallowed"; }
        return "done";
        "#,
        tools(vec![("file.write", echo)]),
    )
    .await;
    let failure = outcome
        .failure
        .as_ref()
        .unwrap_or_else(|| panic!("unknown member must fail the run, got {outcome:?}"));
    assert!(failure.message.contains("file.readx"), "{failure:?}");
    assert!(failure.message.contains("write"), "{failure:?}");
}

/// B3: typeof on a missing name ends the run. Intentional: reading a name that
/// does not exist is judged before the program can branch on the value.
#[tokio::test]
async fn p10_typeof_unknown_suite_ends_the_run() {
    let echo = HostFn::new(|_, _, _, _| async move { Ok(json!({ "ok": true })) });
    let outcome = run(
        r#"const hint = typeof tools.zzz; return hint;"#,
        tools(vec![("alpha.one", echo)]),
    )
    .await;
    let failure = outcome
        .failure
        .as_ref()
        .unwrap_or_else(|| panic!("typeof of a missing name must end the run, got {outcome:?}"));
    assert!(failure.message.contains("zzz"), "{failure:?}");
}

/// B3: membership answers false for a missing name, and a suite is the same
/// object every time it is read.
#[tokio::test]
async fn p11_membership_is_false_and_suites_are_cached() {
    let echo = HostFn::new(|_, _, _, _| async move { Ok(json!({ "ok": true })) });
    let outcome = run(
        r#"
        const same = tools.alpha === tools.alpha;
        return [same, "zzz" in tools, "alpha" in tools];
        "#,
        tools(vec![("alpha.one", echo)]),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!([true, false, true]));
}

// P4 and P9 moved to the bridge: a field list is only meaningful against the
// contract, and the engine harness has none. See
// `src/tools/code/bridge_tests.rs::s4_*`.

/// P8: the read proxy must not misfire on the ways a result is normally used.
/// (Green before B4; must stay green after it.)
#[tokio::test]
async fn p8_proxy_passthrough_does_not_misfire() {
    let host = HostFn::new(|_, _, _, _| async move {
        Ok(json!({ "ok": true, "lines": ["a"], "path": "p", "start_line": 1 }))
    });
    let outcome = run(
        r#"
        const r = await tools.read({ path: "p" });
        const awaited = await r;
        const text = JSON.stringify(r);
        const rendered = String(r);
        const spread = { ...r };
        const ctor = typeof r.constructor;
        console.log(r);
        return [awaited.lines.length, text.includes('"lines"'), rendered.length > 0, spread.path, ctor];
        "#,
        tools_with_contract(
            vec![("read", host)],
            &["ok", "lines", "path", "start_line"],
        ),
    )
    .await;
    assert_eq!(
        value_of(&outcome),
        &json!([1, true, true, "p", "function"])
    );
}

/// P5: benchmark for the per-field read cost. Not a gate. (Expected flat
/// before B4; re-run after it.)
#[tokio::test]
#[ignore = "benchmark: field reads through the read proxy"]
async fn p5_read_proxy_overhead() {
    let host = HostFn::new(|_, _, _, _| async move {
        Ok(json!({ "ok": true, "lines": ["a"], "path": "p", "start_line": 1 }))
    });
    let baseline = Instant::now();
    let none = run(
        r#"
        const r = await tools.read({ path: "p" });
        return r.path === "p" ? 1 : 0;
        "#,
        tools_with_contract(
            vec![("read", host.clone())],
            &["ok", "lines", "path", "start_line"],
        ),
    )
    .await;
    let baseline = baseline.elapsed();
    assert_eq!(value_of(&none), &json!(1));
    let thousand = Instant::now();
    let outcome = run(
        r#"
        const r = await tools.read({ path: "p" });
        let n = 0;
        for (let i = 0; i < 1000; i++) { if (r.path === "p") n++; }
        return n;
        "#,
        tools_with_contract(
            vec![("read", host)],
            &["ok", "lines", "path", "start_line"],
        ),
    )
    .await;
    let thousand = thousand.elapsed();
    assert_eq!(value_of(&outcome), &json!(1000));
    println!(
        "p5: 1000 proxied reads {:?} vs 1 read {:?} => {:?} per read",
        thousand,
        baseline,
        (thousand.saturating_sub(baseline)) / 1000
    );
}
