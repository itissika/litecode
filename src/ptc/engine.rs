//! QuickJS-NG driver. The context lives on this thread; host futures are
//! spawned onto the caller's tokio runtime and are not aborted when the
//! program stops.

use std::cell::RefCell;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::Duration;

use std::panic::AssertUnwindSafe;

use futures_util::FutureExt;
use futures_util::future::{Either, select};
use rquickjs::context::EvalOptions;
use rquickjs::prelude::{Async, Rest};
use rquickjs::{AsyncContext, AsyncRuntime, Ctx, Error, Function, Object, Promise, Value};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use super::boundary::{self, BoundaryError};
use super::budget::BudgetClock;
use super::calls::Ledger;
use super::output::OutputLedger;
use super::{ExecuteRequest, Failure, HostFn, LogLevel, Outcome, ResourceKind};

struct Flags {
    output: AtomicBool,
}

enum Stop {
    Cancel,
    Timeout,
    Output,
}

enum ProgramEnd {
    Success(serde_json::Value),
    Failure(Failure),
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct ThreadWaker(thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park(),
        }
    }
}

pub(crate) fn drive_blocking(request: ExecuteRequest, handle: Handle) -> Outcome {
    block_on(drive(request, handle))
}

async fn drive(request: ExecuteRequest, handle: Handle) -> Outcome {
    let clock = Arc::new(BudgetClock::new(request.budget));
    let calls = Arc::new(Ledger::new(
        request.limits.max_in_flight,
        request.cancel.clone(),
    ));
    let flags = Arc::new(Flags {
        output: AtomicBool::new(false),
    });
    let output = Rc::new(RefCell::new(OutputLedger::new(request.limits.output_bytes)));
    let user = request.cancel.clone();
    let grace = request.grace;
    let output_limit = request.limits.output_bytes;

    let runtime = match AsyncRuntime::new() {
        Ok(runtime) => runtime,
        Err(error) => return failure_outcome(Failure::internal(error.to_string())),
    };
    runtime.set_memory_limit(request.limits.memory_bytes).await;
    {
        let user = user.clone();
        let flags = Arc::clone(&flags);
        let clock = Arc::clone(&clock);
        runtime
            .set_interrupt_handler(Some(Box::new(move || {
                user.is_cancelled()
                    || flags.output.load(Ordering::Relaxed)
                    || clock.should_interrupt()
            })))
            .await;
    }

    let context = match AsyncContext::full(&runtime).await {
        Ok(context) => context,
        Err(error) => return failure_outcome(Failure::internal(error.to_string())),
    };

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let watcher = AbortOnDrop(spawn_watcher(
        handle.clone(),
        Arc::clone(&calls),
        Arc::clone(&clock),
        Arc::clone(&flags),
        user.clone(),
        stop_tx,
    ));

    let namespaces = request.namespaces.clone();
    let installed = context
        .async_with({
            let calls = Arc::clone(&calls);
            let clock = Arc::clone(&clock);
            let user = user.clone();
            let flags = Arc::clone(&flags);
            let output = Rc::clone(&output);
            let handle = handle.clone();
            async move |ctx| {
                install(
                    &ctx,
                    &namespaces,
                    &handle,
                    &calls,
                    &clock,
                    &user,
                    &flags,
                    &output,
                )
            }
        })
        .await;
    if let Err(failure) = installed {
        drop(watcher);
        drop(context);
        drop(runtime);
        return failure_outcome(failure);
    }
    // Libs are loaded first. The user's stack cap applies to the program, so a
    // tight cap still leaves room to install the runtime's own helpers.
    runtime.set_max_stack_size(request.limits.stack_bytes).await;

    let source = request.source.clone();
    let js_clock = Arc::clone(&clock);
    let js_user = user.clone();
    let js_flags = Arc::clone(&flags);
    let js_output = Rc::clone(&output);
    let end = context
        .async_with(async move |ctx| {
            let execution = run_program(ctx.clone(), source, &js_output, output_limit);
            settle(
                select(Box::pin(execution), Box::pin(stop_rx)).await,
                &js_output,
                &js_flags,
                &js_clock,
                &js_user,
                output_limit,
            )
        })
        .await;
    // A host call returns a promise before its Rust future is polled. Drain the
    // queue so those calls are actually submitted, then wait or seal them.
    drive_queued(&runtime).await;

    let end = match end {
        ProgramEnd::Success(value) => match natural_wait(&calls, &clock, &user, grace) {
            None => ProgramEnd::Success(value),
            Some(failure) => ProgramEnd::Failure(failure),
        },
        ProgramEnd::Failure(failure) => {
            calls.seal(grace);
            ProgramEnd::Failure(failure)
        }
    };
    drop(watcher);
    drop(context);
    drop(runtime);

    let logs = output.borrow().logs().to_vec();
    let calls = calls.snapshot();
    match end {
        ProgramEnd::Success(value) => Outcome {
            logs,
            value: Some(value),
            failure: None,
            calls,
        },
        ProgramEnd::Failure(failure) => Outcome {
            logs,
            value: None,
            failure: Some(failure),
            calls,
        },
    }
}

/// Poll queued jobs until every host future has either started or parked.
///
/// This does not wait for host calls to finish. A parked oneshot wait returns
/// no progress and the loop stops.
async fn drive_queued(runtime: &AsyncRuntime) {
    for _ in 0..1024 {
        match runtime.execute_pending_job().await {
            Ok(true) => continue,
            _ => return,
        }
    }
}

fn spawn_watcher(
    handle: Handle,
    calls: Arc<Ledger>,
    clock: Arc<BudgetClock>,
    flags: Arc<Flags>,
    user: CancellationToken,
    stop_tx: tokio::sync::oneshot::Sender<Stop>,
) -> tokio::task::JoinHandle<()> {
    handle.spawn(async move {
        let mut stop_tx = Some(stop_tx);
        loop {
            let stop = if user.is_cancelled() {
                Some(Stop::Cancel)
            } else if flags.output.load(Ordering::Relaxed) {
                Some(Stop::Output)
            } else if clock.exhausted() {
                Some(Stop::Timeout)
            } else {
                None
            };
            if let Some(stop) = stop {
                calls.cancel_open();
                if let Some(tx) = stop_tx.take() {
                    let _ = tx.send(stop);
                }
                return;
            }
            let slice = clock
                .remaining()
                .clamp(Duration::from_millis(5), Duration::from_millis(20));
            tokio::select! {
                _ = tokio::time::sleep(slice) => {}
                _ = user.cancelled() => {}
            }
        }
    })
}

fn natural_wait(
    calls: &Ledger,
    clock: &BudgetClock,
    user: &CancellationToken,
    grace: Duration,
) -> Option<Failure> {
    loop {
        if user.is_cancelled() {
            calls.seal(grace);
            return Some(Failure::cancelled());
        }
        if clock.exhausted() {
            calls.seal(grace);
            return Some(Failure::timeout());
        }
        if calls.in_flight() == 0 {
            return None;
        }
        thread::park_timeout(Duration::from_millis(5));
    }
}

#[allow(clippy::too_many_arguments)]
fn settle<Left, Right>(
    raced: Either<
        (ProgramEnd, Left),
        (Result<Stop, tokio::sync::oneshot::error::RecvError>, Right),
    >,
    output: &RefCell<OutputLedger>,
    flags: &Flags,
    clock: &BudgetClock,
    user: &CancellationToken,
    output_limit: usize,
) -> ProgramEnd {
    let end = match raced {
        Either::Left((end, _)) => end,
        Either::Right((Ok(Stop::Cancel), _)) => ProgramEnd::Failure(Failure::cancelled()),
        Either::Right((Ok(Stop::Timeout), _)) => ProgramEnd::Failure(Failure::timeout()),
        Either::Right((Ok(Stop::Output), _)) => ProgramEnd::Failure(Failure::output(output_limit)),
        Either::Right((Err(_), _)) => {
            ProgramEnd::Failure(Failure::internal("budget watcher stopped"))
        }
    };
    if output.borrow().tripped() || flags.output.load(Ordering::Relaxed) {
        return ProgramEnd::Failure(Failure::output(output_limit));
    }
    if user.is_cancelled() {
        return ProgramEnd::Failure(Failure::cancelled());
    }
    if matches!(end, ProgramEnd::Success(_)) && clock.exhausted() {
        return ProgramEnd::Failure(Failure::timeout());
    }
    end
}

struct Shared {
    handle: Handle,
    calls: Arc<Ledger>,
    clock: Arc<BudgetClock>,
    user: CancellationToken,
    flags: Arc<Flags>,
}

#[allow(clippy::too_many_arguments)]
fn install(
    ctx: &Ctx<'_>,
    namespaces: &[super::Namespace],
    handle: &Handle,
    calls: &Arc<Ledger>,
    clock: &Arc<BudgetClock>,
    user: &CancellationToken,
    flags: &Arc<Flags>,
    output: &Rc<RefCell<OutputLedger>>,
) -> Result<(), Failure> {
    install_libs(ctx)?;
    install_console(ctx, output, flags)?;
    let shared = Shared {
        handle: handle.clone(),
        calls: Arc::clone(calls),
        clock: Arc::clone(clock),
        user: user.clone(),
        flags: Arc::clone(flags),
    };
    for namespace in namespaces {
        install_namespace(ctx, namespace, &shared)?;
    }
    Ok(())
}

fn install_libs(ctx: &Ctx<'_>) -> Result<(), Failure> {
    let source = format!(
        "(function() {{\n{}\nreturn makeLibs();\n}})()",
        include_str!("libs.js")
    );
    let libs = eval_object(ctx, &source)?;
    define_const(ctx, "libs", libs)
}

fn install_console(
    ctx: &Ctx<'_>,
    output: &Rc<RefCell<OutputLedger>>,
    flags: &Arc<Flags>,
) -> Result<(), Failure> {
    let console = Object::new(ctx.clone()).map_err(|error| Failure::internal(error.to_string()))?;
    for (name, level) in [
        ("log", LogLevel::Log),
        ("info", LogLevel::Info),
        ("warn", LogLevel::Warn),
        ("error", LogLevel::Error),
        ("debug", LogLevel::Debug),
    ] {
        let output = Rc::clone(output);
        let flags = Arc::clone(flags);
        let function = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<Value<'_>>| -> rquickjs::Result<()> {
                let message = format_console(&args.0);
                let accepted = output.borrow_mut().push(level, message);
                if !accepted {
                    flags.output.store(true, Ordering::Release);
                    return Err(boundary::uncatchable(&ctx, "output limit"));
                }
                Ok(())
            },
        )
        .and_then(|function| function.with_name(name))
        .map_err(|error| Failure::internal(error.to_string()))?;
        console
            .set(name, function)
            .map_err(|error| Failure::internal(error.to_string()))?;
    }
    define_const(ctx, "console", console)
}

fn install_namespace(
    ctx: &Ctx<'_>,
    namespace: &super::Namespace,
    shared: &Shared,
) -> Result<(), Failure> {
    let object = Object::new(ctx.clone()).map_err(|error| Failure::internal(error.to_string()))?;
    for binding in &namespace.functions {
        let function = bind_host(
            ctx,
            binding.function.clone(),
            shared,
            namespace.name.clone(),
            binding.name.clone(),
        )?;
        object
            .set(&binding.name, function)
            .map_err(|error| Failure::internal(error.to_string()))?;
    }
    define_const(ctx, &namespace.name, object)
}

fn bind_host<'js>(
    ctx: &Ctx<'js>,
    host: HostFn,
    shared: &Shared,
    namespace: String,
    name: String,
) -> Result<Function<'js>, Failure> {
    let handle = shared.handle.clone();
    let calls = Arc::clone(&shared.calls);
    let clock = Arc::clone(&shared.clock);
    let user = shared.user.clone();
    let flags = Arc::clone(&shared.flags);
    let label = name.clone();
    Function::new(
        ctx.clone(),
        Async(move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            call_host(
                ctx,
                args,
                host.clone(),
                handle.clone(),
                Arc::clone(&calls),
                Arc::clone(&clock),
                user.clone(),
                Arc::clone(&flags),
                namespace.clone(),
                name.clone(),
            )
        }),
    )
    .and_then(|function| function.with_name(label.as_str()))
    .map_err(|error| Failure::internal(error.to_string()))
}

#[allow(clippy::too_many_arguments)]
async fn call_host<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
    host: HostFn,
    handle: Handle,
    calls: Arc<Ledger>,
    clock: Arc<BudgetClock>,
    user: CancellationToken,
    flags: Arc<Flags>,
    namespace: String,
    name: String,
) -> rquickjs::Result<Value<'js>> {
    invoke(
        ctx, args.0, host, handle, calls, clock, user, flags, namespace, name,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn invoke<'js>(
    ctx: Ctx<'js>,
    args: Vec<Value<'js>>,
    host: HostFn,
    handle: Handle,
    calls: Arc<Ledger>,
    clock: Arc<BudgetClock>,
    user: CancellationToken,
    flags: Arc<Flags>,
    namespace: String,
    name: String,
) -> rquickjs::Result<Value<'js>> {
    let json_args = match args.as_slice() {
        [] => serde_json::Value::Null,
        [one] => match boundary::from_js(one) {
            Ok(json) => json,
            Err(BoundaryError::Illegal(message)) => {
                return Err(boundary::throw_type(&ctx, &message));
            }
            Err(BoundaryError::Thrown(thrown)) => {
                return Err(boundary::throw_message(&ctx, &thrown.message));
            }
        },
        _ => {
            return Err(boundary::throw_type(
                &ctx,
                "host functions take one JSON argument",
            ));
        }
    };

    let Some(ticket) = calls.begin(&namespace, &name, clock.wall_elapsed()) else {
        return Err(boundary::throw_type(&ctx, "in-flight call limit exceeded"));
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let token = ticket.token.clone();
    let pause = BudgetClock::handle(&clock);
    let finish_calls = Arc::clone(&calls);
    let finish_clock = Arc::clone(&clock);
    let idx = ticket.idx;
    handle.spawn(async move {
        let result = AssertUnwindSafe(host.call(json_args, token, pause))
            .catch_unwind()
            .await;
        let mapped = match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(error.message),
            Err(_) => Err("host function panicked".into()),
        };
        finish_calls.finish(idx, finish_clock.wall_elapsed());
        let _ = tx.send(mapped);
    });

    let received = rx.await;
    if user.is_cancelled() || flags.output.load(Ordering::Relaxed) || clock.exhausted() {
        return Err(boundary::uncatchable(&ctx, "stopped"));
    }
    match received {
        Ok(Ok(value)) => match boundary::to_js(&ctx, &value) {
            Ok(js) => Ok(js),
            Err(message) => Err(boundary::throw_type(&ctx, &message)),
        },
        Ok(Err(message)) => Err(boundary::throw_message(&ctx, &message)),
        Err(_) => Err(boundary::throw_message(
            &ctx,
            "host call ended without a result",
        )),
    }
}

async fn run_program(
    ctx: Ctx<'_>,
    source: String,
    output: &Rc<RefCell<OutputLedger>>,
    output_limit: usize,
) -> ProgramEnd {
    let wrapped = format!("(async () => {{\n{source}\n}})()\n");
    let mut options = EvalOptions::default();
    options.strict = false;
    options.filename = Some("program.js".into());
    let promise = match ctx.eval_with_options::<Promise, _>(wrapped, options) {
        Ok(promise) => promise,
        Err(error) => return classify(&ctx, error, true),
    };
    match promise.into_future::<Value<'_>>().await {
        Ok(value) => accept_value(&value, output.as_ref(), output_limit),
        Err(error) => classify(&ctx, error, false),
    }
}

fn accept_value(
    value: &Value<'_>,
    output: &RefCell<OutputLedger>,
    output_limit: usize,
) -> ProgramEnd {
    match boundary::from_js(value) {
        Err(BoundaryError::Illegal(message)) => ProgramEnd::Failure(Failure::illegal(message)),
        Err(BoundaryError::Thrown(thrown)) => {
            ProgramEnd::Failure(Failure::exception(thrown.message, thrown.stack))
        }
        Ok(json) => match serde_json::to_vec(&json) {
            Err(error) => ProgramEnd::Failure(Failure::internal(error.to_string())),
            Ok(bytes) => {
                if !output.borrow_mut().try_value(bytes.len()) {
                    return ProgramEnd::Failure(Failure::output(output_limit));
                }
                ProgramEnd::Success(json)
            }
        },
    }
}

fn classify(ctx: &Ctx<'_>, error: Error, from_eval: bool) -> ProgramEnd {
    let thrown = match error {
        Error::Exception => Some(boundary::take_exception(ctx)),
        other => return ProgramEnd::Failure(Failure::internal(other.to_string())),
    };
    let Some(thrown) = thrown else {
        return ProgramEnd::Failure(Failure::internal("exception"));
    };
    if from_eval && thrown.name.as_deref() == Some("SyntaxError") {
        return ProgramEnd::Failure(Failure::syntax(thrown.message));
    }
    if is_oom(&thrown.message) {
        return ProgramEnd::Failure(Failure::resource(
            ResourceKind::Memory,
            thrown.message,
            thrown.stack,
        ));
    }
    if is_stack(&thrown.message) {
        return ProgramEnd::Failure(Failure::resource(
            ResourceKind::Stack,
            thrown.message,
            thrown.stack,
        ));
    }
    if thrown.message == "interrupted" {
        return ProgramEnd::Failure(Failure::timeout());
    }
    ProgramEnd::Failure(Failure::exception(thrown.message, thrown.stack))
}

fn is_oom(message: &str) -> bool {
    message.contains("out of memory")
}

fn is_stack(message: &str) -> bool {
    message.contains("Maximum call stack size exceeded")
}

fn eval_object<'js>(ctx: &Ctx<'js>, source: &str) -> Result<Object<'js>, Failure> {
    match ctx.eval::<Object<'_>, _>(source) {
        Ok(object) => Ok(object),
        Err(Error::Exception) => {
            let thrown = boundary::take_exception(ctx);
            Err(Failure::internal(thrown.message))
        }
        Err(error) => Err(Failure::internal(error.to_string())),
    }
}

fn define_const<'js>(ctx: &Ctx<'js>, name: &str, value: Object<'js>) -> Result<(), Failure> {
    ctx.globals()
        .set("__litecode_slot", value)
        .map_err(|error| Failure::internal(error.to_string()))?;
    match ctx.eval::<(), _>("Object.freeze(__litecode_slot)") {
        Ok(()) => {}
        Err(Error::Exception) => {
            let thrown = boundary::take_exception(ctx);
            return Err(Failure::internal(thrown.message));
        }
        Err(error) => return Err(Failure::internal(error.to_string())),
    }
    let source = format!(
        "Object.defineProperty(globalThis, '{name}', {{ value: __litecode_slot, writable: false, configurable: false, enumerable: true }}); delete globalThis.__litecode_slot;"
    );
    match ctx.eval::<(), _>(source) {
        Ok(()) => Ok(()),
        Err(Error::Exception) => {
            let thrown = boundary::take_exception(ctx);
            Err(Failure::internal(thrown.message))
        }
        Err(error) => Err(Failure::internal(error.to_string())),
    }
}

fn format_console(args: &[Value<'_>]) -> String {
    args.iter()
        .map(|value| {
            if value.is_string() {
                value.get::<String>().unwrap_or_else(|_| "[string]".into())
            } else {
                format_debug(value, 0, &mut Vec::new())
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_debug(value: &Value<'_>, depth: usize, stack: &mut Vec<usize>) -> String {
    if value.is_null() {
        return "null".into();
    }
    if value.is_undefined() {
        return "undefined".into();
    }
    if let Some(bool) = value.as_bool() {
        return bool.to_string();
    }
    if let Some(number) = value.as_number() {
        return number.to_string();
    }
    if value.is_string() {
        return value.get::<String>().unwrap_or_else(|_| "[string]".into());
    }
    if value.is_big_int() {
        return "[BigInt]".into();
    }
    if value.is_symbol() {
        return "[Symbol]".into();
    }
    if value.is_function() {
        return "[Function]".into();
    }
    if !value.is_object() || depth >= 4 {
        return "[Object]".into();
    }
    let ptr = unsafe { rquickjs::qjs::JS_VALUE_GET_PTR(value.as_raw()) as usize };
    if stack.contains(&ptr) {
        return "[Circular]".into();
    }
    stack.push(ptr);
    let text = if let Some(array) = value.as_array() {
        let mut parts = Vec::new();
        let len = array.len().min(20);
        for index in 0..len {
            let element = array.get::<Value<'_>>(index).ok();
            parts.push(match element {
                Some(element) => format_debug(&element, depth + 1, stack),
                None => "[?]".into(),
            });
        }
        format!("[{}]", parts.join(", "))
    } else if let Some(object) = value.as_object() {
        let mut parts = Vec::new();
        for (key, child) in object.props::<String, Value<'_>>().take(20).flatten() {
            parts.push(format!("{key}: {}", format_debug(&child, depth + 1, stack)));
        }
        format!("{{{}}}", parts.join(", "))
    } else {
        "[Object]".into()
    };
    stack.pop();
    text
}

fn failure_outcome(failure: Failure) -> Outcome {
    Outcome {
        logs: Vec::new(),
        value: None,
        failure: Some(failure),
        calls: Vec::new(),
    }
}
