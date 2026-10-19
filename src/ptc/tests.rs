use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::{
    CallState, ExecuteRequest, FailureKind, HostBinding, HostError, HostFn, Limits, LogLevel,
    Namespace, Outcome, ResourceKind, execute,
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
    outcome.value.as_ref().expect("success carries a value")
}

fn failure_kind(outcome: &Outcome) -> FailureKind {
    outcome
        .failure
        .as_ref()
        .unwrap_or_else(|| panic!("expected a failure, got {outcome:?}"))
        .kind
}

#[tokio::test]
async fn returns_json_values() {
    let outcome = run(
        r#"
        const bare = Object.create(null);
        bare.a = 1;
        return { n: null, t: true, f: false, i: 1, x: 1.5, s: "hi", a: [1, "a", null], o: bare };
        "#,
        Vec::new(),
    )
    .await;
    assert_eq!(
        value_of(&outcome),
        &json!({"n": null, "t": true, "f": false, "i": 1, "x": 1.5, "s": "hi", "a": [1, "a", null], "o": {"a": 1}})
    );
}

#[tokio::test]
async fn log_levels() {
    let outcome = run(
        r#"
        console.log("L");
        console.info("I");
        console.warn("W");
        console.error("E");
        console.debug("D");
        return null;
        "#,
        Vec::new(),
    )
    .await;
    assert_eq!(value_of(&outcome), &Value::Null);
    let levels: Vec<_> = outcome.logs.iter().map(|entry| entry.level).collect();
    assert_eq!(
        levels,
        vec![
            LogLevel::Log,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
            LogLevel::Debug,
        ]
    );
    let text: Vec<_> = outcome
        .logs
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(text, vec!["L", "I", "W", "E", "D"]);
}

#[tokio::test]
async fn concurrent_host_calls() {
    let slow = HostFn::new(|_, _, _| async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        Ok(json!("slow"))
    });
    let fast = HostFn::new(|_, _, _| async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        Ok(json!("fast"))
    });
    let outcome = run(
        "await Promise.all([tools.slow(null), tools.fast(null)]); return null;",
        tools(vec![("slow", slow), ("fast", fast)]),
    )
    .await;
    assert_eq!(value_of(&outcome), &Value::Null);
    assert_eq!(outcome.calls.len(), 2);
    assert_eq!(outcome.calls[0].seq, 1);
    assert_eq!(outcome.calls[0].namespace, "tools");
    assert_eq!(outcome.calls[0].name, "slow");
    assert_eq!(outcome.calls[0].state, CallState::Completed);
    assert_eq!(outcome.calls[1].seq, 2);
    assert_eq!(outcome.calls[1].name, "fast");
    assert_eq!(outcome.calls[1].state, CallState::Completed);
    assert!(outcome.calls[1].ended.unwrap() < outcome.calls[0].ended.unwrap());
}

#[tokio::test]
async fn in_flight_limit() {
    let calls = Arc::new(AtomicUsize::new(0));
    let hold_calls = Arc::clone(&calls);
    let ping_calls = Arc::clone(&calls);
    let hold = HostFn::new(move |_, _, _| {
        hold_calls.fetch_add(1, Ordering::SeqCst);
        async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            Ok(json!(null))
        }
    });
    let ping = HostFn::new(move |_, _, _| {
        ping_calls.fetch_add(10, Ordering::SeqCst);
        async move { Ok(json!(null)) }
    });
    let mut limits = limits();
    limits.max_in_flight = 1;
    let outcome = run_with(
        r#"
        const pending = tools.hold(null);
        let message = "none";
        try { await tools.ping(null); message = "called"; }
        catch (e) { message = String(e.message); }
        await pending;
        return message;
        "#,
        tools(vec![("hold", hold), ("ping", ping)]),
        limits,
        Duration::from_secs(2),
        Duration::from_millis(200),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!("in-flight call limit exceeded"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(outcome.calls.len(), 1);
    assert_eq!(outcome.calls[0].name, "hold");
    assert_eq!(outcome.calls[0].state, CallState::Completed);
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
    assert!(outcome.value.is_none());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn cancel_while_js_running_cancels_inflight() {
    let started = Arc::new(Notify::new());
    let saw = Arc::new(AtomicBool::new(false));
    let started_host = Arc::clone(&started);
    let saw_host = Arc::clone(&saw);
    let watch = HostFn::new(move |_, cancel, _| {
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
    assert!(outcome.value.is_none());
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
    let watch = HostFn::new(move |_, cancel, _| {
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
    assert!(outcome.value.is_none());
    assert!(saw.load(Ordering::SeqCst));
    assert_eq!(outcome.calls[0].state, CallState::CancelledFinished);
}

#[tokio::test]
async fn cancel_grace_records_unfinished() {
    let started = Arc::new(Notify::new());
    let token_slot: Arc<Mutex<Option<CancellationToken>>> = Arc::new(Mutex::new(None));
    let started_host = Arc::clone(&started);
    let token_host = Arc::clone(&token_slot);
    let hang = HostFn::new(move |_, cancel, _| {
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
    let pause_host = HostFn::new(|_, _, pause| async move {
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

    let plain = HostFn::new(|_, cancel, _| async move {
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
        failure.message.contains("Maximum call stack size exceeded"),
        "{}",
        failure.message
    );
}

#[tokio::test]
async fn syntax_error() {
    let outcome = run("return (", Vec::new()).await;
    let failure = outcome.failure.expect("syntax");
    assert_eq!(failure.kind, FailureKind::Syntax);
    assert!(!failure.message.is_empty());
    assert!(outcome.calls.is_empty());
    assert!(outcome.value.is_none());
}

#[tokio::test]
async fn exception_includes_stack() {
    let outcome = run(
        r#"
        function inner() { throw new Error("boom"); }
        function outer() { return inner(); }
        return outer();
        "#,
        Vec::new(),
    )
    .await;
    let failure = outcome.failure.expect("exception");
    assert_eq!(failure.kind, FailureKind::Exception);
    assert!(failure.message.contains("boom"), "{}", failure.message);
    let stack = failure.stack.expect("stack");
    assert!(stack.contains("inner"), "{stack}");
    assert!(stack.contains("outer"), "{stack}");
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
    assert!(outcome.value.is_none());
    assert_eq!(outcome.logs.len(), 2);
    assert_eq!(outcome.logs[0].message, "abcdefghij");
    assert_eq!(outcome.logs[1].message, "XY");
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
    assert_eq!(outcome.logs.len(), 1);
    assert_eq!(outcome.logs[0].message, "ab");
    assert!(outcome.value.is_none());

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
    assert_eq!(outcome.logs[0].message, "ab");
}

#[tokio::test]
async fn illegal_output_values() {
    let cases = [
        ("return NaN;", "non-finite number"),
        ("return Infinity;", "non-finite number"),
        ("return -Infinity;", "non-finite number"),
        ("return undefined;", "undefined"),
        ("return { a: 1, b: undefined };", "undefined property"),
        ("return 1n;", "bigint"),
        ("return function () {};", "function"),
        ("return Symbol('s');", "symbol"),
        ("const a = {}; a.self = a; return a;", "circular reference"),
        ("return new Date();", "Date"),
        ("return [1,,2];", "sparse array"),
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
        assert!(outcome.value.is_none());
    }
}

#[tokio::test]
async fn illegal_arguments_skip_host() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_host = Arc::clone(&calls);
    let host = HostFn::new(move |_, _, _| {
        calls_host.fetch_add(1, Ordering::SeqCst);
        async move { Ok(json!(null)) }
    });
    let cases = [
        ("await tools.f(NaN);", "non-finite number"),
        ("await tools.f(Infinity);", "non-finite number"),
        ("await tools.f(undefined);", "undefined"),
        ("await tools.f({ a: undefined });", "undefined property"),
        ("await tools.f(1n);", "bigint"),
        ("await tools.f(function () {});", "function"),
        ("await tools.f(Symbol('s'));", "symbol"),
        (
            "const a = {}; a.self = a; await tools.f(a);",
            "circular reference",
        ),
        ("await tools.f(new Date());", "Date"),
        ("await tools.f([1,,2]);", "sparse array"),
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
async fn host_safe_integer_round_trips() {
    let host = HostFn::new(|_, _, _| async move { Ok(json!(9_007_199_254_740_991_i64)) });
    let outcome = run("return await tools.n(null);", tools(vec![("n", host)])).await;
    assert_eq!(value_of(&outcome), &json!(9_007_199_254_740_991_i64));
    assert_eq!(outcome.calls[0].state, CallState::Completed);
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
        let host = HostFn::new(move |_, _, _| {
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

    let fraction = HostFn::new(|_, _, _| async move { Ok(json!(1.25)) });
    let outcome = run("return await tools.n(null);", tools(vec![("n", fraction)])).await;
    assert_eq!(value_of(&outcome), &json!(1.25));
}

#[tokio::test]
async fn no_ambient_capabilities() {
    let outcome = run(
        r#"
        const names = ["fetch", "require", "process", "setTimeout", "std", "os"];
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
async fn libs_samples() {
    let outcome = run(
        r#"
        return {
          sum: libs.stats.sum([1, 2, 3, 4]),
          median: libs.stats.median([1, 2, 3, 4]),
          max: libs.stats.max([1, 5, 2]),
          csv: libs.csv.parse('a,b\n1,"x,y"'),
          lines: libs.text.lines("a\r\nb"),
          counts: libs.data.countBy([{k:"a"},{k:"a"},{k:"b"}], "k"),
          unique: libs.data.unique([1, 1, 2])
        };
        "#,
        Vec::new(),
    )
    .await;
    assert_eq!(
        value_of(&outcome),
        &json!({
            "sum": 10,
            "median": 2.5,
            "max": 5,
            "csv": [["a", "b"], ["1", "x,y"]],
            "lines": ["a", "b"],
            "counts": {"a": 2, "b": 1},
            "unique": [1, 2]
        })
    );
}

#[tokio::test]
async fn unawaited_calls_are_waited_and_recorded() {
    let finished = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&finished);
    let echo = HostFn::new(move |args, _, _| {
        let flag = Arc::clone(&flag);
        async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            flag.store(true, Ordering::SeqCst);
            Ok(args)
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
    assert_eq!(outcome.calls[0].state, CallState::Completed);
    assert!(outcome.calls[0].ended.is_some());
}

#[tokio::test]
async fn cold_start_median() {
    let mut samples = Vec::new();
    for _ in 0..11 {
        let started = Instant::now();
        let outcome = run("return 1;", Vec::new()).await;
        samples.push(started.elapsed());
        assert_eq!(value_of(&outcome), &json!(1));
    }
    samples.remove(0);
    samples.sort();
    let median = samples[samples.len() / 2];
    eprintln!("ptc cold start median {median:?}");
    assert!(
        median < Duration::from_millis(250),
        "cold start median {median:?}"
    );
}

#[tokio::test]
async fn host_error_finishes_the_call() {
    let host = HostFn::new(|_, _, _| async move { Err(HostError::new("nope")) });
    let outcome = run(
        r#"try { await tools.f(null); return "called"; } catch (e) { return String(e.message); }"#,
        tools(vec![("f", host)]),
    )
    .await;
    assert_eq!(value_of(&outcome), &json!("nope"));
    assert_eq!(outcome.calls[0].state, CallState::Completed);
}
