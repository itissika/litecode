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
mod output;

use std::fmt;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread;
use std::time::Duration;

use futures_util::future::BoxFuture;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

pub use budget::{BudgetPause, BudgetPauseGuard};

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

/// Error returned by a host function.
///
/// The runtime does not interpret the message. The call is still "completed":
/// the promise rejects, and the program may catch it.
#[derive(Debug, Clone)]
pub struct HostError {
    pub message: String,
}

impl HostError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub type HostFuture = BoxFuture<'static, Result<serde_json::Value, HostError>>;

/// Async host function. It receives one JSON argument, a cancellation token for
/// this call, and a handle that pauses the program budget.
#[derive(Clone)]
pub struct HostFn {
    invoke: std::sync::Arc<
        dyn Fn(serde_json::Value, CancellationToken, BudgetPause) -> HostFuture + Send + Sync,
    >,
}

impl HostFn {
    pub fn new<F, Fut>(function: F) -> Self
    where
        F: Fn(serde_json::Value, CancellationToken, BudgetPause) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<serde_json::Value, HostError>> + Send + 'static,
    {
        Self {
            invoke: std::sync::Arc::new(move |args, cancel, pause| {
                Box::pin(function(args, cancel, pause))
            }),
        }
    }

    pub(crate) fn call(
        &self,
        args: serde_json::Value,
        cancel: CancellationToken,
        pause: BudgetPause,
    ) -> HostFuture {
        (self.invoke)(args, cancel, pause)
    }
}

impl fmt::Debug for HostFn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HostFn")
    }
}

/// One function inside a binding namespace.
#[derive(Debug, Clone)]
pub struct HostBinding {
    pub name: String,
    pub function: HostFn,
}

/// A global object whose properties are async host functions.
///
/// `libs` and `console` are reserved. Names must be JavaScript identifiers.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub resource: Option<ResourceKind>,
    pub message: String,
    pub stack: Option<String>,
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
        }
    }

    pub(crate) fn syntax(message: impl Into<String>) -> Self {
        Self::new(FailureKind::Syntax, message, None, None)
    }

    pub(crate) fn exception(message: impl Into<String>, stack: Option<String>) -> Self {
        Self::new(FailureKind::Exception, message, stack, None)
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

/// Result of one program. `value` is set only when `failure` is absent.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub logs: Vec<LogEntry>,
    pub value: Option<serde_json::Value>,
    pub failure: Option<Failure>,
    pub calls: Vec<CallRecord>,
}

impl Outcome {
    fn from_failure(failure: Failure) -> Self {
        Self {
            logs: Vec::new(),
            value: None,
            failure: Some(failure),
            calls: Vec::new(),
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
    let spawned = thread::Builder::new().name("ptc-js".into()).spawn(move || {
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
        if matches!(namespace.name.as_str(), "libs" | "console") {
            return Err(format!("'{}' is reserved", namespace.name));
        }
        if !namespaces.insert(namespace.name.as_str()) {
            return Err(format!("duplicate namespace '{}'", namespace.name));
        }
        let mut functions = std::collections::HashSet::new();
        for binding in &namespace.functions {
            if !is_ident(&binding.name) {
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
