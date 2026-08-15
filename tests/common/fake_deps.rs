//! Fake [`AgentDeps`] for agent-loop unit tests — returns authority `Item`s only.

use std::cell::{Cell, RefCell};

use litecode::agent::AgentDeps;
use litecode::authority::responses::{
    AssistantRole, FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall, MessageItem,
    OutputMessage, OutputMessageContent, OutputStatus, OutputTextContent,
};
use litecode::types::{Item, LitecodeError, Result, Transcript};

/// Build an assistant output-text `Item` (authority OutputMessage).
pub fn assistant_text_item(text: &str, id: &str) -> Item {
    Item::Message(MessageItem::Output(OutputMessage {
        content: vec![OutputMessageContent::OutputText(OutputTextContent {
            text: text.into(),
            annotations: vec![],
            logprobs: None,
        })],
        id: id.into(),
        role: AssistantRole::Assistant,
        phase: None,
        status: OutputStatus::Completed,
    }))
}

/// Build a `function_call` Item.
pub fn function_call_item(call_id: &str, name: &str, arguments: &str, id: &str) -> Item {
    Item::FunctionCall(FunctionToolCall {
        arguments: arguments.into(),
        call_id: call_id.into(),
        namespace: None,
        name: name.into(),
        id: Some(id.into()),
        status: None,
    })
}

/// Snapshot of transcript item types at each `persist_items` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistSnapshot {
    pub len: usize,
    pub types: Vec<&'static str>,
}

fn item_type_name(item: &Item) -> &'static str {
    match item {
        Item::Message(_) => "message",
        Item::FunctionCall(_) => "function_call",
        Item::FunctionCallOutput(_) => "function_call_output",
        Item::Reasoning(_) => "reasoning",
        _ => "other",
    }
}

pub struct FakeAgentDeps {
    /// Queued model outputs; each entry is one `call_model` return value.
    pub responses: Vec<Vec<Item>>,
    call_index: Cell<usize>,
    pub max_steps: u32,
    pub cancelled: Cell<bool>,
    /// If true, `call_model` marks the turn cancelled after returning items.
    pub cancel_after_model: bool,
    pub persist_fail: bool,
    pub execute_calls: Cell<u32>,
    pub stop_on_text: bool,
    pub compact_calls: Cell<u64>,
    /// Ordered snapshots from `persist_items` (for timing assertions).
    pub persist_log: RefCell<Vec<PersistSnapshot>>,
}

impl Default for FakeAgentDeps {
    fn default() -> Self {
        Self {
            responses: vec![],
            call_index: Cell::new(0),
            max_steps: 50,
            cancelled: Cell::new(false),
            cancel_after_model: false,
            persist_fail: false,
            execute_calls: Cell::new(0),
            stop_on_text: true,
            compact_calls: Cell::new(0),
            persist_log: RefCell::new(Vec::new()),
        }
    }
}

impl FakeAgentDeps {
    pub fn with_responses(responses: Vec<Vec<Item>>) -> Self {
        Self {
            responses,
            ..Default::default()
        }
    }

    pub fn with_text_response(text: &str) -> Self {
        Self::with_responses(vec![vec![assistant_text_item(text, "msg_fake_1")]])
    }
}

impl AgentDeps for FakeAgentDeps {
    async fn call_model(&mut self) -> Result<Vec<Item>> {
        let idx = self.call_index.get();
        self.call_index.set(idx + 1);
        let items = self
            .responses
            .get(idx)
            .cloned()
            .ok_or_else(|| LitecodeError::Llm("no more fake responses".into()))?;
        if self.cancel_after_model {
            self.cancelled.set(true);
        }
        Ok(items)
    }

    async fn execute_tools(
        &self,
        tool_uses: &[FunctionToolCall],
        transcript: &mut Transcript,
    ) -> Result<()> {
        self.execute_calls
            .set(self.execute_calls.get().saturating_add(1));
        for tu in tool_uses {
            transcript.push(Item::FunctionCallOutput(FunctionCallOutputItemParam {
                call_id: tu.call_id.clone(),
                output: FunctionCallOutput::Text("fake result".into()),
                id: None,
                status: None,
            }));
        }
        Ok(())
    }

    async fn should_stop(&self, output: &[Item]) -> Result<bool> {
        let has_tools = output.iter().any(|i| matches!(i, Item::FunctionCall(_)));
        if has_tools {
            return Ok(false);
        }
        if self.stop_on_text {
            let has_text = output.iter().any(|i| {
                matches!(i, Item::Message(_)) && !litecode::types::item_text_preview(i).is_empty()
            });
            if has_text {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn compact_if_needed(&self, _transcript: &mut Transcript, _step: u64) -> Result<()> {
        self.compact_calls
            .set(self.compact_calls.get().saturating_add(1));
        Ok(())
    }

    fn emit_todo_progress(&mut self) {}

    fn is_cancelled(&self) -> bool {
        self.cancelled.get()
    }

    fn max_steps(&self) -> u32 {
        self.max_steps
    }

    fn persist_items(&self, items: &mut Vec<Item>) -> Result<()> {
        if self.persist_fail {
            return Err(LitecodeError::ToolExecution("persist failed".into()));
        }
        self.persist_log.borrow_mut().push(PersistSnapshot {
            len: items.len(),
            types: items.iter().map(item_type_name).collect(),
        });
        Ok(())
    }

    fn begin_step(&mut self, _step: u64) {}
}
