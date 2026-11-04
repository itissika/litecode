//! Bot product tools. Injected only into bot sessions by the tool registry.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::bot::BotService;
use crate::context_pipeline::Context;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::{DirectToolSpec, Tool};
use crate::types::ToolCallResult;

pub const BOT_CREATE_TOOL: &str = "bot_create";
pub const BOT_LIST_TOOL: &str = "bot_list";
pub const BOT_SEND_TOOL: &str = "bot_send";

/// Create a bot owned by the calling bot.
pub struct BotCreateTool {
    bots: Arc<BotService>,
}

impl BotCreateTool {
    pub fn new(bots: Arc<BotService>) -> Self {
        Self { bots }
    }
}

impl Tool for BotCreateTool {
    fn name(&self) -> &str {
        BOT_CREATE_TOOL
    }

    fn direct_spec(&self, _ctx: &Context) -> Option<DirectToolSpec> {
        Some(DirectToolSpec {
            description: crate::tools::description_text(include_str!("descriptions/bot_create.md")),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Stable bot name, unique in this workspace."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional one-line purpose shown by bot_list."
                    }
                },
                "required": ["name"]
            }),
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        let bots = Arc::clone(&self.bots);
        Box::pin(async move {
            if bots.bot_for_session(&execution.session_id).is_none() {
                return ToolCallResult::error("bot_create is only available to bot sessions");
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
                    "Created bot \"{}\" (id: {}, profile: {}, depth: {}). It has its own default session; message it with bot_send.",
                    bot.name, bot.id, bot.profile_id, bot.depth
                )),
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
    bots: Arc<BotService>,
}

impl BotListTool {
    pub fn new(bots: Arc<BotService>) -> Self {
        Self { bots }
    }
}

impl Tool for BotListTool {
    fn name(&self) -> &str {
        BOT_LIST_TOOL
    }

    fn direct_spec(&self, _ctx: &Context) -> Option<DirectToolSpec> {
        Some(DirectToolSpec {
            description: crate::tools::description_text(include_str!("descriptions/bot_list.md")),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        })
    }

    fn execute(
        &self,
        _input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        let bots = Arc::clone(&self.bots);
        Box::pin(async move {
            if bots.bot_for_session(&execution.session_id).is_none() {
                return ToolCallResult::error("bot_list is only available to bot sessions");
            }
            let bots = bots.list();
            if bots.is_empty() {
                return ToolCallResult::ok("No bots in this workspace.");
            }
            let mut out = format!("Bots in this workspace ({}):", bots.len());
            for bot in bots {
                out.push_str(&format!("\n- {}", bot.name));
                out.push_str(&format!(" — id {}", bot.id));
                out.push_str(&format!(" — profile {}", bot.profile_id));
                if bot.depth > 0 {
                    out.push_str(&format!(" — depth {}", bot.depth));
                }
                if !bot.description.is_empty() {
                    out.push_str(&format!(" — {}", bot.description));
                }
            }
            ToolCallResult::ok(out)
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
    bots: Arc<BotService>,
}

impl BotSendTool {
    pub fn new(bots: Arc<BotService>) -> Self {
        Self { bots }
    }
}

impl Tool for BotSendTool {
    fn name(&self) -> &str {
        BOT_SEND_TOOL
    }

    fn direct_spec(&self, _ctx: &Context) -> Option<DirectToolSpec> {
        Some(DirectToolSpec {
            description: crate::tools::description_text(include_str!("descriptions/bot_send.md")),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "bot": {
                        "type": "string",
                        "description": "Target bot id or name."
                    },
                    "message": {
                        "type": "string",
                        "description": "Message text delivered to the target bot's session."
                    }
                },
                "required": ["bot", "message"]
            }),
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        let bots = Arc::clone(&self.bots);
        Box::pin(async move {
            if bots.bot_for_session(&execution.session_id).is_none() {
                return ToolCallResult::error("bot_send is only available to bot sessions");
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
                    "Queued a message for bot \"{}\" (session {}). This call returned without waiting for a reply.",
                    bot.name, bot.default_session_id
                )),
                Err(error) => ToolCallResult::error(error.to_string()),
            }
        })
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(10))
    }
}

/// The complete bot tool set, appended only for a bot session.
pub fn bot_tools(bots: Arc<BotService>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(BotCreateTool::new(Arc::clone(&bots))),
        Arc::new(BotListTool::new(Arc::clone(&bots))),
        Arc::new(BotSendTool::new(bots)),
    ]
}

fn required_str<'a>(input: &'a Value, key: &str) -> Result<&'a str, ToolCallResult> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ToolCallResult::error(format!("missing required parameter `{key}`")))
}
