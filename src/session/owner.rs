//! Derived session -> bot identity read model.
//!
//! The bot registry remains the only authority. This index is filled from the
//! registry on startup and refreshed on every bot commit/remove so all session
//! consumers share one answer without joining by agent_id or title.

/// The bot identity a session belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBotOwner {
    pub session_id: String,
    pub bot_id: String,
    pub name: String,
    pub description: String,
}
