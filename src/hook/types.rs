use serde::Serialize;
use serde_json::Value;

use crate::types::Item;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HookAction {
    #[default]
    Continue,
    Allow,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectPlacement {
    Head,
    PreTurn,
    PostToolResults,
    Tail,
}

/// Item injection produced by a hook — already in Responses `Item` form.
#[derive(Debug, Clone)]
pub struct HookInjection {
    pub item: Item,
    pub placement: InjectPlacement,
}

impl HookInjection {
    pub fn user_text(content: impl Into<String>, placement: InjectPlacement) -> Self {
        Self {
            item: crate::types::user_text(content.into()),
            placement,
        }
    }

    pub fn assistant_text(content: impl Into<String>, placement: InjectPlacement) -> Self {
        Self {
            item: super::assistant_text(content.into()),
            placement,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HookOutput {
    pub action: HookAction,
    pub inject_items: Vec<HookInjection>,
    pub updated_input: Option<Value>,
    pub display_message: Option<String>,
}

impl HookOutput {
    pub fn ok() -> Self {
        Self::default()
    }

    pub fn block() -> Self {
        Self {
            action: HookAction::Block,
            ..Default::default()
        }
    }

    pub fn with_items(items: Vec<HookInjection>) -> Self {
        Self {
            action: HookAction::Continue,
            inject_items: items,
            ..Default::default()
        }
    }

    pub fn merge(&mut self, other: HookOutput) {
        match other.action {
            HookAction::Block => self.action = HookAction::Block,
            HookAction::Allow => {
                if self.action != HookAction::Block {
                    self.action = HookAction::Allow;
                }
            }
            HookAction::Continue => {}
        }
        self.inject_items.extend(other.inject_items);
        if other.updated_input.is_some() {
            self.updated_input = other.updated_input;
        }
        if let Some(msg) = other.display_message {
            self.display_message = Some(match &self.display_message {
                Some(existing) => format!("{}\n{}", existing, msg),
                None => msg,
            });
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HookPayload {
    pub event: String,
    pub session_id: String,
    pub cwd: String,
    pub data: Value,
}

impl HookPayload {
    pub fn new(event: &str, session_id: &str, cwd: &str, data: Value) -> Self {
        Self {
            event: event.to_string(),
            session_id: session_id.to_string(),
            cwd: cwd.to_string(),
            data,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleType {
    Gating,
    Override,
    Notification,
}

impl LifecycleType {
    pub fn classify(point: &str) -> Self {
        match point {
            "PreToolUse" | "UserPromptSubmit" | "PreCompact" | "PermissionRequest" => {
                LifecycleType::Gating
            }
            "Stop" => LifecycleType::Override,
            _ => LifecycleType::Notification,
        }
    }
}

pub const LIFECYCLE_POINTS_V2: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
    "PreCompact",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_action_default() {
        assert_eq!(HookAction::default(), HookAction::Continue);
    }

    #[test]
    fn hook_output_ok() {
        let out = HookOutput::ok();
        assert_eq!(out.action, HookAction::Continue);
        assert!(out.inject_items.is_empty());
    }

    #[test]
    fn hook_output_block() {
        let out = HookOutput::block();
        assert_eq!(out.action, HookAction::Block);
    }

    #[test]
    fn hook_output_with_items() {
        let items = vec![HookInjection::user_text("hello", InjectPlacement::Head)];
        let out = HookOutput::with_items(items);
        assert_eq!(out.action, HookAction::Continue);
        assert_eq!(out.inject_items.len(), 1);
    }

    #[test]
    fn merge_action_block_wins() {
        let mut a = HookOutput::ok();
        a.merge(HookOutput::block());
        assert_eq!(a.action, HookAction::Block);
    }

    #[test]
    fn merge_items_concatenated() {
        let mut a = HookOutput::with_items(vec![HookInjection::user_text(
            "first",
            InjectPlacement::Head,
        )]);
        a.merge(HookOutput::with_items(vec![HookInjection::user_text(
            "second",
            InjectPlacement::Tail,
        )]));
        assert_eq!(a.inject_items.len(), 2);
    }

    #[test]
    fn merge_display_message_appends() {
        let mut a = HookOutput {
            display_message: Some("first".into()),
            ..Default::default()
        };
        a.merge(HookOutput {
            display_message: Some("second".into()),
            ..Default::default()
        });
        assert!(
            a.display_message
                .as_deref()
                .unwrap()
                .contains("first\nsecond")
        );
    }

    #[test]
    fn lifecycle_classify_gating() {
        assert_eq!(LifecycleType::classify("PreToolUse"), LifecycleType::Gating);
        assert_eq!(
            LifecycleType::classify("UserPromptSubmit"),
            LifecycleType::Gating
        );
        assert_eq!(LifecycleType::classify("PreCompact"), LifecycleType::Gating);
        assert_eq!(
            LifecycleType::classify("PermissionRequest"),
            LifecycleType::Gating
        );
    }

    #[test]
    fn lifecycle_classify_override() {
        assert_eq!(LifecycleType::classify("Stop"), LifecycleType::Override);
    }

    #[test]
    fn lifecycle_classify_notification() {
        assert_eq!(
            LifecycleType::classify("SessionStart"),
            LifecycleType::Notification
        );
        assert_eq!(
            LifecycleType::classify("PostToolUse"),
            LifecycleType::Notification
        );
    }

    #[test]
    fn hook_payload_construction() {
        let p = HookPayload::new(
            "PreToolUse",
            "sess_1",
            "/tmp",
            serde_json::json!({"tool": "bash"}),
        );
        assert_eq!(p.event, "PreToolUse");
        assert_eq!(p.session_id, "sess_1");
        assert_eq!(p.cwd, "/tmp");
    }

    #[test]
    fn hook_context_is_context() {
        let ctx = crate::context_pipeline::Context {
            cwd: ".".into(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(
                &std::path::PathBuf::from("."),
            ),
            agents_md: None,
            claude_md: None,
        };
        assert_eq!(ctx.cwd, std::path::PathBuf::from("."));
    }
}
