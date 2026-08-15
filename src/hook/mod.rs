pub mod dispatcher;
pub mod external;
pub mod registry;
pub mod types;

use crate::authority::responses::{
    AssistantRole, InputMessage, InputRole, MessageItem, OutputMessage, OutputMessageContent,
    OutputStatus, OutputTextContent,
};
use crate::types::{Item, Transcript};

pub use dispatcher::HookDispatcher;
pub use external::ExternalHookAdapter;
pub use registry::{HookRegistry, HookRegistryBuilder};
pub use types::{
    HookAction, HookInjection, HookOutput, HookPayload, InjectPlacement, LIFECYCLE_POINTS_V2,
    LifecycleType,
};

pub(crate) fn assistant_text(content: String) -> Item {
    Item::Message(MessageItem::Output(OutputMessage {
        content: vec![OutputMessageContent::OutputText(OutputTextContent {
            text: content,
            annotations: vec![],
            logprobs: None,
        })],
        id: format!("hook_{}", chrono::Utc::now().timestamp_millis()),
        role: AssistantRole::Assistant,
        phase: None,
        status: OutputStatus::Completed,
    }))
}

pub fn apply_hook_output(transcript: &mut Transcript, output: HookOutput, _ts: i64) {
    if let Some(msg) = &output.display_message {
        tracing::info!(message = %msg, "hook display");
    }

    for inj in output.inject_items {
        match inj.placement {
            InjectPlacement::Head => {
                transcript.insert(0, inj.item);
            }
            InjectPlacement::PreTurn => {
                let pos = transcript
                    .iter()
                    .rposition(|m| {
                        matches!(
                            m,
                            Item::Message(MessageItem::Input(InputMessage {
                                role: InputRole::User,
                                ..
                            }))
                        )
                    })
                    .unwrap_or(transcript.len());
                transcript.insert(pos, inj.item);
            }
            InjectPlacement::PostToolResults => {
                let pos = transcript
                    .iter()
                    .rposition(|m| matches!(m, Item::FunctionCallOutput(_)))
                    .map(|i| i + 1)
                    .unwrap_or(transcript.len());
                transcript.insert(pos, inj.item);
            }
            InjectPlacement::Tail => {
                transcript.push(inj.item);
            }
        }
    }
}
