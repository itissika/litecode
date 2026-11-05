//! Bot product layer: identity, directory, and message delivery.
//!
//! A bot is a product identity, not a session kind. Each bot owns a default
//! session created and driven exclusively through
//! [`crate::session::SessionService`]; bot fields live only in this layer's
//! workspace-local registry.

mod resolve;
mod service;

pub use resolve::{
    AgentResolution, BOT_MAX_STEPS, expand_builtin_prompt, platform_prompt,
    resolve_agent_for_session, step_max_for_session,
};
pub use service::{
    BOT_FOLDER_PREFIX, BotBody, BotError, BotEvent, BotPosition, BotRecord, BotService,
    BotTurnRunner, BotUpdate, MAX_BOTS_PER_WORKSPACE, is_bot_folder_id,
};
