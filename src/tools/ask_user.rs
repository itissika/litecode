//! ask_user — ask the human one or more questions via the PermissionSink bridge.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::permission::{
    AskAnswer, AskOption, AskOutcome, AskPrompt, AskQuestion, AskReply, PermissionSink,
};
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

pub struct AskUserTool {
    permission_sink: Mutex<Option<Arc<dyn PermissionSink>>>,
}

impl AskUserTool {
    pub fn new() -> Self {
        Self {
            permission_sink: Mutex::new(None),
        }
    }

    fn permission_sink(&self) -> Option<Arc<dyn PermissionSink>> {
        self.permission_sink.lock().unwrap().clone()
    }
}

impl Default for AskUserTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for AskUserTool {
    fn name(&self) -> &str {
        "ask_user"
    }

    fn direct_spec(&self, _ctx: &Context) -> Option<crate::tool::DirectToolSpec> {
        let description = crate::tools::description_text(include_str!("descriptions/ask_user.md"));
        let input_schema = {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "summary": {
                        "type": "string",
                        "description": "Optional card title shown above the questions."
                    },
                    "questions": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {
                                    "type": "string",
                                    "description": "Stable id keyed in the answers map."
                                },
                                "prompt": {
                                    "type": "string",
                                    "description": "Question text shown to the user."
                                },
                                "options": {
                                    "type": "array",
                                    "minItems": 2,
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "id": { "type": "string" },
                                            "label": { "type": "string" }
                                        },
                                        "required": ["id", "label"]
                                    },
                                    "description": "Two or more choices for this question."
                                },
                                "multi_select": {
                                    "type": "boolean",
                                    "description": "When true, the user may pick multiple options. Default false."
                                }
                            },
                            "required": ["id", "prompt", "options"]
                        },
                        "description": "One or more questions. Prefer this over legacy top-level question/options."
                    },
                    "question": {
                        "type": "string",
                        "description": "Legacy single-question text (compat → questions[0] with id q0)."
                    },
                    "options": {
                        "type": "array",
                        "minItems": 2,
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "label": { "type": "string" }
                            },
                            "required": ["id", "label"]
                        },
                        "description": "Legacy flat options (compat with top-level question)."
                    },
                    "multi_select": {
                        "type": "boolean",
                        "description": "Legacy: multi_select for the single top-level question."
                    }
                }
            })
        };
        Some(crate::tool::DirectToolSpec {
            description,
            input_schema,
        })
    }

    /// Direct call path: there is no turn token to cancel, so only an answer or
    /// a dead bridge ends the wait. The runtime path goes through `execute`.
    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.run(&input, &CancellationToken::new(), self.permission_sink())
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let sink = self.permission_sink();
        let cancel = execution.cancel.clone();
        Box::pin(async move { self.run(&input, &cancel, sink) })
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        let questions = parse_questions(input)?;
        if questions.is_empty() {
            return Err("ask_user requires questions (>=1) or legacy question+options".into());
        }
        Ok(())
    }

    fn set_permission_sink(&self, sink: Arc<dyn PermissionSink>) {
        *self.permission_sink.lock().unwrap() = Some(sink);
    }
}

fn parse_option(opt: &Value, path: &str) -> Result<AskOption, String> {
    let id = opt
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{path}.id must be a non-empty string"))?;
    let label = opt
        .get("label")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{path}.label must be a non-empty string"))?;
    Ok(AskOption {
        id: id.to_string(),
        label: label.to_string(),
    })
}

fn parse_options_array(arr: &[Value], path: &str) -> Result<Vec<AskOption>, String> {
    if arr.len() < 2 {
        return Err(format!("{path} must contain at least 2 choices"));
    }
    let mut ids = HashSet::new();
    let mut out = Vec::with_capacity(arr.len());
    for (i, opt) in arr.iter().enumerate() {
        let parsed = parse_option(opt, &format!("{path}[{i}]"))?;
        if !ids.insert(parsed.id.clone()) {
            return Err(format!("{path}[{i}].id `{}` is duplicated", parsed.id));
        }
        out.push(parsed);
    }
    Ok(out)
}

fn parse_questions(input: &Value) -> Result<Vec<AskQuestion>, String> {
    if let Some(arr) = input.get("questions").and_then(|v| v.as_array()) {
        if arr.is_empty() {
            return Err("questions must contain at least 1 question".into());
        }
        let mut qids = HashSet::new();
        let mut out = Vec::with_capacity(arr.len());
        for (i, q) in arr.iter().enumerate() {
            let id = q
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("questions[{i}].id must be a non-empty string"))?;
            if !qids.insert(id.to_string()) {
                return Err(format!("questions[{i}].id `{id}` is duplicated"));
            }
            let prompt = q
                .get("prompt")
                .or_else(|| q.get("question"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("questions[{i}].prompt must be a non-empty string"))?;
            let options = q
                .get("options")
                .and_then(|v| v.as_array())
                .ok_or_else(|| format!("questions[{i}].options is required"))?;
            let options = parse_options_array(options, &format!("questions[{i}].options"))?;
            let multi_select = q
                .get("multi_select")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            // WP-080: free text is system-owned; `allow_free_text` / `free_text` from
            // older agents are silently ignored. Every question gets the final entry
            // 「其他（自己填写）」.
            let free_text = true;
            out.push(AskQuestion {
                id: id.to_string(),
                prompt: prompt.to_string(),
                options,
                multi_select,
                free_text,
            });
        }
        return Ok(out);
    }

    // Compat: top-level question + options → single question id q0.
    let question = input
        .get("question")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let options = input.get("options").and_then(|v| v.as_array());
    match (question, options) {
        (Some(prompt), Some(opts)) => {
            let options = parse_options_array(opts, "options")?;
            let multi_select = input
                .get("multi_select")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let free_text = true; // WP-080: legacy flag ignored.
            Ok(vec![AskQuestion {
                id: "q0".into(),
                prompt: prompt.to_string(),
                options,
                multi_select,
                free_text,
            }])
        }
        _ => Ok(Vec::new()),
    }
}

impl AskUserTool {
    fn run(
        &self,
        input: &Value,
        cancel: &CancellationToken,
        sink: Option<Arc<dyn PermissionSink>>,
    ) -> ToolCallResult {
        let questions = match parse_questions(input) {
            Ok(q) if !q.is_empty() => q,
            Ok(_) => {
                return ToolCallResult::error(
                    "ask_user requires questions (>=1) or legacy question+options",
                );
            }
            Err(e) => return ToolCallResult::error(e),
        };

        let summary = input
            .get("summary")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("");

        let Some(sink) = sink else {
            return ToolCallResult::error(
                "ask_user requires a permission Ask bridge (no sink)".to_string(),
            );
        };

        let prompt = AskPrompt::ask_user("ask_user", "ask", summary, &questions);
        let reply = ask_and_wait(sink, &prompt, cancel);
        if cancel.is_cancelled() {
            return ToolCallResult::error("ask_user aborted (turn cancelled)");
        }
        match reply.outcome {
            // A stale/dead UI route is no foreground human, not a tool failure.
            AskOutcome::Aborted => {
                ToolCallResult::ok("User did not answer (no foreground session).".to_string())
            }
            AskOutcome::Deny => {
                ToolCallResult::ok("User skipped the question(s) (no answer).".to_string())
            }
            AskOutcome::Allow { .. } => {
                ToolCallResult::ok(format_answers(&questions, &reply.answers))
            }
        }
    }
}

/// No answer reached the tool: the bridge thread could not start, or its
/// channel closed. Same class as a stale route — aborted, never a silent Deny.
fn no_answer() -> AskReply {
    AskReply::from_outcome(AskOutcome::Aborted)
}

/// Wait for the human: an answer, a turn cancellation, or a dead bridge.
///
/// There is no deadline. A question nobody has answered yet is not a result,
/// and a bot turn is allowed to wait for the person it asked.
fn ask_and_wait(
    sink: Arc<dyn PermissionSink>,
    prompt: &AskPrompt<'_>,
    cancel: &CancellationToken,
) -> AskReply {
    if cancel.is_cancelled() {
        return AskReply::from_outcome(AskOutcome::Aborted);
    }

    let owned = OwnedPrompt::from(prompt);
    let inner = Arc::clone(&sink);
    let local_cancel = cancel.child_token();
    let thread_cancel = local_cancel.clone();
    let (tx, rx) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("ask-user".into())
        .spawn(move || {
            let reply = inner.ask(&owned.as_prompt(), &thread_cancel);
            let _ = tx.send(reply);
        });
    if thread.is_err() {
        local_cancel.cancel();
        return no_answer();
    }

    // The 25 ms tick only makes cancellation responsive; it is not a deadline.
    loop {
        if cancel.is_cancelled() {
            local_cancel.cancel();
            return AskReply::from_outcome(AskOutcome::Aborted);
        }
        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(reply) => {
                if cancel.is_cancelled() {
                    local_cancel.cancel();
                    return AskReply::from_outcome(AskOutcome::Aborted);
                }
                return reply;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return no_answer(),
        }
    }
}

/// Owned copy for the answer worker, which outlives the borrowed prompt.
struct OwnedPrompt {
    tool: String,
    rule_id: String,
    summary: String,
    kind: crate::permission::AskKind,
    free_text: bool,
    options: Vec<AskOption>,
    multi_select: bool,
    questions: Vec<AskQuestion>,
}

impl From<&AskPrompt<'_>> for OwnedPrompt {
    fn from(prompt: &AskPrompt<'_>) -> Self {
        Self {
            tool: prompt.tool.to_string(),
            rule_id: prompt.rule_id.to_string(),
            summary: prompt.summary.to_string(),
            kind: prompt.kind,
            free_text: prompt.free_text,
            options: prompt.options.to_vec(),
            multi_select: prompt.multi_select,
            questions: prompt.questions.to_vec(),
        }
    }
}

impl OwnedPrompt {
    fn as_prompt(&self) -> AskPrompt<'_> {
        AskPrompt {
            tool: &self.tool,
            rule_id: &self.rule_id,
            summary: &self.summary,
            kind: self.kind,
            free_text: self.free_text,
            options: &self.options,
            multi_select: self.multi_select,
            questions: &self.questions,
        }
    }
}

fn format_answers(questions: &[AskQuestion], answers: &HashMap<String, AskAnswer>) -> String {
    let mut lines = Vec::new();
    for q in questions {
        let Some(ans) = answers.get(&q.id) else {
            lines.push(format!("[{}] (no answer)", q.id));
            continue;
        };
        if ans.selected.is_empty() {
            // Text-only answer: user picked the system entry 「其他（自己填写）」.
            if ans.free_text.is_some() {
                lines.push(format!(
                    "[{}] selected: (none) — answered via 「其他（自己填写）」",
                    q.id
                ));
            } else {
                lines.push(format!("[{}] selected: (none)", q.id));
            }
        } else {
            let labels: Vec<String> = ans
                .selected
                .iter()
                .map(|id| {
                    q.options
                        .iter()
                        .find(|o| o.id == *id)
                        .map(|o| format!("{} ({})", o.label, o.id))
                        .unwrap_or_else(|| id.clone())
                })
                .collect();
            lines.push(format!("[{}] selected: {}", q.id, labels.join(", ")));
            lines.push(format!(
                "[{}] selected_ids: {}",
                q.id,
                ans.selected.join(",")
            ));
        }
        if let Some(text) = ans.free_text.as_deref() {
            lines.push(format!("[{}] free_text: {text}", q.id));
        }
    }
    if lines.is_empty() {
        "User submitted with no answers.".to_string()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::{AskPrompt, AskReply, RecordingPermissionSink};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn questions_input() -> Value {
        serde_json::json!({
            "questions": [{
                "id": "q",
                "prompt": "pick",
                "options": [
                    { "id": "a", "label": "A" },
                    { "id": "b", "label": "B" }
                ]
            }]
        })
    }

    struct NeverAnswer {
        cancelled: Arc<AtomicBool>,
    }

    impl PermissionSink for NeverAnswer {
        fn ask(&self, _prompt: &AskPrompt<'_>, cancel: &CancellationToken) -> AskReply {
            while !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.cancelled.store(true, Ordering::Release);
            AskReply::from_outcome(AskOutcome::Aborted)
        }
    }

    struct StaleSink;

    impl PermissionSink for StaleSink {
        fn ask(&self, _prompt: &AskPrompt<'_>, _cancel: &CancellationToken) -> AskReply {
            AskReply::from_outcome(AskOutcome::Aborted)
        }
    }

    struct SkipSink;

    impl PermissionSink for SkipSink {
        fn ask(&self, _prompt: &AskPrompt<'_>, _cancel: &CancellationToken) -> AskReply {
            AskReply::from_outcome(AskOutcome::Deny)
        }
    }

    fn run_tool(
        tool: &AskUserTool,
        sink: Arc<dyn PermissionSink>,
        cancel: &CancellationToken,
    ) -> ToolCallResult {
        tool.run(&questions_input(), cancel, Some(sink))
    }

    fn never_answer() -> (Arc<NeverAnswer>, Arc<AtomicBool>) {
        let cancelled = Arc::new(AtomicBool::new(false));
        (
            Arc::new(NeverAnswer {
                cancelled: Arc::clone(&cancelled),
            }),
            cancelled,
        )
    }

    /// No answer is not a result: the call keeps waiting, for a session and for
    /// a bot alike. Only cancellation ends it (the bot sink has no extra clock).
    #[test]
    fn an_unanswered_question_waits_until_the_turn_is_cancelled() {
        for bot in [false, true] {
            let (sink, sink_cancelled) = never_answer();
            let sink: Arc<dyn PermissionSink> = if bot {
                Arc::new(crate::bot::BotPermissionSink::new(sink))
            } else {
                sink
            };
            let tool = Arc::new(AskUserTool::new());
            let cancel = CancellationToken::new();
            let worker = {
                let tool = Arc::clone(&tool);
                let cancel = cancel.clone();
                std::thread::spawn(move || run_tool(&tool, sink, &cancel))
            };
            std::thread::sleep(Duration::from_millis(200));
            assert!(
                !worker.is_finished(),
                "bot={bot}: an unanswered ask_user must keep waiting"
            );
            cancel.cancel();
            let result = worker.join().expect("worker");
            assert!(result.error.is_some(), "bot={bot}: {result:?}");
            assert!(
                sink_cancelled.load(Ordering::Acquire),
                "bot={bot}: the waiting sink must be released"
            );
        }
    }

    /// A bridge that disappears without answering is aborted, not denied.
    #[test]
    fn a_dead_bridge_is_aborted_not_denied() {
        struct PanickingSink;

        impl PermissionSink for PanickingSink {
            fn ask(&self, _prompt: &AskPrompt<'_>, _cancel: &CancellationToken) -> AskReply {
                panic!("ask bridge thread died")
            }
        }

        let tool = AskUserTool::new();
        let result = run_tool(&tool, Arc::new(PanickingSink), &CancellationToken::new());
        assert!(result.error.is_none(), "{result:?}");
        assert!(
            result.content.contains("no foreground session"),
            "{result:?}"
        );
    }

    #[test]
    fn stale_route_is_a_successful_no_answer() {
        let tool = AskUserTool::new();
        let result = run_tool(&tool, Arc::new(StaleSink), &CancellationToken::new());
        assert!(result.error.is_none(), "{result:?}");
        assert!(result.content.contains("no foreground session"));
        assert!(!result.content.contains("aborted"));
    }

    #[test]
    fn deliberate_skip_is_a_successful_no_answer() {
        let tool = AskUserTool::new();
        let result = run_tool(&tool, Arc::new(SkipSink), &CancellationToken::new());
        assert!(result.error.is_none(), "{result:?}");
        assert!(result.content.contains("skipped"));
    }

    #[test]
    fn real_cancel_aborts_immediately() {
        let (sink, cancelled) = never_answer();
        let cancel = CancellationToken::new();
        let child = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            child.cancel();
        });
        let tool = AskUserTool::new();
        let started = std::time::Instant::now();
        let result = run_tool(&tool, sink, &cancel);
        assert!(result.error.is_some(), "{result:?}");
        assert!(
            result
                .error
                .as_ref()
                .is_some_and(|error| error.message.contains("aborted"))
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn no_sink_is_a_tool_error() {
        let tool = AskUserTool::new();
        let result = tool.run(&questions_input(), &CancellationToken::new(), None);
        assert!(result.error.is_some());
        assert!(
            result
                .error
                .is_some_and(|error| error.message.contains("no sink"))
        );
    }

    #[test]
    fn foreground_answer_still_wins() {
        let mut answers = HashMap::new();
        answers.insert(
            "q".into(),
            crate::permission::AskAnswer {
                selected: vec!["a".into()],
                free_text: None,
            },
        );
        let mut recording = RecordingPermissionSink::new(AskOutcome::Allow { always: false });
        recording.answers = answers;
        let tool = AskUserTool::new();
        let result = run_tool(&tool, Arc::new(recording), &CancellationToken::new());
        assert!(result.error.is_none(), "{result:?}");
        assert!(result.content.contains("selected: A (a)"));
    }
}
