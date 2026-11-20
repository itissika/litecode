//! Bot product tools. Injected only into bot sessions by the tool registry.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::bot::{BotRecord, BotService};
use crate::session::manager::SessionStatus;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

pub const BOT_CREATE_TOOL: &str = "bot.create";
pub const BOT_LIST_TOOL: &str = "bot.list";
pub const BOT_SEND_TOOL: &str = "bot.send";

/// Create a bot owned by the calling bot.
pub struct BotCreateTool {
    bots: Option<Arc<BotService>>,
}

impl BotCreateTool {
    pub fn new(bots: Arc<BotService>) -> Self {
        Self { bots: Some(bots) }
    }

    pub fn cold() -> Self {
        Self { bots: None }
    }
}

impl Tool for BotCreateTool {
    fn name(&self) -> &str {
        BOT_CREATE_TOOL
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        let Some(bots) = self.bots.as_ref().map(Arc::clone) else {
            return Box::pin(std::future::ready(ToolCallResult::error(
                "bot.create requires an active bot service",
            )));
        };
        Box::pin(async move {
            if bots.bot_for_session(&execution.session_id).is_none() {
                return ToolCallResult::error("bot.create is only available to bot sessions");
            }
            let name = match required_str(&input, "name") {
                Ok(name) => name,
                Err(error) => return error,
            };
            let description = input
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match bots
                .create_from_bot(&execution.session_id, name, description)
                .await
            {
                Ok(bot) => ToolCallResult::ok(format!(
                    "Created bot \"{}\" (id: {}). It has its own session; message it with bot.send.",
                    bot.name, bot.id
                ))
                .with_data(json!({
                    "id": bot.id,
                    "name": bot.name,
                    "session_id": bot.session_id,
                })),
                Err(error) => ToolCallResult::error(error.to_string()),
            }
        })
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(15))
    }
}

/// List every bot in this workspace.
pub struct BotListTool {
    bots: Option<Arc<BotService>>,
}

impl BotListTool {
    pub fn new(bots: Arc<BotService>) -> Self {
        Self { bots: Some(bots) }
    }

    pub fn cold() -> Self {
        Self { bots: None }
    }
}

impl Tool for BotListTool {
    fn name(&self) -> &str {
        BOT_LIST_TOOL
    }

    fn execute(
        &self,
        _input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        let Some(bots) = self.bots.as_ref().map(Arc::clone) else {
            return Box::pin(std::future::ready(ToolCallResult::error(
                "bot.list requires an active bot service",
            )));
        };
        Box::pin(async move {
            let Some(caller) = bots.bot_for_session(&execution.session_id) else {
                return ToolCallResult::error("bot.list is only available to bot sessions");
            };
            let visible = bots.list_for_caller(&caller.id);
            let rows: Vec<Value> = visible
                .iter()
                .map(|bot| {
                    let st = bots
                        .service()
                        .status(&bot.session_id)
                        .unwrap_or(SessionStatus::Idle);
                    json!({
                        "id": bot.id,
                        "name": bot.name,
                        "session_id": bot.session_id,
                        "status": st.as_str(),
                        "is_self": bot.id == caller.id,
                        "description": bot.description,
                    })
                })
                .collect();
            let text = bot_list_text(&visible, &caller.id, |session_id| {
                bots.service()
                    .status(session_id)
                    .unwrap_or(SessionStatus::Idle)
            });
            ToolCallResult::ok(text).with_data(json!({
                "caller_id": caller.id,
                "bots": rows,
            }))
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(5))
    }
}

/// Queue a message for another bot and return without waiting.
pub struct BotSendTool {
    bots: Option<Arc<BotService>>,
}

impl BotSendTool {
    pub fn new(bots: Arc<BotService>) -> Self {
        Self { bots: Some(bots) }
    }

    pub fn cold() -> Self {
        Self { bots: None }
    }
}

impl Tool for BotSendTool {
    fn name(&self) -> &str {
        BOT_SEND_TOOL
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        let Some(bots) = self.bots.as_ref().map(Arc::clone) else {
            return Box::pin(std::future::ready(ToolCallResult::error(
                "bot.send requires an active bot service",
            )));
        };
        Box::pin(async move {
            if bots.bot_for_session(&execution.session_id).is_none() {
                return ToolCallResult::error("bot.send is only available to bot sessions");
            }
            let target = match required_str(&input, "bot") {
                Ok(target) => target,
                Err(error) => return error,
            };
            let message = match required_str(&input, "message") {
                Ok(message) => message,
                Err(error) => return error,
            };
            match bots
                .send_from_bot(&execution.session_id, target, message)
                .await
            {
                Ok(bot) => ToolCallResult::ok(format!(
                    "Queued a message for bot \"{}\" (id {}, session {}). This call returned without waiting for a reply.",
                    bot.name, bot.id, bot.session_id
                ))
                .with_data(json!({
                    "id": bot.id,
                    "name": bot.name,
                    "session_id": bot.session_id,
                    "queued": true,
                })),
                Err(error) => ToolCallResult::error(error.to_string()),
            }
        })
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(10))
    }
}

#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    use crate::tool::code_contract::DriftCase;
    vec![
        DriftCase {
            tool: BOT_CREATE_TOOL,
            all_optional_present: ToolCallResult::ok("ok").with_data(json!({
                "id": "b1",
                "name": "worker",
                "session_id": "s1",
            })),
            all_optional_absent: ToolCallResult::ok("ok").with_data(json!({
                "id": "b1",
                "name": "worker",
                "session_id": "s1",
            })),
        },
        DriftCase {
            tool: BOT_LIST_TOOL,
            all_optional_present: ToolCallResult::ok("ok").with_data(json!({
                "caller_id": "b0",
                "bots": [{
                    "id": "b1",
                    "name": "worker",
                    "session_id": "s1",
                    "status": "idle",
                    "is_self": false,
                    "description": "helper",
                }],
            })),
            all_optional_absent: ToolCallResult::ok("ok").with_data(json!({
                "caller_id": "b0",
                "bots": [{
                    "id": "b1",
                    "name": "worker",
                    "session_id": "s1",
                    "status": "idle",
                    "is_self": false,
                    "description": "",
                }],
            })),
        },
        DriftCase {
            tool: BOT_SEND_TOOL,
            all_optional_present: ToolCallResult::ok("ok").with_data(json!({
                "id": "b1",
                "name": "worker",
                "session_id": "s1",
                "queued": true,
            })),
            all_optional_absent: ToolCallResult::ok("ok").with_data(json!({
                "id": "b1",
                "name": "worker",
                "session_id": "s1",
                "queued": true,
            })),
        },
    ]
}

/// The complete bot tool set, appended only for a bot session.
pub fn bot_tools(bots: Arc<BotService>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(BotCreateTool::new(Arc::clone(&bots))),
        Arc::new(BotListTool::new(Arc::clone(&bots))),
        Arc::new(BotSendTool::new(bots)),
    ]
}

/// Text projection shared by the tool and its unit test. The caller's row is
/// marked `(self)` right after its id, so a bot reading the list can tell which
/// identity is its own. Each row also carries its session's current status.
fn bot_list_text(
    bots: &[BotRecord],
    caller_id: &str,
    status: impl Fn(&str) -> SessionStatus,
) -> String {
    if bots.is_empty() {
        return "No bots are visible to you.".to_string();
    }
    let mut out = format!("Bots visible to you ({}):", bots.len());
    if bots.iter().any(|bot| bot.id.as_str() == caller_id) {
        out.push_str("\nThe entry marked (self) is the calling bot.");
    }
    for bot in bots {
        out.push_str(&format!("\n- {}", bot.name));
        out.push_str(&format!(" — id {}", bot.id));
        if bot.id.as_str() == caller_id {
            out.push_str(" (self)");
        }
        out.push_str(&format!(
            " — bot status: {}",
            status(&bot.session_id).as_str()
        ));
        if !bot.description.is_empty() {
            out.push_str(&format!(" — {}", bot.description));
        }
    }
    out
}

fn required_str<'a>(input: &'a Value, key: &str) -> Result<&'a str, ToolCallResult> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ToolCallResult::error(format!("missing required parameter `{key}`")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bot(id: &str, name: &str) -> BotRecord {
        BotRecord {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            created_at: 1,
            session_id: format!("s-{id}"),
            body: crate::bot::BotBody::default(),
            position: crate::bot::BotPosition::None,
            sort_order: 0,
        }
    }

    /// G6 for the three bot tools: closed objects, so an unknown parameter fails
    /// validation instead of being silently ignored.
    #[test]
    fn bot_schemas_are_closed_objects_and_reject_unknown_parameters() {
        for name in [BOT_CREATE_TOOL, BOT_LIST_TOOL, BOT_SEND_TOOL] {
            let schema = crate::tool::code_contract::builtin_input_schema(name)
                .unwrap_or_else(|| panic!("missing contract schema for {name}"));
            assert_eq!(schema["type"], "object", "{name}");
            assert_eq!(schema["additionalProperties"], serde_json::json!(false), "{name}");
            let mut input = serde_json::Map::new();
            for key in schema["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|key| key.as_str())
            {
                input.insert(key.to_string(), serde_json::json!("x"));
            }
            input.insert("__nope__".into(), serde_json::json!(1));
            let error = crate::tool::schema_validate::schema_validate(
                &schema,
                &serde_json::Value::Object(input),
            )
            .expect_err(&format!("{name} must reject an unknown parameter"));
            assert!(
                error.contains("unknown parameter '__nope__'"),
                "{name}: {error}"
            );
        }
    }

    #[test]
    fn list_marks_the_calling_bot_after_its_id() {
        let text = bot_list_text(
            &[bot("b1", "Researcher"), bot("b2", "Writer")],
            "b1",
            |_| SessionStatus::Idle,
        );
        assert!(text.contains("Bots visible to you"));
        assert!(text.contains("bot status: idle"));
        assert!(!text.contains("in this workspace"));
        assert!(text.contains("The entry marked (self) is the calling bot."));
        assert!(text.contains("id b1 (self)"));
        assert!(!text.contains("id b2 (self)"));
    }
}
