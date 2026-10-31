//! Embedded JavaScript runtime for code mode.
//!
//! This module does not know about tools, sessions, or permissions. [`execute`]
//! takes a program, named async host bindings, a pausable time budget, a
//! cancellation token, and resource limits. It returns logs, a JSON value, an
//! orthogonal failure, and a host-call log.
//!
//! JavaScript runs on a dedicated thread. Host futures run on the caller's
//! tokio runtime. The engine is QuickJS-NG through `rquickjs`: `quickjs-libc`
//! is not linked, and there is no module loader. Replacing the engine or moving
//! execution into a subprocess should not change this contract.
//!
//! # Host integers
//!
//! Integers coming back from a host are injected as JavaScript numbers only
//! when their magnitude is at most [`MAX_SAFE_INTEGER`] (`2^53 - 1`). A larger
//! integer fails that call with a type error and is not delivered, so the
//! boundary never drops bits. `2^53` itself is rejected. Non-integer JSON
//! numbers are injected as numbers.

mod boundary;
mod budget;
mod calls;
mod engine;
mod fs;
mod heap;
mod json;
mod output;

use std::fmt;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread;
use std::time::Duration;

use futures_util::future::BoxFuture;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

pub(crate) use boundary::json_bytes;
pub use boundary::{DROPPED_CAP, DroppedUndefined};
pub use budget::{BudgetPause, BudgetPauseGuard};
pub use json::Json;

/// Largest integer that is exact in an IEEE-754 number (`2^53 - 1`).
pub const MAX_SAFE_INTEGER: i64 = (1_i64 << 53) - 1;

/// Limits for one program execution.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// QuickJS heap cap in bytes. `0` means unlimited.
    pub memory_bytes: usize,
    /// QuickJS stack cap in bytes.
    pub stack_bytes: usize,
    /// Shared hard cap for console text and the final JSON value, in UTF-8 bytes.
    pub output_bytes: usize,
    /// Maximum number of host calls that may be in flight together.
    pub max_in_flight: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: 16 * 1024 * 1024,
            stack_bytes: 512 * 1024,
            output_bytes: 1024 * 1024,
            max_in_flight: 64,
        }
    }
}

/// How a host failure is thrown into the program.
///
/// The runtime does not interpret tool codes. `Tool` is a host failure that
/// carries a code; `Type` is a protocol failure; `Error` is a plain exception.
#[derive(Debug, Clone, PartialEq)]
pub enum HostErrorKind {
    Error,
    Type,
    Tool {
        code: String,
        details: Option<serde_json::Value>,
        retryable: bool,
    },
}

/// Error returned by a host function.
///
/// The call is still completed: the promise rejects, and the program may catch it.
#[derive(Debug, Clone, PartialEq)]
pub struct HostError {
    pub kind: HostErrorKind,
    pub message: String,
}

impl HostError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            kind: HostErrorKind::Error,
            message: message.into(),
        }
    }

    pub fn type_error(message: impl Into<String>) -> Self {
        Self {
            kind: HostErrorKind::Type,
            message: message.into(),
        }
    }

    pub fn tool(
        code: impl Into<String>,
        message: impl Into<String>,
        details: Option<serde_json::Value>,
        retryable: bool,
    ) -> Self {
        Self {
            kind: HostErrorKind::Tool {
                code: code.into(),
                details,
                retryable,
            },
            message: message.into(),
        }
    }
}

/// Identity of an uncaught `ToolError`.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolErrorInfo {
    pub code: String,
    pub tool: String,
    pub seq: u64,
    pub details: Option<serde_json::Value>,
}

pub type HostFuture = BoxFuture<'static, Result<serde_json::Value, HostError>>;

/// Identity of one host call. `seq` is the runtime ledger's submission number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallInfo {
    pub seq: u64,
    /// Program budget left when the call began.
    pub remaining: std::time::Duration,
}

/// Async host function. It receives one JSON argument, a cancellation token for
/// this call, a handle that pauses the program budget, and the call's seq.
///
/// The argument is a [`Json`] value. Dropping it, even when the host ignores
/// it, does not walk a deep tree on the thread stack.
#[derive(Clone)]
pub struct HostFn {
    invoke: std::sync::Arc<
        dyn Fn(Json, CancellationToken, BudgetPause, CallInfo) -> HostFuture + Send + Sync,
    >,
}

impl HostFn {
    pub fn new<F, Fut>(function: F) -> Self
    where
        F: Fn(Json, CancellationToken, BudgetPause, CallInfo) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<serde_json::Value, HostError>> + Send + 'static,
    {
        Self {
            invoke: std::sync::Arc::new(move |args, cancel, pause, info| {
                Box::pin(function(args, cancel, pause, info))
            }),
        }
    }

    pub(crate) fn call(
        &self,
        args: Json,
        cancel: CancellationToken,
        pause: BudgetPause,
        info: CallInfo,
    ) -> HostFuture {
        (self.invoke)(args, cancel, pause, info)
    }
}

impl fmt::Debug for HostFn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HostFn")
    }
}

/// One function inside a binding namespace.
///
/// A dotted name nests: `deck.only` is exposed as `tools.deck.only`.
/// Empty segments are rejected at install time.
#[derive(Debug, Clone)]
pub struct HostBinding {
    pub name: String,
    pub function: HostFn,
}

/// A global object whose properties are async host functions.
///
/// `libs` and `console` are reserved. Names must be JavaScript identifiers.
/// Dotted binding names create nested objects with async functions on the leaf.
#[derive(Debug, Clone)]
pub struct Namespace {
    pub name: String,
    pub functions: Vec<HostBinding>,
}

/// One execution of a program.
#[derive(Debug, Clone)]
pub struct ExecuteRequest {
    /// Async function body. Top-level `await` and `return` are supported.
    pub source: String,
    pub namespaces: Vec<Namespace>,
    pub limits: Limits,
    pub budget: Duration,
    /// How long to wait for in-flight host calls after a terminating path
    /// cancels them. This is wall time and is not charged to `budget`.
    pub grace: Duration,
    pub cancel: CancellationToken,
    /// Workspace root for the read-only `fs.ls/stat/exists` probes
    /// (WP-210 I). `None` leaves `fs` undefined.
    pub fs_root: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Log,
    Info,
    Warn,
    Error,
    Debug,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    /// The host future finished, and cancellation had not been signaled.
    Completed,
    /// Cancellation was signaled and the host future finished within the grace period.
    CancelledFinished,
    /// Cancellation was signaled and the host future was still running when the
    /// grace period ended. The future is left running; it is not aborted.
    CancelledUnfinished,
}

/// One host call, in submission order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    pub seq: u64,
    pub namespace: String,
    pub name: String,
    /// Wall time from the start of this execution, including paused intervals.
    pub started: Duration,
    pub ended: Option<Duration>,
    pub state: CallState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Memory,
    Stack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Syntax,
    Exception,
    Timeout,
    Cancelled,
    ResourceLimit,
    OutputLimit,
    IllegalOutput,
    Internal,
}

/// 1-based position in the user's source, after the wrapper line is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub kind: FailureKind,
    pub resource: Option<ResourceKind>,
    pub message: String,
    pub stack: Option<String>,
    pub location: Option<SourceLocation>,
    /// The syntax site was outside the user's source.
    pub at_end_of_input: bool,
    /// Set when the program let a `ToolError` escape.
    pub tool_error: Option<ToolErrorInfo>,
    /// Set when the program let a `TypeError` escape.
    pub uncaught_type_error: bool,
}

impl Failure {
    pub(crate) fn new(
        kind: FailureKind,
        message: impl Into<String>,
        stack: Option<String>,
        resource: Option<ResourceKind>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            stack,
            resource,
            location: None,
            at_end_of_input: false,
            tool_error: None,
            uncaught_type_error: false,
        }
    }

    pub(crate) fn syntax(
        message: impl Into<String>,
        stack: Option<String>,
        source_lines: u32,
    ) -> Self {
        let message = message.into();
        let (location, at_end_of_input) = locate_syntax(stack.as_deref(), &message, source_lines);
        let mut failure = Self::new(FailureKind::Syntax, message, stack, None);
        failure.location = location;
        failure.at_end_of_input = at_end_of_input;
        failure
    }

    pub(crate) fn exception(message: impl Into<String>, stack: Option<String>) -> Self {
        Self::new(FailureKind::Exception, message, stack, None)
    }

    /// Runtime exception: first `program.js` frame of the stack, mapped to the
    /// user's source (wrapper line removed). Left unset when the throw site is
    /// not in the user's code.
    pub(crate) fn locate_runtime(mut self, source_lines: u32) -> Self {
        if let Some((line, column)) = self.stack.as_deref().and_then(program_js_site) {
            let adjusted = line.saturating_sub(1);
            if adjusted >= 1 && adjusted <= source_lines {
                self.location = Some(SourceLocation {
                    line: adjusted,
                    column,
                });
            }
        }
        self
    }

    pub(crate) fn timeout() -> Self {
        Self::new(FailureKind::Timeout, "budget exceeded", None, None)
    }

    pub(crate) fn cancelled() -> Self {
        Self::new(FailureKind::Cancelled, "cancelled", None, None)
    }

    pub(crate) fn resource(
        kind: ResourceKind,
        message: impl Into<String>,
        stack: Option<String>,
    ) -> Self {
        Self::new(FailureKind::ResourceLimit, message, stack, Some(kind))
    }

    pub(crate) fn output(limit: usize) -> Self {
        Self::new(
            FailureKind::OutputLimit,
            format!("output exceeded {limit} bytes"),
            None,
            None,
        )
    }

    pub(crate) fn illegal(message: impl Into<String>) -> Self {
        Self::new(FailureKind::IllegalOutput, message, None, None)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(FailureKind::Internal, message, None, None)
    }
}

/// QuickJS reports `program.js:L:C` against the wrapped source. The wrapper
/// occupies line 1, so the user's line is `L - 1`. A site outside the user's
/// source is `at end of input`.
pub(crate) fn locate_syntax(
    stack: Option<&str>,
    message: &str,
    source_lines: u32,
) -> (Option<SourceLocation>, bool) {
    let site = stack
        .and_then(program_js_site)
        .or_else(|| program_js_site(message));
    let Some((line, column)) = site else {
        return (None, false);
    };
    let adjusted = line.saturating_sub(1);
    if adjusted == 0 || adjusted > source_lines {
        return (None, true);
    }
    (
        Some(SourceLocation {
            line: adjusted,
            column,
        }),
        false,
    )
}

fn program_js_site(text: &str) -> Option<(u32, u32)> {
    let marker = "program.js:";
    let start = text.find(marker)?;
    let rest = &text[start + marker.len()..];
    let line_digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if line_digits == 0 {
        return None;
    }
    let line: u32 = rest[..line_digits].parse().ok()?;
    let rest = &rest[line_digits..];
    if !rest.starts_with(':') {
        return None;
    }
    let rest = &rest[1..];
    let column_digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if column_digits == 0 {
        return None;
    }
    let column: u32 = rest[..column_digits].parse().ok()?;
    Some((line, column))
}

/// Result of one program.
///
/// `value` is kept when the program has returned. `failure` coexists with
/// `value` only for [`FailureKind::Cancelled`] (the user cancelled after the
/// return). A later [`FailureKind::Timeout`] or [`FailureKind::Internal`]
/// during drain is a warning on `logs`.
///
/// `value` is [`Json`], so dropping, cloning, comparing, or debugging an
/// outcome does not overflow the stack on a deep tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub logs: Vec<LogEntry>,
    pub value: Option<Json>,
    pub failure: Option<Failure>,
    pub calls: Vec<CallRecord>,
    /// Active program time when the run settled (excludes approval pauses).
    pub duration_ms: u64,
    /// Result paths where `undefined` was dropped / nulled (WP-206 #7).
    pub dropped_undefined: boundary::DroppedUndefined,
}

impl Outcome {
    pub(crate) fn from_failure(failure: Failure) -> Self {
        Self {
            logs: Vec::new(),
            value: None,
            failure: Some(failure),
            calls: Vec::new(),
            duration_ms: 0,
            dropped_undefined: Default::default(),
        }
    }
}

/// Run `request.source` as an async function body.
///
/// Must be called from a tokio runtime. The returned future does not execute
/// JavaScript on that runtime's worker threads.
pub async fn execute(request: ExecuteRequest) -> Outcome {
    let handle = match Handle::try_current() {
        Ok(handle) => handle,
        Err(_) => {
            return Outcome::from_failure(Failure::internal(
                "execute must be called from a tokio runtime",
            ));
        }
    };
    if let Err(message) = validate(&request) {
        return Outcome::from_failure(Failure::internal(message));
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    // The thread must be deeper than QuickJS's native limit (itself past the
    // stack watch), or a deep program would hit the OS guard page first.
    let mut builder = thread::Builder::new().name("ptc-js".into());
    if let Some(bytes) = engine::js_thread_stack_bytes(request.limits.stack_bytes) {
        builder = builder.stack_size(bytes);
    }
    let spawned = builder.spawn(move || {
        let outcome = catch_unwind(AssertUnwindSafe(|| engine::drive_blocking(request, handle)));
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(_) => Outcome::from_failure(Failure::internal("javascript thread panicked")),
        };
        let _ = tx.send(outcome);
    });

    match spawned {
        Err(_) => Outcome::from_failure(Failure::internal("failed to spawn the javascript thread")),
        Ok(_) => match rx.await {
            Ok(outcome) => outcome,
            Err(_) => {
                Outcome::from_failure(Failure::internal("javascript thread dropped the result"))
            }
        },
    }
}

fn validate(request: &ExecuteRequest) -> Result<(), String> {
    let mut namespaces = std::collections::HashSet::new();
    for namespace in &request.namespaces {
        if !is_ident(&namespace.name) {
            return Err(format!("invalid namespace '{}'", namespace.name));
        }
        if matches!(namespace.name.as_str(), "libs" | "console" | "ToolError") {
            return Err(format!("'{}' is reserved", namespace.name));
        }
        if !namespaces.insert(namespace.name.as_str()) {
            return Err(format!("duplicate namespace '{}'", namespace.name));
        }
        let mut functions = std::collections::HashSet::new();
        for binding in &namespace.functions {
            let path_is_valid = binding.name.split('.').all(is_ident);
            if !path_is_valid {
                return Err(format!(
                    "invalid function '{}.{}'",
                    namespace.name, binding.name
                ));
            }
            if !functions.insert(binding.name.as_str()) {
                return Err(format!(
                    "duplicate function '{}.{}'",
                    namespace.name, binding.name
                ));
            }
        }
        for name in &functions {
            if functions
                .iter()
                .any(|other| name.starts_with(&format!("{other}.")))
            {
                return Err(format!(
                    "function '{}.{name}' nests under another function",
                    namespace.name
                ));
            }
        }
    }
    Ok(())
}

fn is_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(char) if char == '_' || char == '$' || char.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|char| char == '_' || char == '$' || char.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests;
