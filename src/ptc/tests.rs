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
        wall_budget: None,
        grace,
        cancel,
        fs_root: None,
        fs_allow_outside_workspace: false,
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

fn last_json(outcome: &Outcome) -> Option<&super::Json> {
    outcome
        .entries
        .last()
        .and_then(|entry| entry.values.last())
}

fn value_of(outcome: &Outcome) -> &Value {
    assert!(
        outcome.failure.is_none(),
        "expected success, got {outcome:?}"
    );
    last_json(outcome)
        .expect("success carries an inspected value")
        .as_value()
}

#[tokio::test]
async fn a_proxy_of_a_plain_object_is_presented() {
    let outcome = run("inspect(new Proxy({ a: 1 }, {}));", tools(vec![])).await;
    assert!(
        outcome.failure.is_none(),
        "a proxy result must be presented: {outcome:?}"
    );
    assert_eq!(value_of(&outcome), &json!({ "a": 1 }));
}

/// `fs.ls` keeps its value pure (`{ entries }`) and surfaces truncation via the
/// run's upfloat warnings channel, matching `glob` / `grep` / `read`.
#[tokio::test]
async fn fs_ls_surfaces_truncation_as_upfloat_warning() {
    let dir = tempfile::tempdir().unwrap();
    for index in 0..(crate::ptc::fs::LS_CAP + 1) {
        std::fs::write(dir.path().join(format!("f{index:04}.txt")), "x").unwrap();
    }
    let outcome = execute(ExecuteRequest {
        source: "inspect(fs.ls(\".\"));".into(),
        namespaces: Vec::new(),
        limits: limits(),
        budget: Duration::from_secs(5),
        wall_budget: None,
        grace: Duration::from_millis(200),
        cancel: CancellationToken::new(),
        fs_root: Some(dir.path().to_path_buf()),
        fs_allow_outside_workspace: false,
    })
    .await;
    let value = value_of(&outcome);
    assert!(value.get("truncated").is_none(), "{value}");
    assert_eq!(value["entries"].as_array().unwrap().len(), crate::ptc::fs::LS_CAP);
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.contains("fs.ls(\".\")") && w.contains("showing first 500 of 501 entries")),
        "warnings={:?}",
        outcome.warnings
    );
}

#[tokio::test]
async fn fs_ls_and_stat_throw_error_on_missing_or_outside() {
    let dir = tempfile::tempdir().unwrap();
    let outcome = execute(ExecuteRequest {
        source: r#"
            const out = {};
            try { fs.ls("nope"); } catch (e) { out.ls = [e instanceof Error, e.code]; }
            try { fs.stat("../"); } catch (e) { out.stat = [e instanceof Error, e.code]; }
            inspect(out);
        "#
        .into(),
        namespaces: Vec::new(),
        limits: limits(),
        budget: Duration::from_secs(5),
        wall_budget: None,
        grace: Duration::from_millis(200),
        cancel: CancellationToken::new(),
        fs_root: Some(dir.path().to_path_buf()),
        fs_allow_outside_workspace: false,
    })
    .await;
    let value = value_of(&outcome);
    assert_eq!(value["ls"], json!([true, "not_found"]), "{value}");
    assert_eq!(value["stat"], json!([true, "permission"]), "{value}");
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
        "inspect(await tools.deck.only({ n: 1 }));",
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
    assert!(outcome.entries.is_empty());
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
    assert!(outcome.entries.is_empty());
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
        "inspect(await tools.watch(null));",
        tools(vec![("watch", watch)]),
        limits(),
        Duration::from_secs(5),
        Duration::from_millis(400),
        cancel,
    )
    .await;
    waiter.await.unwrap();
    assert_eq!(failure_kind(&outcome), FailureKind::Cancelled);
    assert!(outcome.entries.is_empty());
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
        "inspect(await tools.hang(null));",
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
        "inspect(await tools.wait(null));",
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
        "inspect(await tools.wait(null));",
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
        "const blob = new Uint8Array(16 * 1024 * 1024); inspect(blob[0]);",
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
          inspect("caught " + blob[0]);
        } catch (e) {
          inspect("caught");
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
        outcome.entries.is_empty(),
        "program emitted {:?}",
        outcome.entries
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
          inspect("caught " + blob[0]);
        } catch (e) {
          inspect("caught");
        }
        "#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    let failure = outcome.failure.expect("memory fault overrides success");
    assert_eq!(failure.kind, FailureKind::ResourceLimit, "{failure:?}");
    assert_eq!(failure.resource, Some(ResourceKind::Memory), "{failure:?}");
}

#[tokio::test]
async fn stack_limit() {
    let mut limits = limits();
    limits.stack_bytes = 64 * 1024;
    let outcome = run_with(
        "function f(){ return 1 + f(); } inspect(f());",
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
        try { f(); } catch (e) { return; }
        "#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
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
          inspect(f());
        } catch (e) {
          inspect("caught");
        }
        "#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    let failure = outcome.failure.expect("stack overflow is not catchable");
    assert_eq!(failure.kind, FailureKind::ResourceLimit, "{failure:?}");
    assert_eq!(failure.resource, Some(ResourceKind::Stack), "{failure:?}");
}

#[tokio::test]
async fn success_survives_normal_inspect() {
    let outcome = run("inspect(\"kept\");", Vec::new()).await;
    assert_eq!(value_of(&outcome), &json!("kept"), "{outcome:?}");
    assert!(outcome.failure.is_none(), "{outcome:?}");
}

#[tokio::test]
async fn output_limit_is_uncatchable_and_keeps_prior_entries() {
    let mut limits = limits();
    limits.output_bytes = 12;
    let outcome = run_with(
        r#"
        inspect("abcdefghij");
        try { inspect("XYZ"); inspect("caught"); }
        catch (e) { inspect("caught"); }
        "#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(failure_kind(&outcome), FailureKind::OutputLimit);
    assert_eq!(outcome.entries.len(), 1);
    assert_eq!(outcome.entries[0].values[0].as_value(), &json!("abcdefghij"));
}

#[tokio::test]
async fn output_limit_sums_across_inspect_calls() {
    let mut limits = limits();
    limits.output_bytes = 8;
    let outcome = run_with(
        r#"inspect("ab"); inspect("hello");"#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(failure_kind(&outcome), FailureKind::OutputLimit);
    assert_eq!(outcome.entries.len(), 1);
    assert_eq!(outcome.entries[0].values[0].as_value(), &json!("ab"));

    limits.output_bytes = 5;
    let outcome = run_with(
        r#"inspect("ab"); inspect(1);"#,
        Vec::new(),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    assert!(outcome.failure.is_none(), "{outcome:?}");
    assert_eq!(outcome.entries.len(), 2);
    assert_eq!(outcome.entries[0].values[0].as_value(), &json!("ab"));
    assert_eq!(outcome.entries[1].values[0].as_value(), &json!(1));
}

#[tokio::test]
async fn inspect_accepts_arbitrary_js_values_without_failing() {
    let host = HostFn::new(|_, _, _, _| async move {
        Err(HostError::tool(
            "no_match",
            "missing line",
            Some(json!({ "line": 42 })),
            true,
        ))
    });
    let outcome = run(
        r#"
        const circObj = { a: 1 }; circObj.self = circObj;
        const circArr = [1]; circArr.push(circArr);
        const circSet = new Set([1]); circSet.add(circSet);
        const circMap = new Map([["a", 1]]); circMap.set("self", circMap);
        const symObj = { a: 1 }; symObj[Symbol("s")] = 2;
        class Box { constructor(v) { this.v = v; } }
        class Empty {}
        let toolErr = null;
        try { await tools.edit({}); } catch (e) { toolErr = e; }
        inspect({
            bi: 123456789012345678901234567890n,
            sym: Symbol("tag"),
            fn: function helper() {},
            anon: () => {},
            err: new TypeError("bad type"),
            toolErr,
            circObj,
            circArr,
            date: new Date("2026-01-02T03:04:05.000Z"),
            badDate: new Date(NaN),
            re: /foo\d+/gi,
            set: new Set(["a", "b", "c"]),
            circSet,
            mapStr: new Map([["x", 1], ["y", 2]]),
            mapNum: new Map([[1, "one"]]),
            circMap,
            bytes: new Uint8Array([1, 2, 255]),
            bigBytes: new Uint8Array(5000),
            buf: new ArrayBuffer(16),
            prom: Promise.resolve(1),
            smallSparse: [1, , 2],
            symObj,
            box: new Box(9),
            empty: new Empty(),
            nan: NaN,
            inf: Infinity,
            ninf: -Infinity,
            ns: tools,
        });
        "#,
        tools(vec![("edit", host)]),
    )
    .await;
    assert!(outcome.failure.is_none(), "{outcome:?}");
    let v = value_of(&outcome);
    assert_eq!(v["bi"], json!("123456789012345678901234567890n"));
    assert_eq!(v["sym"], json!("Symbol(tag)"));
    assert_eq!(v["fn"], json!("[Function: helper]"));
    assert!(v["anon"].as_str().unwrap().starts_with("[Function"));
    assert_eq!(v["err"]["name"], json!("TypeError"));
    assert_eq!(v["err"]["message"], json!("bad type"));
    assert_eq!(v["toolErr"]["code"], json!("no_match"));
    assert_eq!(v["toolErr"]["tool"], json!("edit"));
    assert_eq!(v["toolErr"]["retryable"], json!(true));
    assert_eq!(v["toolErr"]["details"], json!({ "line": 42 }));
    assert_eq!(v["circObj"], json!({ "a": 1, "self": "[Circular]" }));
    assert_eq!(v["circArr"], json!([1, "[Circular]"]));
    assert_eq!(v["date"], json!("2026-01-02T03:04:05.000Z"));
    assert_eq!(v["badDate"], json!("[Invalid Date]"));
    assert_eq!(v["re"], json!("/foo\\d+/gi"));
    assert_eq!(v["set"], json!(["a", "b", "c"]));
    assert_eq!(v["circSet"], json!([1, "[Circular]"]));
    assert_eq!(v["mapStr"], json!({ "x": 1, "y": 2 }));
    assert_eq!(v["mapNum"], json!([[1, "one"]]));
    assert_eq!(v["circMap"], json!({ "a": 1, "self": "[Circular]" }));
    assert_eq!(v["bytes"], json!([1, 2, 255]));
    assert!(
        v["bigBytes"]
            .as_str()
            .unwrap()
            .starts_with("[Uint8Array(5000):"),
        "{}",
        v["bigBytes"]
    );
    assert_eq!(v["buf"], json!("[ArrayBuffer(byteLength=16)]"));
    assert_eq!(v["prom"], json!("[Promise]"));
    assert_eq!(v["smallSparse"], json!([1, null, 2]));
    assert_eq!(v["symObj"], json!({ "a": 1 }));
    assert_eq!(v["box"], json!({ "v": 9 }));
    assert_eq!(v["empty"], json!("[Empty]"));
    assert_eq!(v["nan"], json!("NaN"));
    assert_eq!(v["inf"], json!("Infinity"));
    assert_eq!(v["ninf"], json!("-Infinity"));
    assert_eq!(v["ns"]["edit"], json!("[Function: edit]"));
}

#[tokio::test]
async fn console_is_removed_and_guides_to_inspect() {
    let outcome = run("console.log('hi');", Vec::new()).await;
    let failure = outcome.failure.expect("console must fail");
    assert_eq!(failure.kind, FailureKind::Exception);
    assert!(
        failure.message.contains("use inspect(value) to emit output"),
        "{}",
        failure.message
    );
}

#[tokio::test]
async fn top_level_return_value_is_rejected_before_any_host_call() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let host = HostFn::new(move |_, _, _, _| {
        counter.fetch_add(1, Ordering::SeqCst);
        async move { Ok(json!(1)) }
    });
    let outcome = run(
        "const r = await tools.f({});\nreturn r;",
        tools(vec![("f", host)]),
    )
    .await;
    let failure = outcome.failure.expect("top-level return value must fail");
    assert_eq!(failure.kind, FailureKind::Syntax);
    assert!(failure.message.contains("inspect(value)"), "{failure:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    // Bare top-level `return;` works as early exit.
    let early = run("inspect('first'); return; inspect('second');", Vec::new()).await;
    assert!(early.failure.is_none(), "{early:?}");
    assert_eq!(early.entries.len(), 1);
    assert_eq!(early.entries[0].values[0].as_value(), &json!("first"));
}

#[tokio::test]
async fn unknown_tool_is_a_contract_failure() {
    // B3: naming a tool that does not exist is a contract failure, not a
    // catchable TypeError with a bare "not a function".
    let outcome = run("inspect(await tools.nope({}));", tools(vec![])).await;
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
        let source = format!(
            "try {{ {body} inspect('called'); }} catch (e) {{ inspect(String(e.message)); }}"
        );
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
            try { inspect({ ok: true, value: await tools.n(null) }); }
            catch (e) { inspect({ ok: false, message: String(e.message) }); }
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
    let outcome = run("inspect(await tools.n(null));", tools(vec![("n", fraction)])).await;
    assert_eq!(value_of(&outcome), &json!(1.25));
}

#[tokio::test]
async fn no_ambient_capabilities() {
    let outcome = run(
        r#"
        const names = ["fetch", "require", "process", "setTimeout", "std", "os", "Atomics", "SharedArrayBuffer", "console"];
        const present = names.filter((name) => typeof globalThis[name] !== "undefined");
        let imported = false;
        try { await import("fs"); imported = true; } catch (e) { imported = false; }
        inspect({
          present,
          imported,
          evalOk: eval("1+1") === 2,
          fnOk: (new Function("return 3"))() === 3
        });
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
        inspect("ok");
        "#,
        tools(vec![]),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!("ok"));
    assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
    assert!(
        outcome
            .warnings
            .iter()
            .any(|text| text.contains("unhandled promise rejection")
                && text.contains("boom-unhandled")),
        "{:?}",
        outcome.warnings
    );
}

#[tokio::test]
async fn handled_promise_rejection_does_not_warn() {
    let outcome = run(
        r#"
        Promise.reject("caught").catch(() => {});
        inspect("ok");
        "#,
        tools(vec![]),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!("ok"));
    assert!(
        outcome
            .warnings
            .iter()
            .all(|text| !text.contains("unhandled promise rejection")),
        "{:?}",
        outcome.warnings
    );
}

#[tokio::test]
async fn synchronous_type_error_in_wrapper_does_not_warn_unhandled_rejection() {
    let outcome = run(
        "const src = String.raw`Public entry point: ``run_tasks``.\n`;\ninspect(src);",
        tools(vec![]),
    )
    .await;
    let failure = outcome.failure.as_ref().expect("TypeError failure");
    assert_eq!(failure.kind, FailureKind::Exception);
    assert!(failure.uncaught_type_error, "{failure:?}");
    assert!(
        outcome
            .warnings
            .iter()
            .all(|text| !text.contains("unhandled promise rejection")),
        "{:?}",
        outcome.warnings
    );
}

#[tokio::test]
async fn wall_budget_stops_paused_host_wait_while_active_budget_stays_frozen() {
    let sleepy = HostFn::new(move |_args, cancel, pause, _info| async move {
        let _guard = pause.pause();
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(500)) => Ok(json!("finished")),
            _ = cancel.cancelled() => Err(crate::ptc::HostError::new("cancelled")),
        }
    });
    let outcome = execute(ExecuteRequest {
        source: "inspect(await tools.sleepy({}));".into(),
        namespaces: tools(vec![("sleepy", sleepy)]),
        limits: limits(),
        budget: Duration::from_millis(60),
        wall_budget: Some(Duration::from_millis(120)),
        grace: Duration::from_millis(50),
        cancel: CancellationToken::new(),
        fs_root: None,
        fs_allow_outside_workspace: false,
    })
    .await;
    let failure = outcome.failure.as_ref().expect("wall timeout");
    assert_eq!(failure.kind, FailureKind::Timeout);
    assert_eq!(failure.message, "wall budget exceeded");
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
        "tools.echo({ n: 1 }); inspect('done');",
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
        outcome
            .warnings
            .iter()
            .any(|text| text.contains("1 host call was still running")),
        "{:?}",
        outcome.warnings
    );
}

#[tokio::test]
async fn tool_error_is_a_plain_catchable_error() {
    let host = HostFn::new(|_, _, _, _| async move {
        Err(HostError::tool("not_found", "missing", None, false))
    });
    let outcome = run(
        r#"
        try {
          await tools.read(null);
          inspect("resolved");
        } catch (e) {
          inspect([
            e instanceof Error,
            e.name,
            e.code,
            e.tool,
            typeof e.seq,
            e.message,
            e.__litecode_host_call__ === true,
          ]);
        }
        "#,
        tools(vec![("read", host)]),
    )
    .await;
    assert_eq!(
        value_of(&outcome),
        &json!([
            true,
            "Error",
            "not_found",
            "read",
            "number",
            "missing",
            true
        ])
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
        "tools.slow(null); inspect('ok');",
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
        outcome
            .warnings
            .iter()
            .any(|text| text.contains("1 host call was still running")),
        "{:?}",
        outcome.warnings
    );

    let paused = HostFn::new(|_, _, pause, _| async move {
        let _guard = pause.pause();
        tokio::time::sleep(Duration::from_secs(2)).await;
        Ok(json!(null))
    });
    let started = Instant::now();
    let outcome = run_with(
        "tools.hold(null); inspect(1);",
        tools(vec![("hold", paused)]),
        limits(),
        Duration::from_millis(50),
        Duration::from_millis(80),
        CancellationToken::new(),
    )
    .await;
    assert!(
        started.elapsed() < Duration::from_millis(600),
        "paused call did not finish: {:?}",
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
        assert!(outcome.entries.is_empty());
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
        inspect(values);
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
async fn sparse_huge_array_summarized_before_allocation() {
    if !isolated(concat!(
        module_path!(),
        "::sparse_huge_array_summarized_before_allocation"
    )) {
        return;
    }
    let mut wide = limits();
    wide.memory_bytes = 64 * 1024 * 1024;
    wide.output_bytes = 1024 * 1024;
    for source in [
        "const sparse = []; sparse.length = 1e8; inspect(sparse);",
        "inspect(new Array(1e8));",
        "const sparse = []; sparse[0] = 1; sparse[3] = 2; sparse.length = 1e8; inspect(sparse);",
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
        assert!(outcome.failure.is_none(), "{source}: {outcome:?}");
        let text = value_of(&outcome).as_str().unwrap_or("");
        assert!(
            text.starts_with("[SparseArray(length=100000000"),
            "{source}: {text}"
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
    inspect(value);
    "#
}

async fn deep_nest_child() {
    {
        let inspected = run(deep_array_source(), Vec::new()).await;
        assert!(
            inspected.failure.is_none(),
            "deep inspect: {}",
            failure_text(&inspected)
        );
        let value = last_json(&inspected).expect("deep value");
        assert_eq!(array_depth(value.as_value()), 20_000);
        exercise_json(value);
    }

    {
        let ignored = HostFn::new(|_value, _, _, _| async move { Ok(json!(1)) });
        let ignored_argument = run(
            r#"
            let value = 0;
            for (let i = 0; i < 20000; i++) value = [value];
            inspect(await tools.f(value));
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
            "inspect(await tools.deep(null));",
            tools(vec![("deep", injected)]),
        )
        .await;
        assert!(
            host_to_js.failure.is_none(),
            "deep host result: {}",
            failure_text(&host_to_js)
        );
        let value = last_json(&host_to_js).expect("injected value");
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
        assert!(over_budget.entries.is_empty());
    }

    {
        let mixed = run(
            r#"
            let value = 0;
            for (let i = 0; i < 20000; i++) value = [value];
            inspect([value, 1n]);
            "#,
            Vec::new(),
        )
        .await;
        assert!(mixed.failure.is_none(), "{mixed:?}");
        let arr = value_of(&mixed).as_array().expect("pair");
        assert_eq!(array_depth(&arr[0]), 20_000);
        assert_eq!(arr[1], json!("1n"));
    }

    let mut wide = limits();
    wide.memory_bytes = 64 * 1024 * 1024;
    wide.output_bytes = 64 * 1024;
    let started = Instant::now();
    let sparse = run_with(
        "const sparse = []; sparse.length = 1e8; inspect(sparse);",
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
    assert!(sparse.failure.is_none(), "{sparse:?}");
    assert_eq!(
        value_of(&sparse),
        &json!("[SparseArray(length=100000000, populated=0)]")
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
        "await tools.echo({ path: 'a' }, { timeout_ms: 5 }); inspect(null);",
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
async fn undefined_in_inspect_is_stripped_or_nulled_and_reported() {
    let bare = run("inspect(undefined);", Vec::new()).await;
    assert!(bare.failure.is_none(), "{bare:?}");
    assert_eq!(value_of(&bare), &json!("undefined"));
    assert!(bare.dropped_undefined.is_empty());
    let cases: [(&str, Value, &[&str], usize); 4] = [
        ("inspect({ a: 1, b: undefined });", json!({"a": 1}), &["b"], 1),
        (
            "inspect({ a: { b: undefined } });",
            json!({"a": {}}),
            &["a.b"],
            1,
        ),
        (
            "inspect({ list: [1, undefined] });",
            json!({"list": [1, null]}),
            &["list[1]"],
            1,
        ),
        (
            "inspect({ 'a-b': undefined });",
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
    let outcome = run("inspect([undefined, undefined, 3]);", Vec::new()).await;
    assert_eq!(value_of(&outcome), &json!([null, null, 3]));
    assert_eq!(
        outcome.dropped_undefined.head_line().as_deref(),
        Some("dropped undefined at [0], [1]")
    );
    let clean = run("inspect({ a: null });", Vec::new()).await;
    assert!(clean.dropped_undefined.is_empty());
    assert!(clean.dropped_undefined.head_line().is_none());
}

#[tokio::test]
async fn dropped_undefined_list_is_capped_with_more_count() {
    let outcome = run(
        "const o = {}; for (let i = 0; i < 25; i++) o['k' + i] = undefined; o.keep = 1; inspect(o);",
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
        "await tools.f({ a: 1, b: undefined, c: [undefined, 2] }); await tools.f(undefined); inspect('done');",
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
        "const back = await tools.echo({zeta: 1, alpha: 2, mid: {y: 1, b: 2}});\ninspect({z: 1, a: back, m: 3});",
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
            "invalid_output",
            "tool 'read' returned data that does not match its JS contract",
            "read",
        ))
    });
    let outcome = run(
        r#"
        try { await tools.read({ path: "a.txt" }); } catch (e) { inspect("swallowed"); }
        inspect("done");
        "#,
        tools(vec![("read", host)]),
    )
    .await;
    let failure = outcome
        .failure
        .as_ref()
        .unwrap_or_else(|| panic!("a contract error must end the run, got {outcome:?}"));
    assert!(
        failure.message.contains("does not match its JS contract"),
        "{failure:?}"
    );
}

/// M2: a contract error inside Promise.all still ends the run.
#[tokio::test]
async fn m2_contract_error_inside_promise_all_ends_run() {
    let host = HostFn::new(|_, _, _, _| async move {
        Err(HostError::contract(
            "invalid_output",
            "tool 'read' returned data that does not match its JS contract",
            "read",
        ))
    });
    let outcome = run(
        r#"
        try { await Promise.all([tools.read({ path: "a.txt" })]); } catch (e) { inspect("swallowed"); }
        inspect("done");
        "#,
        tools(vec![("read", host)]),
    )
    .await;
    assert!(
        outcome.failure.is_some(),
        "a contract error must not resolve: {outcome:?}"
    );
}

/// M3: after a contract error nothing else is dispatched.
#[tokio::test]
async fn m3_no_host_call_after_contract_error() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let host = HostFn::new(move |_, _, _, _| {
        let counter = Arc::clone(&counter);
        async move {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(HostError::contract(
                    "invalid_output",
                    "tool 'read' returned data that does not match its JS contract",
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
        inspect("finished");
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
#[tokio::test]
async fn p2_unknown_member_throws_at_property_access() {
    let echo = HostFn::new(|_, _, _, _| async move { Ok(json!({ "ok": true })) });
    let outcome = run(
        r#"const f = tools.file.readx; inspect(typeof f);"#,
        tools(vec![("file.write", echo)]),
    )
    .await;
    let failure = outcome
        .failure
        .as_ref()
        .unwrap_or_else(|| panic!("reading an unknown member must fail, got {outcome:?}"));
    assert!(failure.message.contains("file.readx"), "{failure:?}");
}

/// P6: an unknown suite names the available suites.
#[tokio::test]
async fn p6_unknown_suite_names_the_candidates() {
    let echo = HostFn::new(|_, _, _, _| async move { Ok(json!({ "ok": true })) });
    let outcome = run(
        r#"
        try { tools.zzz.one({ path: "a" }); } catch (e) { inspect("swallowed"); }
        inspect("done");
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
#[tokio::test]
async fn p7_unknown_member_names_the_candidates() {
    let echo = HostFn::new(|_, _, _, _| async move { Ok(json!({ "ok": true })) });
    let outcome = run(
        r#"
        try { await tools.file.readx({ path: "a" }); } catch (e) { inspect("swallowed"); }
        inspect("done");
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

/// B3: typeof on a missing name ends the run.
#[tokio::test]
async fn p10_typeof_unknown_suite_ends_the_run() {
    let echo = HostFn::new(|_, _, _, _| async move { Ok(json!({ "ok": true })) });
    let outcome = run(
        r#"const hint = typeof tools.zzz; inspect(hint);"#,
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
        inspect([same, "zzz" in tools, "alpha" in tools]);
        "#,
        tools(vec![("alpha.one", echo)]),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!([true, false, true]));
}

/// P8: the read proxy must not misfire on the ways a result is normally used.
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
        inspect(r);
        inspect([awaited.lines.length, text.includes('"lines"'), rendered.length > 0, spread.path, ctor]);
        "#,
        tools_with_contract(
            vec![("read", host)],
            &["ok", "lines", "path", "start_line"],
        ),
    )
    .await;
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(
        value_of(&outcome),
        &json!([1, true, true, "p", "function"])
    );
}

/// P5: benchmark for the per-field read cost. Not a gate.
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
        inspect(r.path === "p" ? 1 : 0);
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
        inspect(n);
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

#[tokio::test]
async fn erasable_typescript_runs_and_preserves_lines() {
    let echo = HostFn::new(|args, _, _, _| async move { Ok(args.into_value()) });
    let outcome = run(
        r#"
        interface EchoInput {
            path: string;
            extra?: number;
        }
        type Summary = {
            path: string;
            total: number;
            tag: "ok";
        };
        const req: EchoInput = { path: "src/lib.rs" };
        const res = await tools.echo<EchoInput>(req) as EchoInput;
        const picked = <string>res.path!;
        function sum<T extends number>(...nums: T[]): number {
            return nums.reduce((acc, n) => acc + n, 0);
        }
        const check = (x?: number): number => (x ?? 10);
        const out = {
            path: picked,
            total: sum<number>(1, 2, check()),
            tag: "ok" as const,
        } satisfies Summary;
        inspect(out);
        "#,
        tools(vec![("echo", echo)]),
    )
    .await;
    assert_eq!(
        value_of(&outcome),
        &json!({ "path": "src/lib.rs", "total": 13, "tag": "ok" })
    );

    let thrown = run(
        "const a: number = 1;\nconst b: string = \"x\";\nthrow new Error(\"boom\");\n",
        Vec::new(),
    )
    .await;
    let failure = thrown.failure.expect("must fail");
    assert_eq!(failure.kind, FailureKind::Exception);
    assert_eq!(failure.location.map(|l| l.line), Some(3));
}

#[tokio::test]
async fn non_erasable_typescript_fails_syntax() {
    let enum_outcome = run("enum E { A }\ninspect(1);\n", Vec::new()).await;
    let enum_failure = enum_outcome.failure.expect("enum must fail");
    assert_eq!(enum_failure.kind, FailureKind::Syntax);
    assert!(enum_failure.message.contains("enum"));

    let ns_outcome = run("namespace N { export const v = 1; }\ninspect(1);\n", Vec::new()).await;
    let ns_failure = ns_outcome.failure.expect("namespace must fail");
    assert_eq!(ns_failure.kind, FailureKind::Syntax);

    let prop_outcome = run(
        "class C { constructor(public x: number) {} }\ninspect(new C(1).x);\n",
        Vec::new(),
    )
    .await;
    let prop_failure = prop_outcome.failure.expect("parameter property must fail");
    assert_eq!(prop_failure.kind, FailureKind::Syntax);
}

#[tokio::test]
async fn unresolved_promise_deadlock_aborts_fast() {
    let started = Instant::now();
    let outcome = run_with(
        "await new Promise(() => {});",
        Vec::new(),
        limits(),
        Duration::from_secs(10),
        Duration::from_millis(200),
        CancellationToken::new(),
    )
    .await;
    let elapsed = started.elapsed();
    let failure = outcome.failure.expect("deadlocked promise must abort");
    assert_eq!(failure.kind, FailureKind::Exception);
    assert!(
        failure.message.contains("unresolved Promise deadlock"),
        "{failure:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "deadlock should abort rapidly, took {elapsed:?}"
    );
}

#[tokio::test]
async fn slow_host_call_inside_manual_promise_does_not_false_deadlock() {
    let slow = HostFn::new(|_, _, _, _| async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        Ok(json!({ "ok": true }))
    });
    let outcome = run(
        r#"
        const p = tools.slow({});
        inspect(await new Promise((resolve) => p.then(resolve)));
        "#,
        tools(vec![("slow", slow)]),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!({ "ok": true }));
}

#[tokio::test]
async fn sync_infinite_loop_hits_timeout_promptly() {
    let started = Instant::now();
    let outcome = run_with(
        "while (true) {}",
        Vec::new(),
        limits(),
        Duration::from_millis(80),
        Duration::from_millis(50),
        CancellationToken::new(),
    )
    .await;
    let elapsed = started.elapsed();
    let failure = outcome.failure.expect("infinite loop must time out");
    assert_eq!(failure.kind, FailureKind::Timeout);
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
}
