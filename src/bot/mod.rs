//! Bot product layer: identity, directory, and message delivery.
//!
//! A bot is a product identity, not a session kind. Each bot owns a default
//! session created and driven exclusively through
//! [`crate::session::SessionService`]; bot fields live only in this layer's
//! workspace-local registry.

mod service;

pub use service::{
    BOT_PROFILE_PREFIX, BotError, BotEvent, BotRecord, BotService, BotTurnRunner, BotUpdate,
    MAX_BOT_DEPTH, MAX_BOTS_PER_WORKSPACE, is_bot_profile_id,
};
