//! QuickJS-NG driver. The context lives on this thread; host futures are
//! spawned onto the caller's tokio runtime and are not aborted when the
//! program stops.

use std::cell::RefCell;
use std::ffi::c_void;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::Duration;

use std::panic::AssertUnwindSafe;

use futures_util::FutureExt;
use futures_util::future::{Either, select};
use rquickjs::context::EvalOptions;
use rquickjs::prelude::{Async, Rest};
use rquickjs::{
    AsyncContext, AsyncRuntime, Ctx, Error, Exception, Function, Object, Persistent, Promise, Value,
};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use super::boundary::{self, BoundaryError};
use super::budget::BudgetClock;
use super::calls::Ledger;
use super::heap::LimitAlloc;
use super::output::OutputLedger;
use super::{ExecuteRequest, Failure, FailureKind, HostFn, Json, LogLevel, Outcome, ResourceKind};

/// Jobs executed while draining the queue after the program settles.
///
/// Hitting the ceiling fails the run. A lower silent cap used to drop a host
/// call that sat behind a few thousand microtasks and still report success.
const MAX_DRAIN_JOBS: usize = 1_000_000;

struct Faults {
    interrupt: AtomicU8,
    memory: Arc<AtomicBool>,
    /// Set while the JS thread's stack pointer is below the armed limit.
    /// Cleared again once execution returns above that limit.
    stack: AtomicBool,
    /// `0` means the stack watch is not armed.
    stack_limit: AtomicUsize,
    /// Raw `JSContext*`, valid only while that context is alive.
    ctx: Arc<AtomicPtr<c_void>>,
    /// Byte offset of `JSContext.interrupt_counter`. `0` means unknown.
    counter_offset: Arc<AtomicUsize>,
}

impl Faults {
    const NONE: u8 = 0;
    const CANCEL: u8 = 1;
    const TIMEOUT: u8 = 2;
    const OUTPUT: u8 = 3;
    const STACK: u8 = 4;
    const MEMORY: u8 = 5;

    fn new() -> Arc<Self> {
        Arc::new(Self {
            interrupt: AtomicU8::new(Self::NONE),
            memory: Arc::new(AtomicBool::new(false)),
            stack: AtomicBool::new(false),
            stack_limit: AtomicUsize::new(0),
            ctx: Arc::new(AtomicPtr::new(std::ptr::null_mut())),
            counter_offset: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn record(&self, kind: u8) {
        self.interrupt.store(kind, Ordering::Release);
    }

    fn interrupt(&self) -> u8 {
        self.interrupt.load(Ordering::Acquire)
    }

    fn memory_fault(&self) -> bool {
        self.memory.load(Ordering::Acquire)
    }

    fn stack_fault(&self) -> bool {
        self.stack.load(Ordering::Acquire)
    }

    fn disarm(&self) {
        self.ctx.store(std::ptr::null_mut(), Ordering::Release);
        self.counter_offset.store(0, Ordering::Release);
        self.stack_limit.store(0, Ordering::Release);
        self.stack.store(false, Ordering::Release);
    }
}

struct Protos {
    internal: Persistent<Object<'static>>,
    range: Persistent<Object<'static>>,
    syntax: Persistent<Object<'static>>,
    type_error: Persistent<Object<'static>>,
    tool: Persistent<Object<'static>>,
}

struct Flags {
    output: AtomicBool,
}

enum Stop {
    Cancel,
    Timeout,
    Output,
}

enum ProgramEnd {
    Success(Json),
    /// The user cancelled after the program had already returned.
    CancelledAfterReturn(Json),
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
    let stack_bytes = request.limits.stack_bytes;
    let protos: Rc<RefCell<Option<Protos>>> = Rc::new(RefCell::new(None));

    let faults = Faults::new();
    let runtime = match AsyncRuntime::new_with_alloc(LimitAlloc::new(
        request.limits.memory_bytes,
        Arc::clone(&faults.memory),
        Arc::clone(&faults.ctx),
        Arc::clone(&faults.counter_offset),
    )) {
        Ok(runtime) => runtime,
        Err(error) => return Outcome::from_failure(Failure::internal(error.to_string())),
    };
    {
        let user = user.clone();
        let flags = Arc::clone(&flags);
        let clock = Arc::clone(&clock);
        let faults = Arc::clone(&faults);
        runtime
            .set_interrupt_handler(Some(Box::new(move || {
                note_stack(&faults);
                if user.is_cancelled() {
                    faults.record(Faults::CANCEL);
                    return true;
                }
                if flags.output.load(Ordering::Relaxed) {
                    faults.record(Faults::OUTPUT);
                    return true;
                }
                if clock.should_interrupt() {
                    faults.record(Faults::TIMEOUT);
                    return true;
                }
                // QuickJS's own stack overflow is a catchable RangeError. Abort
                // here so the program cannot catch it. The same interrupt that
                // notices the native stack is past the limit throws. The flag
                // is latched, so a later poll after a caught native backstop
                // throw still seals the run.
                if faults.stack_fault() {
                    faults.record(Faults::STACK);
                    return true;
                }
                // A refused heap allocation sets the fault in the allocator and
                // asks for this check on the next opcode. Returning true marks
                // the stop uncatchable, same as the stack seal.
                if faults.memory_fault() {
                    faults.record(Faults::MEMORY);
                    return true;
                }
                false
            })))
            .await;
    }

    let context = match AsyncContext::full(&runtime).await {
        Ok(context) => context,
        Err(error) => return Outcome::from_failure(Failure::internal(error.to_string())),
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
            let faults = Arc::clone(&faults);
            let output = Rc::clone(&output);
            let protos = Rc::clone(&protos);
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
                    &faults,
                    &output,
                )?;
                let saved = save_prototypes(&ctx)?;
                *protos.borrow_mut() = Some(saved);
                Ok(())
            }
        })
        .await;
    if let Err(failure) = installed {
        drop(watcher);
        drop(protos);
        drop(context);
        drop(runtime);
        return Outcome::from_failure(failure);
    }
    // Libs are loaded first. The user's stack cap applies to the program, so a
    // tight cap still leaves room to install the runtime's own helpers.
    // QuickJS's native check is only a backstop: it sits `stack_bytes + headroom`
    // deep so the uncatchable stack watch (armed at `stack_bytes`) always seals
    // first. The native check throws a plain, catchable RangeError.
    runtime
        .set_max_stack_size(native_stack_bytes(request.limits.stack_bytes))
        .await;

    let source = request.source.clone();
    let js_clock = Arc::clone(&clock);
    let js_user = user.clone();
    let js_flags = Arc::clone(&flags);
    let js_faults = Arc::clone(&faults);
    let js_output = Rc::clone(&output);
    let js_protos = Rc::clone(&protos);
    let end = context
        .async_with(async move |ctx| {
            let execution = run_program(
                ctx.clone(),
                source,
                &js_output,
                output_limit,
                stack_bytes,
                &js_faults,
                &js_flags,
                &js_clock,
                &js_user,
                &js_protos,
            );
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
    // A host call returns a promise before its Rust future is polled. Drain
    // every queued job (budget, cancellation, and the output cap still apply)
    // so a call behind a long microtask chain is submitted before we seal.
    let drained = drive_queued(
        &runtime,
        &clock,
        &user,
        &flags,
        &faults,
        &protos,
        output_limit,
    )
    .await;
    let end = reconcile_drain(end, drained, &output);
    let pending = calls.seal(grace);
    let end = match end {
        ProgramEnd::Success(value) if pending > 0 => {
            let note = format!(
                "{pending} host calls were still running when the program returned and have been cancelled"
            );
            if output.borrow_mut().push(LogLevel::Warn, note) {
                ProgramEnd::Success(value)
            } else {
                ProgramEnd::Failure(Failure::output(output_limit))
            }
        }
        other => other,
    };
    // Final seal: the latch is set only by `note_stack` when the real stack
    // pointer crossed the watch limit (never by a message). A program that
    // caught the overflow and returned without reaching another interrupt
    // point is still a stack failure, not a success or plain exception.
    let end = seal_latched_stack(end, &faults);
    faults.disarm();
    drop(watcher);
    drop(protos);
    drop(context);
    drop(runtime);

    let logs = output.borrow().logs().to_vec();
    let dropped_undefined = output.borrow().dropped_undefined().clone();
    let calls = calls.snapshot();
    let duration_ms = clock.wall_elapsed().as_millis() as u64;
    match end {
        ProgramEnd::Success(value) => Outcome {
            logs,
            value: Some(value),
            failure: None,
            calls,
            duration_ms,
            dropped_undefined: dropped_undefined.clone(),
        },
        ProgramEnd::CancelledAfterReturn(value) => Outcome {
            logs,
            value: Some(value),
            failure: Some(Failure::cancelled()),
            calls,
            duration_ms,
            dropped_undefined: dropped_undefined.clone(),
        },
        ProgramEnd::Failure(failure) => Outcome {
            logs,
            value: None,
            failure: Some(failure),
            calls,
            duration_ms,
            dropped_undefined: dropped_undefined.clone(),
        },
    }
}

/// Poll queued jobs until every host future has either started or parked.
///
/// This does not wait for host calls to finish. A parked oneshot wait returns
/// no progress and the loop stops. `Some` is an engine stop (budget, cancel,
/// output, memory, stack) or the explicit drain ceiling — never a success that
/// left jobs behind.
async fn drive_queued(
    runtime: &AsyncRuntime,
    clock: &BudgetClock,
    user: &CancellationToken,
    flags: &Flags,
    faults: &Faults,
    protos: &RefCell<Option<Protos>>,
    output_limit: usize,
) -> Option<Failure> {
    let mut steps = 0usize;
    loop {
        if let Some(failure) = engine_stop(user, flags, clock, faults, output_limit) {
            return Some(failure);
        }
        if steps >= MAX_DRAIN_JOBS {
            return Some(Failure::internal("pending job limit exceeded"));
        }
        match runtime.execute_pending_job().await {
            Ok(true) => steps += 1,
            Ok(false) => return None,
            Err(error) => {
                steps += 1;
                let failure = error
                    .0
                    .with(|ctx| {
                        let value = ctx.catch();
                        classify_value(
                            &ctx,
                            &value,
                            false,
                            output_limit,
                            faults,
                            flags,
                            clock,
                            user,
                            protos,
                            0,
                        )
                    })
                    .await;
                if let ProgramEnd::Failure(failure) = failure
                    && stops_the_run(failure.kind)
                {
                    return Some(failure);
                }
            }
        }
    }
}

/// A program that already returned keeps its value when the drain then stops
/// for budget or an internal fault. User cancel keeps the value and records
/// cancellation. Any other drain failure replaces the success.
fn reconcile_drain(
    end: ProgramEnd,
    drained: Option<Failure>,
    output: &RefCell<OutputLedger>,
) -> ProgramEnd {
    match (end, drained) {
        (ProgramEnd::Success(value), Some(failure)) => match failure.kind {
            FailureKind::Cancelled => ProgramEnd::CancelledAfterReturn(value),
            FailureKind::Timeout | FailureKind::Internal => {
                let note = match failure.kind {
                    FailureKind::Timeout => {
                        "program budget expired while draining host calls after the program returned"
                    }
                    _ => "host drain stopped after the program returned",
                };
                let _ = output.borrow_mut().push(LogLevel::Warn, note.to_string());
                ProgramEnd::Success(value)
            }
            _ => ProgramEnd::Failure(failure),
        },
        (end, _) => end,
    }
}

fn engine_stop(
    user: &CancellationToken,
    flags: &Flags,
    clock: &BudgetClock,
    faults: &Faults,
    output_limit: usize,
) -> Option<Failure> {
    if user.is_cancelled() {
        faults.record(Faults::CANCEL);
        return Some(Failure::cancelled());
    }
    if flags.output.load(Ordering::Relaxed) {
        faults.record(Faults::OUTPUT);
        return Some(Failure::output(output_limit));
    }
    if clock.exhausted() {
        faults.record(Faults::TIMEOUT);
        return Some(Failure::timeout());
    }
    None
}

fn stops_the_run(kind: FailureKind) -> bool {
    matches!(
        kind,
        FailureKind::Timeout
            | FailureKind::Cancelled
            | FailureKind::ResourceLimit
            | FailureKind::OutputLimit
            | FailureKind::Internal
    )
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
    if let ProgramEnd::Success(value) = end {
        // The program already returned. Budget expiry must not rewrite that
        // into a timeout. Only a user cancel keeps the value and records failure.
        if user.is_cancelled() {
            return ProgramEnd::CancelledAfterReturn(value);
        }
        return ProgramEnd::Success(value);
    }
    if output.borrow().tripped() || flags.output.load(Ordering::Relaxed) {
        return ProgramEnd::Failure(Failure::output(output_limit));
    }
    if user.is_cancelled() {
        return ProgramEnd::Failure(Failure::cancelled());
    }
    let _ = clock;
    end
}

struct Shared {
    handle: Handle,
    calls: Arc<Ledger>,
    clock: Arc<BudgetClock>,
    user: CancellationToken,
    flags: Arc<Flags>,
    faults: Arc<Faults>,
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
    faults: &Arc<Faults>,
    output: &Rc<RefCell<OutputLedger>>,
) -> Result<(), Failure> {
    install_tool_error(ctx)?;
    install_libs(ctx)?;
    install_console(ctx, output, flags, faults)?;
    let shared = Shared {
        handle: handle.clone(),
        calls: Arc::clone(calls),
        clock: Arc::clone(clock),
        user: user.clone(),
        flags: Arc::clone(flags),
        faults: Arc::clone(faults),
    };
    for namespace in namespaces {
        install_namespace(ctx, namespace, &shared)?;
    }
    hide_blocking_intrinsics(ctx)
}

fn install_tool_error(ctx: &Ctx<'_>) -> Result<(), Failure> {
    const SOURCE: &str = r#"
        (function () {
            class ToolError extends Error {
                constructor(message) {
                    super(message);
                    Object.defineProperty(this, "name", {
                        value: "ToolError",
                        writable: false,
                        enumerable: false,
                        configurable: false
                    });
                }
            }
            Object.defineProperty(globalThis, "ToolError", {
                value: ToolError,
                writable: false,
                enumerable: false,
                configurable: false
            });
        })()
    "#;
    match ctx.eval::<(), _>(SOURCE) {
        Ok(()) => Ok(()),
        Err(Error::Exception) => Err(Failure::internal(boundary::take_exception(ctx).message)),
        Err(error) => Err(Failure::internal(error.to_string())),
    }
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
    faults: &Arc<Faults>,
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
        let faults = Arc::clone(faults);
        let function = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<Value<'_>>| -> rquickjs::Result<()> {
                let message = format_console(&args.0);
                let accepted = output.borrow_mut().push(level, message);
                if !accepted {
                    flags.output.store(true, Ordering::Release);
                    faults.record(Faults::OUTPUT);
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
    let faults = Arc::clone(&shared.faults);
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
                Arc::clone(&faults),
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
    faults: Arc<Faults>,
    namespace: String,
    name: String,
) -> rquickjs::Result<Value<'js>> {
    invoke(
        ctx, args.0, host, handle, calls, clock, user, flags, faults, namespace, name,
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
    faults: Arc<Faults>,
    namespace: String,
    name: String,
) -> rquickjs::Result<Value<'js>> {
    let json_args = match args.as_slice() {
        [] => Json::new(serde_json::Value::Null),
        [one] => match boundary::from_js_with(
            one,
            boundary::UndefinedPolicy::Strip,
            &mut boundary::DroppedUndefined::default(),
        ) {
            Ok(json) => json,
            Err(BoundaryError::Illegal(message)) => {
                return Err(boundary::throw_type(&ctx, &message));
            }
            Err(BoundaryError::Thrown(thrown)) => {
                return Err(boundary::throw_message(&ctx, &thrown.message));
            }
        },
        many => {
            // Extra arguments stay generic: the host receives one JSON array,
            // in call order. A single argument is still that value alone.
            let mut items = Vec::with_capacity(many.len());
            for arg in many {
                items.push(match boundary::from_js_with(
                    arg,
                    boundary::UndefinedPolicy::Strip,
                    &mut boundary::DroppedUndefined::default(),
                ) {
                    Ok(json) => json,
                    Err(BoundaryError::Illegal(message)) => {
                        return Err(boundary::throw_type(&ctx, &message));
                    }
                    Err(BoundaryError::Thrown(thrown)) => {
                        return Err(boundary::throw_message(&ctx, &thrown.message));
                    }
                });
            }
            Json::from_array(items)
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
    let seq = ticket.seq;
    let remaining = clock.remaining();
    handle.spawn(async move {
        let info = super::CallInfo {
            seq: ticket.seq,
            remaining,
        };
        let result = AssertUnwindSafe(host.call(json_args, token, pause, info))
            .catch_unwind()
            .await;
        let mapped = match result {
            Ok(Ok(value)) => Ok(Json::new(value)),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(super::HostError::new("host function panicked")),
        };
        finish_calls.finish(idx, finish_clock.wall_elapsed());
        let _ = tx.send(mapped);
    });

    let received = rx.await;
    if user.is_cancelled() {
        faults.record(Faults::CANCEL);
        return Err(boundary::uncatchable(&ctx, "stopped"));
    }
    if flags.output.load(Ordering::Relaxed) {
        faults.record(Faults::OUTPUT);
        return Err(boundary::uncatchable(&ctx, "stopped"));
    }
    if clock.exhausted() {
        faults.record(Faults::TIMEOUT);
        return Err(boundary::uncatchable(&ctx, "stopped"));
    }
    match received {
        Ok(Ok(value)) => match boundary::to_js(&ctx, value) {
            Ok(js) => Ok(js),
            Err(message) => Err(boundary::throw_type(&ctx, &message)),
        },
        Ok(Err(error)) => Err(raise_host_error(&ctx, error, &name, seq)),
        Err(_) => Err(boundary::throw_message(
            &ctx,
            "host call ended without a result",
        )),
    }
}

fn raise_host_error(ctx: &Ctx<'_>, error: super::HostError, tool: &str, seq: u64) -> Error {
    match error.kind {
        super::HostErrorKind::Type => boundary::throw_type(ctx, &error.message),
        super::HostErrorKind::Error => boundary::throw_message(ctx, &error.message),
        super::HostErrorKind::Tool {
            code,
            details,
            retryable,
        } => throw_tool_error(ctx, &error.message, &code, details.as_ref(), retryable, tool, seq),
    }
}

fn throw_tool_error(
    ctx: &Ctx<'_>,
    message: &str,
    code: &str,
    details: Option<&serde_json::Value>,
    retryable: bool,
    tool: &str,
    seq: u64,
) -> Error {
    const FACTORY: &str = r#"
        (function (message, code, details, retryable, tool, seq) {
            const error = new ToolError(message);
            const define = (key, value) => Object.defineProperty(error, key, {
                value: value,
                writable: false,
                enumerable: true,
                configurable: false
            });
            define("code", code);
            define("tool", tool);
            define("seq", seq);
            if (details !== undefined && details !== null) define("details", details);
            if (retryable === true) define("retryable", true);
            return error;
        })
    "#;
    let factory = match ctx.eval::<Function<'_>, _>(FACTORY) {
        Ok(factory) => factory,
        Err(error) => return error,
    };
    let details = match details {
        Some(value) => match boundary::to_js(ctx, Json::new(value.clone())) {
            Ok(js) => js,
            Err(message) => return boundary::throw_type(ctx, &message),
        },
        None => Value::new_undefined(ctx.clone()),
    };
    match factory.call::<_, Object<'_>>((message, code, details, retryable, tool, seq as f64)) {
        Ok(error) => Exception::from_object(error)
            .map(Exception::throw)
            .unwrap_or_else(|| boundary::throw_message(ctx, message)),
        Err(error) => error,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_program(
    ctx: Ctx<'_>,
    source: String,
    output: &Rc<RefCell<OutputLedger>>,
    output_limit: usize,
    stack_bytes: usize,
    faults: &Faults,
    flags: &Flags,
    clock: &BudgetClock,
    user: &CancellationToken,
    protos: &RefCell<Option<Protos>>,
) -> ProgramEnd {
    if let Err(failure) = arm_stack_watch(&ctx, faults, stack_bytes) {
        return ProgramEnd::Failure(failure);
    }
    let source_lines = source.lines().count() as u32;
    let wrapped = format!("(async () => {{\n{source}\n}})()\n");
    let mut options = EvalOptions::default();
    options.strict = false;
    options.filename = Some("program.js".into());
    let promise = match ctx.eval_with_options::<Promise, _>(wrapped, options) {
        Ok(promise) => promise,
        Err(error) => {
            return classify(
                &ctx,
                error,
                true,
                output_limit,
                faults,
                flags,
                clock,
                user,
                protos,
                source_lines,
            );
        }
    };
    match promise.into_future::<Value<'_>>().await {
        Ok(value) => {
            // The stack watch may be disarmed, so the allocator cannot poke
            // the interrupt counter. A caught allocation failure still leaves
            // the fault set; that must not resolve as success.
            if faults.memory_fault() {
                return ProgramEnd::Failure(Failure::resource(
                    ResourceKind::Memory,
                    "out of memory",
                    None,
                ));
            }
            accept_value(&value, output.as_ref(), output_limit)
        }
        Err(error) => classify(
            &ctx,
            error,
            false,
            output_limit,
            faults,
            flags,
            clock,
            user,
            protos,
            source_lines,
        ),
    }
}

fn accept_value(
    value: &Value<'_>,
    output: &RefCell<OutputLedger>,
    output_limit: usize,
) -> ProgramEnd {
    let mut dropped = boundary::DroppedUndefined::default();
    let converted =
        boundary::from_js_with(value, boundary::UndefinedPolicy::StripOrNull, &mut dropped);
    output.borrow_mut().set_dropped_undefined(dropped);
    match converted {
        Err(BoundaryError::Illegal(message)) => ProgramEnd::Failure(Failure::illegal(message)),
        Err(BoundaryError::Thrown(thrown)) => {
            ProgramEnd::Failure(Failure::exception(thrown.message, thrown.stack))
        }
        Ok(json) => match boundary::json_bytes(json.as_value()) {
            Err(error) => ProgramEnd::Failure(Failure::internal(error)),
            Ok(bytes) => {
                if !output.borrow_mut().try_value(bytes.len()) {
                    return ProgramEnd::Failure(Failure::output(output_limit));
                }
                ProgramEnd::Success(json)
            }
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn classify(
    ctx: &Ctx<'_>,
    error: Error,
    from_eval: bool,
    output_limit: usize,
    faults: &Faults,
    flags: &Flags,
    clock: &BudgetClock,
    user: &CancellationToken,
    protos: &RefCell<Option<Protos>>,
    source_lines: u32,
) -> ProgramEnd {
    match error {
        Error::Exception => {
            let value = ctx.catch();
            classify_value(
                ctx,
                &value,
                from_eval,
                output_limit,
                faults,
                flags,
                clock,
                user,
                protos,
                source_lines,
            )
        }
        Error::Allocation => ProgramEnd::Failure(Failure::resource(
            ResourceKind::Memory,
            "out of memory",
            None,
        )),
        other => ProgramEnd::Failure(Failure::internal(other.to_string())),
    }
}

#[allow(clippy::too_many_arguments)]
fn classify_value<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    from_eval: bool,
    output_limit: usize,
    faults: &Faults,
    flags: &Flags,
    clock: &BudgetClock,
    user: &CancellationToken,
    protos: &RefCell<Option<Protos>>,
    source_lines: u32,
) -> ProgramEnd {
    if value.is_uncatchable_error() {
        let failure = match faults.interrupt() {
            Faults::CANCEL => Failure::cancelled(),
            Faults::OUTPUT => Failure::output(output_limit),
            Faults::TIMEOUT => Failure::timeout(),
            Faults::STACK => Failure::resource(ResourceKind::Stack, "stack overflow", None),
            Faults::MEMORY => Failure::resource(ResourceKind::Memory, "out of memory", None),
            _ if user.is_cancelled() => Failure::cancelled(),
            _ if flags.output.load(Ordering::Relaxed) => Failure::output(output_limit),
            _ if clock.exhausted() => Failure::timeout(),
            _ => Failure::internal("uncatchable stop"),
        };
        return ProgramEnd::Failure(failure);
    }

    let thrown = boundary::describe(value);
    let saved = protos.borrow();
    if faults.memory_fault()
        && (value.is_null()
            || value.is_undefined()
            || saved
                .as_ref()
                .is_some_and(|protos| proto_is(ctx, value, &protos.internal)))
    {
        let message = if value.is_null() || value.is_undefined() {
            "out of memory".to_string()
        } else {
            thrown.message
        };
        return ProgramEnd::Failure(Failure::resource(
            ResourceKind::Memory,
            message,
            thrown.stack,
        ));
    }
    if saved
        .as_ref()
        .is_some_and(|protos| proto_is(ctx, value, &protos.range))
        && faults.stack_fault()
    {
        return ProgramEnd::Failure(Failure::resource(
            ResourceKind::Stack,
            thrown.message,
            thrown.stack,
        ));
    }
    if from_eval
        && saved
            .as_ref()
            .is_some_and(|protos| proto_is(ctx, value, &protos.syntax))
    {
        return ProgramEnd::Failure(Failure::syntax(thrown.message, thrown.stack, source_lines));
    }
    if saved
        .as_ref()
        .is_some_and(|protos| proto_is(ctx, value, &protos.tool))
    {
        let mut failure = Failure::exception(thrown.message, thrown.stack);
        failure.tool_error = tool_error_info(ctx, value, saved.as_ref().unwrap());
        return ProgramEnd::Failure(failure);
    }
    if saved
        .as_ref()
        .is_some_and(|protos| proto_is(ctx, value, &protos.type_error))
    {
        let mut failure = Failure::exception(thrown.message, thrown.stack);
        failure.uncaught_type_error = true;
        return ProgramEnd::Failure(failure);
    }
    ProgramEnd::Failure(Failure::exception(thrown.message, thrown.stack))
}

fn tool_error_info<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    protos: &Protos,
) -> Option<super::ToolErrorInfo> {
    if !proto_is(ctx, value, &protos.tool) {
        return None;
    }
    let object = value.as_object()?;
    let code = object.get::<_, String>("code").unwrap_or_else(|_| "internal".into());
    let tool = object.get::<_, String>("tool").unwrap_or_default();
    let seq = object
        .get::<_, f64>("seq")
        .ok()
        .map(|number| number as u64)
        .unwrap_or(0);
    let details = object.get::<_, Value<'_>>("details").ok().and_then(|details| {
        if details.is_undefined() || details.is_null() {
            return None;
        }
        boundary::from_js(&details)
            .ok()
            .map(|json| json.into_value())
    });
    Some(super::ToolErrorInfo {
        code,
        tool,
        seq,
        details,
    })
}

fn proto_is<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    expected: &Persistent<Object<'static>>,
) -> bool {
    let Ok(expected) = expected.clone().restore(ctx) else {
        return false;
    };
    value
        .as_object()
        .and_then(|object| object.get_prototype())
        .is_some_and(|proto| proto == expected)
}

/// QuickJS throws a catchable `RangeError` for stack overflow and keeps no flag.
/// Sample the native stack from the interrupt handler. The same call that sees
/// the stack past the limit returns true, and QuickJS marks that interrupt
/// uncatchable, so `try/catch` cannot swallow the seal.
const STACK_NEAR_BYTES: usize = 48 * 1024;
const STACK_POLL_FAR: i32 = 32;
const STACK_POLL_NEAR: i32 = 1;
/// Bytes of `JSContext` that are known to contain `interrupt_counter` on the
/// pinned QuickJS-NG 0.14 layout (64-bit, no NaN boxing).
const CONTEXT_SCAN_BYTES: usize = 576;

/// Turn any end into `ResourceLimit/Stack` once the stack watch latched.
fn seal_latched_stack(end: ProgramEnd, faults: &Faults) -> ProgramEnd {
    if !faults.stack_fault() {
        return end;
    }
    match end {
        ProgramEnd::Failure(failure) if failure.resource == Some(ResourceKind::Stack) => {
            ProgramEnd::Failure(failure)
        }
        ProgramEnd::Failure(failure) => ProgramEnd::Failure(Failure::resource(
            ResourceKind::Stack,
            "stack overflow",
            failure.stack,
        )),
        _ => ProgramEnd::Failure(Failure::resource(ResourceKind::Stack, "stack overflow", None)),
    }
}

fn note_stack(faults: &Faults) {
    let limit = faults.stack_limit.load(Ordering::Relaxed);
    if limit == 0 {
        return;
    }
    let sp = stack_pointer();
    // Latch: the throw unwinds before we classify, and the promise job that
    // delivers it runs at a shallow stack and would otherwise clear the flag.
    if sp < limit {
        faults.stack.store(true, Ordering::Release);
    }
    // Once latched, poll on every check so a program that caught a native
    // backstop RangeError is stopped at its very next interrupt point.
    let latched = faults.stack.load(Ordering::Relaxed);
    let ctx = faults.ctx.load(Ordering::Relaxed);
    let offset = faults.counter_offset.load(Ordering::Relaxed);
    if ctx.is_null() || offset == 0 {
        return;
    }
    let next = if latched || sp.saturating_sub(limit) < STACK_NEAR_BYTES {
        STACK_POLL_NEAR
    } else {
        STACK_POLL_FAR
    };
    unsafe { write_counter(ctx, offset, next) }
}

fn arm_stack_watch(ctx: &Ctx<'_>, faults: &Faults, stack_bytes: usize) -> Result<(), Failure> {
    let stack_bytes = effective_stack_bytes(stack_bytes);
    if stack_bytes == 0 {
        faults.disarm();
        return Ok(());
    }
    let raw = js_context(ctx);
    unsafe {
        rquickjs::qjs::JS_UpdateStackTop(rquickjs::qjs::JS_GetRuntime(raw));
    }
    let base = stack_pointer();
    let limit = base.saturating_sub(stack_bytes);
    if limit == 0 {
        faults.disarm();
        return Ok(());
    }
    faults.stack_limit.store(limit, Ordering::Release);
    faults.ctx.store(raw.cast(), Ordering::Release);
    let Some(offset) = discover_counter_offset(ctx, raw) else {
        faults.disarm();
        return Err(Failure::internal(
            "stack watch could not find the interrupt counter",
        ));
    };
    faults.counter_offset.store(offset, Ordering::Release);
    unsafe { write_counter(raw.cast(), offset, STACK_POLL_NEAR) }
    Ok(())
}

/// Same ceiling as rquickjs: larger values disable the check.
const MAX_SANE_STACK: usize = 1 << 24;
/// Minimum distance between the watch limit and QuickJS's native limit. The
/// watch polls on every call once within [`STACK_NEAR_BYTES`], so a few frames
/// past the watch limit are enough; this leaves a wide margin for big frames
/// (native helpers, regexp, JSON) between two polls.
const STACK_NATIVE_HEADROOM_MIN: usize = 64 * 1024;
/// Thread stack beyond the native QuickJS limit: Rust/tokio frames below the
/// arm point, host glue, and C code QuickJS runs without a stack check.
pub(crate) const JS_THREAD_STACK_SAFETY: usize = 2 * 1024 * 1024;

/// QuickJS native max stack: `stack_bytes + max(64 KiB, stack_bytes / 2)`,
/// capped at the rquickjs ceiling. `0` (or an over-ceiling cap) keeps the
/// request unchanged, matching the disarmed watch.
pub(crate) fn native_stack_bytes(stack_bytes: usize) -> usize {
    let watch = effective_stack_bytes(stack_bytes);
    if watch == 0 {
        return stack_bytes;
    }
    let headroom = (watch / 2).max(STACK_NATIVE_HEADROOM_MIN);
    watch.saturating_add(headroom).min(MAX_SANE_STACK)
}

/// OS stack for the `ptc-js` thread: native limit + safety. `None` keeps the
/// platform default when no native limit is enforced.
pub(crate) fn js_thread_stack_bytes(stack_bytes: usize) -> Option<usize> {
    let native = effective_stack_bytes(native_stack_bytes(stack_bytes));
    (native != 0).then(|| native + JS_THREAD_STACK_SAFETY)
}

fn effective_stack_bytes(limit: usize) -> usize {
    if limit == 0 || limit > MAX_SANE_STACK {
        0
    } else {
        limit
    }
}

fn discover_counter_offset(ctx: &Ctx<'_>, raw: *mut rquickjs::qjs::JSContext) -> Option<usize> {
    // Burn the counter once so the next window starts from a fresh 10000,
    // then see which i32 drops by about one short loop. A wrap (the counter
    // is reloaded to 10000 inside the probe) is counted too.
    eval_discard(ctx, "for (let i = 0; i < 20000; i++);")?;
    let before = snapshot_ints(raw);
    eval_discard(ctx, "for (let i = 0; i < 800; i++);")?;
    let mut found = None;
    for offset in (0..CONTEXT_SCAN_BYTES).step_by(4) {
        let earlier = before[offset / 4];
        let now = unsafe { read_i32(raw.cast(), offset) };
        if !(1..=10_000).contains(&earlier) || !(1..=10_000).contains(&now) {
            continue;
        }
        let consumed = if now < earlier {
            earlier - now
        } else {
            earlier + (10_000 - now)
        };
        if (200..9_000).contains(&consumed) {
            if found.is_some() {
                return None;
            }
            found = Some(offset);
        }
    }
    found
}

fn snapshot_ints(raw: *mut rquickjs::qjs::JSContext) -> Vec<i32> {
    (0..CONTEXT_SCAN_BYTES)
        .step_by(4)
        .map(|offset| unsafe { read_i32(raw.cast(), offset) })
        .collect()
}

fn eval_discard(ctx: &Ctx<'_>, source: &str) -> Option<()> {
    match ctx.eval::<Value<'_>, _>(source) {
        Ok(_) => Some(()),
        Err(Error::Exception) => {
            let _ = ctx.catch();
            None
        }
        Err(_) => None,
    }
}

/// `Ctx` is a `NonNull<JSContext>` plus a zero-sized lifetime marker.
/// rquickjs 0.14 keeps the pointer accessor crate-private.
fn js_context(ctx: &Ctx<'_>) -> *mut rquickjs::qjs::JSContext {
    unsafe { std::ptr::read(std::ptr::from_ref(ctx).cast()) }
}

fn stack_pointer() -> usize {
    let mut slot = 0u8;
    std::hint::black_box(&raw mut slot) as usize
}

unsafe fn read_i32(ctx: *mut c_void, offset: usize) -> i32 {
    unsafe { ctx.cast::<u8>().add(offset).cast::<i32>().read() }
}

unsafe fn write_counter(ctx: *mut c_void, offset: usize, value: i32) {
    unsafe { ctx.cast::<u8>().add(offset).cast::<i32>().write(value) }
}

fn save_prototypes(ctx: &Ctx<'_>) -> Result<Protos, Failure> {
    Ok(Protos {
        internal: Persistent::save(ctx, eval_object(ctx, "InternalError.prototype")?),
        range: Persistent::save(ctx, eval_object(ctx, "RangeError.prototype")?),
        syntax: Persistent::save(ctx, eval_object(ctx, "SyntaxError.prototype")?),
        type_error: Persistent::save(ctx, eval_object(ctx, "TypeError.prototype")?),
        tool: Persistent::save(ctx, eval_object(ctx, "ToolError.prototype")?),
    })
}

fn hide_blocking_intrinsics(ctx: &Ctx<'_>) -> Result<(), Failure> {
    const SOURCE: &str = r#"
        (function () {
            for (const name of ["Atomics", "SharedArrayBuffer"]) {
                const desc = Object.getOwnPropertyDescriptor(globalThis, name);
                if (!desc) continue;
                if (desc.configurable) {
                    delete globalThis[name];
                } else if (desc.writable) {
                    globalThis[name] = undefined;
                }
            }
        })()
    "#;
    match ctx.eval::<(), _>(SOURCE) {
        Ok(()) => Ok(()),
        Err(Error::Exception) => Err(Failure::internal(boundary::take_exception(ctx).message)),
        Err(error) => Err(Failure::internal(error.to_string())),
    }
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

fn format_error(value: &Value<'_>) -> Option<String> {
    let object = value.as_object()?;
    let name = object.get::<_, String>("name").ok()?;
    if !name.ends_with("Error") {
        return None;
    }
    let message = object.get::<_, String>("message").unwrap_or_default();
    let mut text = format!("{name}: {message}");
    if name == "ToolError"
        && let Ok(code) = object.get::<_, String>("code")
        && !code.is_empty()
    {
        text.push_str(&format!(" [{code}]"));
    }
    Some(text)
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
    if let Some(text) = format_error(value) {
        return text;
    }
    if depth >= 4 && value.is_object() {
        return "[Object …]".into();
    }
    if !value.is_object() {
        return "[Object]".into();
    }
    let ptr = unsafe { rquickjs::qjs::JS_VALUE_GET_PTR(value.as_raw()) as usize };
    if stack.contains(&ptr) {
        return "[Circular]".into();
    }
    stack.push(ptr);
    let text = if let Some(array) = value.as_array() {
        let total = array.len();
        let shown = total.min(20);
        let mut parts = Vec::new();
        for index in 0..shown {
            let element = array.get::<Value<'_>>(index).ok();
            parts.push(match element {
                Some(element) => format_debug(&element, depth + 1, stack),
                None => "[?]".into(),
            });
        }
        if total > shown {
            format!("[{}, … {} more]", parts.join(", "), total - shown)
        } else {
            format!("[{}]", parts.join(", "))
        }
    } else if let Some(object) = value.as_object() {
        let mut parts = Vec::new();
        let mut extra = 0usize;
        for (index, (key, child)) in object.props::<String, Value<'_>>().flatten().enumerate() {
            if index < 20 {
                parts.push(format!(
                    "{key}: {}",
                    format_debug(&child, depth + 1, stack)
                ));
            } else {
                extra += 1;
            }
        }
        if extra > 0 {
            format!("{{{}, … {extra} more}}", parts.join(", "))
        } else {
            format!("{{{}}}", parts.join(", "))
        }
    } else {
        "[Object]".into()
    };
    stack.pop();
    text
}

#[cfg(test)]
mod settle_tests {
    use super::*;

    #[test]
    fn success_is_not_rewritten_when_the_clock_is_already_exhausted() {
        let clock = BudgetClock::new(Duration::from_millis(1));
        thread::sleep(Duration::from_millis(5));
        assert!(clock.exhausted());
        let output = RefCell::new(OutputLedger::new(1024));
        let flags = Flags {
            output: AtomicBool::new(false),
        };
        let user = CancellationToken::new();
        let end = settle::<(), ()>(
            Either::Left((
                ProgramEnd::Success(Json::new(serde_json::json!("kept"))),
                (),
            )),
            &output,
            &flags,
            &clock,
            &user,
            1024,
        );
        assert!(matches!(end, ProgramEnd::Success(_)));
    }

    #[test]
    fn timeout_during_drain_keeps_the_returned_value() {
        let output = RefCell::new(OutputLedger::new(4096));
        let end = reconcile_drain(
            ProgramEnd::Success(Json::new(serde_json::json!("kept"))),
            Some(Failure::timeout()),
            &output,
        );
        match end {
            ProgramEnd::Success(value) => {
                assert_eq!(value.as_value(), &serde_json::json!("kept"));
            }
            _ => panic!("expected the returned value to survive a drain timeout"),
        }
        assert!(
            output.borrow().logs().iter().any(|entry| {
                entry.message.contains("program budget expired while draining")
            })
        );
    }

    #[test]
    fn cancel_during_drain_keeps_the_value() {
        let output = RefCell::new(OutputLedger::new(4096));
        let end = reconcile_drain(
            ProgramEnd::Success(Json::new(serde_json::json!("kept"))),
            Some(Failure::cancelled()),
            &output,
        );
        assert!(matches!(end, ProgramEnd::CancelledAfterReturn(_)));
    }

    #[test]
    fn other_drain_failures_replace_success() {
        let output = RefCell::new(OutputLedger::new(4096));
        let end = reconcile_drain(
            ProgramEnd::Success(Json::new(serde_json::json!("kept"))),
            Some(Failure::output(32)),
            &output,
        );
        match end {
            ProgramEnd::Failure(failure) => assert_eq!(failure.kind, FailureKind::OutputLimit),
            _ => panic!("expected the output failure to replace success"),
        }
    }
}

