//! ask_user — ask the human one or more questions via the PermissionSink bridge.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::permission::{AskAnswer, AskOption, AskOutcome, AskPrompt, AskQuestion, PermissionSink};
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

    fn schema(&self) -> Value {
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
                            },
                            "allow_free_text": {
                                "type": "boolean",
                                "description": "When true, offer an optional free-text field for this question. Default true."
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
                },
                "allow_free_text": {
                    "type": "boolean",
                    "description": "Legacy: free-text for the single top-level question. Default true."
                }
            }
        })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.run(
            &input,
            &CancellationToken::new(),
            self.permission_sink().as_deref(),
        )
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let sink = self.permission_sink();
        let cancel = execution.cancel.clone();
        Box::pin(async move { self.run(&input, &cancel, sink.as_deref()) })
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/ask_user.md"))
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
            let free_text = q
                .get("allow_free_text")
                .or_else(|| q.get("free_text"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
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
            let free_text = input
                .get("allow_free_text")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
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
        sink: Option<&dyn PermissionSink>,
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
        let reply = sink.ask(&prompt, cancel);
        match reply.outcome {
            AskOutcome::Aborted => ToolCallResult::error("ask_user aborted (turn cancelled)"),
            AskOutcome::Deny => {
                ToolCallResult::ok("User skipped the question(s) (no answer).".to_string())
            }
            AskOutcome::Allow { .. } => {
                ToolCallResult::ok(format_answers(&questions, &reply.answers))
            }
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
            lines.push(format!("[{}] selected: (none)", q.id));
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
            lines.push(format!("[{}] selected_ids: {}", q.id, ans.selected.join(",")));
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
    use crate::permission::{AskAnswer, AskOutcome, RecordingPermissionSink};
    use crate::types::ToolSignalLevel;

    #[test]
    fn schema_describes_questions_batch() {
        let tool = AskUserTool::new();
        let schema = tool.schema();
        assert!(schema["properties"]["questions"].is_object());
    }

    #[test]
    fn validate_rejects_single_option() {
        let tool = AskUserTool::new();
        let err = tool
            .validate_input(&serde_json::json!({
                "questions": [{
                    "id": "q1",
                    "prompt": "Pick one?",
                    "options": [{"id": "a", "label": "A"}]
                }]
            }))
            .unwrap_err();
        assert!(err.contains("at least 2"), "{err}");
    }

    #[test]
    fn legacy_question_normalizes_to_q0() {
        let tool = AskUserTool::new();
        tool.set_permission_sink(Arc::new(
            RecordingPermissionSink::from_reply(true, false)
                .with_selected(vec!["ship".into()])
                .with_free_text("lgtm"),
        ));
        let result = tool.call(serde_json::json!({
            "question": "Ship it?",
            "options": [
                {"id": "ship", "label": "Ship"},
                {"id": "hold", "label": "Hold"}
            ]
        }));
        assert_eq!(result.level, ToolSignalLevel::Ok);
        assert!(result.content.contains("[q0] selected_ids: ship"), "{}", result.content);
        assert!(result.content.contains("[q0] free_text: lgtm"), "{}", result.content);
    }

    #[test]
    fn batch_returns_answers_map() {
        let tool = AskUserTool::new();
        let mut answers = HashMap::new();
        answers.insert(
            "color".into(),
            AskAnswer {
                selected: vec!["blue".into()],
                free_text: None,
            },
        );
        answers.insert(
            "size".into(),
            AskAnswer {
                selected: vec!["m".into(), "l".into()],
                free_text: Some("prefer M".into()),
            },
        );
        tool.set_permission_sink(Arc::new(
            RecordingPermissionSink::from_reply(true, false).with_answers(answers),
        ));
        let result = tool.call(serde_json::json!({
            "summary": "Preferences",
            "questions": [
                {
                    "id": "color",
                    "prompt": "Color?",
                    "options": [
                        {"id": "blue", "label": "Blue"},
                        {"id": "red", "label": "Red"}
                    ]
                },
                {
                    "id": "size",
                    "prompt": "Sizes?",
                    "multi_select": true,
                    "options": [
                        {"id": "m", "label": "M"},
                        {"id": "l", "label": "L"}
                    ]
                }
            ]
        }));
        assert_eq!(result.level, ToolSignalLevel::Ok);
        assert!(result.content.contains("[color] selected_ids: blue"), "{}", result.content);
        assert!(result.content.contains("[size] selected_ids: m,l"), "{}", result.content);
        assert!(result.content.contains("[size] free_text: prefer M"), "{}", result.content);
    }

    #[test]
    fn skip_returns_clear_message() {
        let tool = AskUserTool::new();
        tool.set_permission_sink(Arc::new(RecordingPermissionSink::new(AskOutcome::Deny)));
        let result = tool.call(serde_json::json!({
            "questions": [{
                "id": "q1",
                "prompt": "Ship it?",
                "options": [
                    {"id": "ship", "label": "Ship"},
                    {"id": "hold", "label": "Hold"}
                ]
            }]
        }));
        assert_eq!(result.level, ToolSignalLevel::Ok);
        assert!(result.content.contains("skipped"), "{}", result.content);
    }

    #[test]
    fn missing_sink_errors() {
        let tool = AskUserTool::new();
        let result = tool.call(serde_json::json!({
            "questions": [{
                "id": "q1",
                "prompt": "Q?",
                "options": [
                    {"id": "a", "label": "A"},
                    {"id": "b", "label": "B"}
                ]
            }]
        }));
        assert_eq!(result.level, ToolSignalLevel::Error);
        assert!(result.content.contains("Ask bridge"), "{}", result.content);
    }
}
