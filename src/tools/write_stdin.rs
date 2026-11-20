//! Codex-compatible `write_stdin`: poll, feed bytes, or Ctrl-C a session.
//!
//! Empty `chars` waits for new output or exit, whichever comes first, and can
//! wait up to 600000 ms. Ordinary bytes go to stdin.
//!
//! `chars == "\u0003"` is deliberately translated to a process-group SIGINT,
//! not written into a pipe. That is the difference between a real keyboard
//! interrupt (which runs `finally` cleanup) and a hard kill.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::terminal::{AgentJobRegistry, ProcessSignal};
use crate::tool::Tool;
use crate::tool::clamp::clamp_int;
use crate::tool::outcome::ToolOutcome;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::command_job;
use crate::tools::exec_command::{
    ExecCommandData, MAX_OUTPUT_TOKENS, resolve_output_max_bytes,
};
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

const INTERRUPT: &str = "\u{3}";
const DEFAULT_WRITE_YIELD_MS: u64 = 250;
const DEFAULT_POLL_YIELD_MS: u64 = 5_000;
const MIN_YIELD_MS: u64 = 250;
const MAX_YIELD_MS: u64 = 600_000;

#[derive(Debug, Clone)]
struct ParsedArgs {
    session_id: String,
    chars: String,
    yield_ms: u64,
    max_output_tokens: Option<usize>,
    clamped: Vec<crate::tool::clamp::Clamp>,
}

pub struct WriteStdinTool {
    jobs: Arc<AgentJobRegistry>,
}

impl WriteStdinTool {
    pub fn new(jobs: Arc<AgentJobRegistry>) -> Self {
        Self { jobs }
    }

    fn parse_args(input: &Value) -> Result<ParsedArgs, String> {
        let session_id =
            crate::tool::require_nonempty_string_trimmed(input, "session_id")?.to_string();
        let chars = match input.get("chars") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(value)) => value.clone(),
            Some(_) => return Err("parameter 'chars' expected a string".into()),
        };
        let default_yield = if chars.is_empty() {
            DEFAULT_POLL_YIELD_MS
        } else {
            DEFAULT_WRITE_YIELD_MS
        };
        let mut clamped = Vec::new();
        let yield_value =
            clamp_int(input, "yield_time_ms", MIN_YIELD_MS, MAX_YIELD_MS, "ms")?;
        if let Some(note) = yield_value.as_ref().and_then(|value| value.as_clamp("yield_time_ms")) {
            clamped.push(note);
        }
        let yield_ms = yield_value.map_or(default_yield, |value| value.value);
        let max_tokens_value = clamp_int(
            input,
            "max_output_tokens",
            1,
            MAX_OUTPUT_TOKENS as u64,
            "tokens",
        )?;
        if let Some(note) = max_tokens_value
            .as_ref()
            .and_then(|value| value.as_clamp("max_output_tokens"))
        {
            clamped.push(note);
        }
        let max_output_tokens = max_tokens_value.map(|value| value.value as usize);
        Ok(ParsedArgs {
            session_id,
            chars,
            yield_ms,
            max_output_tokens,
            clamped,
        })
    }

    async fn run(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let args = match Self::parse_args(&input) {
            Ok(args) => args,
            Err(error) => return coded_error("invalid_input", error),
        };
        if !command_job::owns_session(&self.jobs, &execution.session_id, &args.session_id) {
            return command_job::not_found(&self.jobs, &execution.session_id, &args.session_id);
        }
        tracing::info!(
            tool = "write_stdin",
            session_id = %args.session_id,
            chars_len = args.chars.len(),
            ctrl_c = args.chars == INTERRUPT,
            yield_ms = args.yield_ms,
            "write_stdin"
        );
        let cursor = self.jobs.read_cursor(&args.session_id).unwrap_or(0);
        let (alive_before, _, _, _) = self.jobs.get(&args.session_id).unwrap_or((
            false,
            None,
            String::new(),
            Default::default(),
        ));
        if alive_before {
            if args.chars == INTERRUPT {
                let _ = self
                    .jobs
                    .signal_registered(&args.session_id, ProcessSignal::Interrupt);
            } else if !args.chars.is_empty() {
                if let Err(error) = self
                    .jobs
                    .write_stdin(&args.session_id, args.chars.as_bytes())
                {
                    return coded_error("io", error);
                }
            }
            let _ = if args.chars.is_empty() {
                self.jobs
                    .wait_exit_or_timeout(
                        &args.session_id,
                        Duration::from_millis(args.yield_ms),
                        &execution.cancel,
                    )
                    .await
            } else {
                self.jobs
                    .wait_output_or_exit(
                        &args.session_id,
                        cursor,
                        Duration::from_millis(args.yield_ms),
                        &execution.cancel,
                    )
                    .await
            };
            if execution.cancel.is_cancelled() {
                return coded_error("cancelled", "write_stdin cancelled");
            }
        }
        let max_bytes = resolve_output_max_bytes(args.max_output_tokens, execution.output_limit);
        let delta = self
            .jobs
            .output_delta(&args.session_id, max_bytes)
            .unwrap_or_else(|| crate::terminal::OutputDelta {
                text: String::new(),
                original_bytes: 0,
                total_file_bytes: 0,
                output_file: String::new(),
                alive: false,
                exit_code: None,
                truncated: false,
            });
        let (alive, exit_code, _, _) = self.jobs.get(&args.session_id).unwrap_or((
            false,
            None,
            String::new(),
            Default::default(),
        ));
        if !alive {
            self.jobs.discard_finished(&args.session_id);
        }
        let mut data = ExecCommandData::from_delta(
            alive.then(|| args.session_id.clone()),
            exit_code.map(|code| code as i32),
            delta,
        );
        data.clamped = args.clamped.clone();
        ToolOutcome::ok(data).into_call_result(match (alive, exit_code) {
            (true, _) => format!("session_id: {}\n", args.session_id),
            (false, Some(code)) => format!("exit_code: {code}\n"),
            (false, None) => "exit_code: null\n".to_string(),
        })
    }
}

impl Tool for WriteStdinTool {
    fn name(&self) -> &str {
        "write_stdin"
    }


    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(self.run(input, execution))
    }

    fn timeout(&self) -> Option<Duration> {
        None
    }

    fn is_cancellable(&self) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        Self::parse_args(input).map(|_| ())
    }

    fn max_result_size(&self) -> usize {
        200_000
    }
}
