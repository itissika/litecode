//! L1 persistence — session store and process-level session management.
//!
//! **Disk truth** is a transcript: `Transcript = Vec<Item>` (authority Responses Items).
//! Detail and `compact_checkpoint` rows both store serialized `Item` JSON in `body` /
//! `body_ref` in the `transcript_items` table (`item_type` = Responses Item type string;
//! `kind` = row envelope `detail` | `compact_checkpoint`).
//!
//! **Schema policy:** delete-and-rebuild only. Half-old `sessions.db` shapes fail closed;
//! delete `.litecode/sessions.db` to recreate. Empty leftover `messages` may be DROP'd.

pub mod estimate;
pub mod gate;
pub mod live;
pub mod manager;
pub mod media;
pub mod media_tokens;
pub mod snapshot;
pub mod snapshot_paths;
pub mod store;
pub mod task_state;
pub mod transcript_fts;
pub mod workspace_lock;

pub use estimate::{autocompact_threshold, compact_prompt, compute_token_estimate};
pub use gate::SessionGate;
pub use live::{LifecycleEvent, LiveTurn, TurnProgress};
pub use manager::{SessionManager, SessionRecord};
pub use media_tokens::{
    AUDIO_FALLBACK_TOKENS, FILE_FALLBACK_TOKENS, IMAGE_BASE_TOKENS, IMAGE_FALLBACK_TOKENS,
    IMAGE_TILE_TOKENS, VIDEO_FALLBACK_TOKENS, input_content_media_tokens,
};
pub use store::{Session, SessionContextMeter, TranscriptRow, data_root_from_db_path};
pub use task_state::{
    PlanRef, TaskReminders, TodoItem, TodoStatus, plan_dir, prune_stale_active_plan, render_todos,
};
pub use workspace_lock::WorkspaceLock;
