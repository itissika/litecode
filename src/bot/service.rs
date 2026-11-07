//! Workspace-local bot registry and product operations.
//!
//! The registry is a small JSON file next to `sessions.db`. It is the only
//! authority for bot identity and body; sessions stay ordinary sessions and the
//! session model never learns about bots. A bot's session points at the bot id,
//! never at a settings agent profile.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::config::DocId;
use crate::config::schema::AgentToolBinding;
use crate::config::settings_writer::SettingsWriter;
use crate::runtime::RuntimeHandle;
use crate::session::delete::{DeleteSessionError, RemovedSessionCleanup};
use crate::session::{SessionBotOwner, SessionService};
use crate::types::UserInput;

/// Current on-disk registry shape. Version 4 is single-authority: the record
/// owns the body and points at one ordinary session by `session_id`.
const BOT_REGISTRY_VERSION: u32 = 4;

/// Map the authoritative record into the session read model. The bot directory
/// stays the only source; this value is refreshed on every commit/remove.
fn bot_owner(record: &BotRecord) -> SessionBotOwner {
    SessionBotOwner {
        session_id: record.session_id.clone(),
        bot_id: record.id.clone(),
        name: record.name.clone(),
        description: record.description.clone(),
    }
}

/// Parent edge of a placement. Root/folder rows have no parent.
fn parent_of(bot: &BotRecord) -> Option<&str> {
    match &bot.position {
        BotPosition::Bot { id } => Some(id),
        _ => None,
    }
}

/// The org visibility set: self, ancestors, same-parent siblings, own subtree.
fn visible_bot_ids(bots: &[BotRecord], caller_id: &str) -> HashSet<String> {
    let mut visible = HashSet::new();
    let Some(caller) = bots.iter().find(|bot| bot.id == caller_id) else {
        return visible;
    };
    visible.insert(caller.id.clone());

    // Ancestor chain. The seen set also turns a corrupt placement cycle into a
    // finite read instead of an infinite walk.
    let mut seen = HashSet::from([caller.id.clone()]);
    let mut cursor = parent_of(caller);
    while let Some(parent_id) = cursor {
        if !seen.insert(parent_id.to_string()) {
            break;
        }
        let Some(parent) = bots.iter().find(|bot| bot.id == parent_id) else {
            break;
        };
        visible.insert(parent.id.clone());
        cursor = parent_of(parent);
    }

    // Same parent means sibling. None is also a parent group: every root is a
    // peer of every other root, but a root's children are not visible to a
    // different root's branch (cousins/nephews stay out of scope).
    let caller_parent = parent_of(caller);
    for bot in bots {
        if bot.id != caller.id && parent_of(bot) == caller_parent {
            visible.insert(bot.id.clone());
        }
    }

    // Own subtree (children recursively). Use an explicit stack and a seen set.
    let mut stack = vec![caller.id.clone()];
    while let Some(id) = stack.pop() {
        for child in bots
            .iter()
            .filter(|bot| parent_of(bot) == Some(id.as_str()))
        {
            if visible.insert(child.id.clone()) {
                stack.push(child.id.clone());
            }
        }
    }
    visible
}

/// Hard cap for one workspace. User-created and bot-created bots share it.
pub const MAX_BOTS_PER_WORKSPACE: usize = 16;
/// Bot folders are session-folder rows with this reserved id prefix. Sessions
/// never move into one; bots never move into an unprefixed folder.
pub const BOT_FOLDER_PREFIX: &str = "bot-folder-";
const MAX_BOT_NAME_CHARS: usize = 48;
const MAX_BOT_DESCRIPTION_CHARS: usize = 280;

/// A folder owned by the bot domain rather than the session domain.
pub fn is_bot_folder_id(id: &str) -> bool {
    id.starts_with(BOT_FOLDER_PREFIX)
}

/// Failure of one bot product operation.
#[derive(Debug, thiserror::Error)]
pub enum BotError {
    #[error("bot not found: {0}")]
    NotFound(String),
    #[error("a bot named '{0}' already exists")]
    NameTaken(String),
    #[error("bot name is required")]
    EmptyName,
    #[error("bot name is longer than {MAX_BOT_NAME_CHARS} characters")]
    NameTooLong,
    #[error("bot name contains control characters")]
    InvalidName,
    #[error("bot message is required")]
    EmptyMessage,
    #[error("more than one bot is named '{0}'; use the bot id")]
    AmbiguousTarget(String),
    #[error("this workspace already has {MAX_BOTS_PER_WORKSPACE} bots")]
    QuotaExceeded,
    #[error("this session is not a bot session")]
    NotBotSession,
    #[error("bot registry: {0}")]
    Registry(String),
    #[error("session: {0}")]
    Session(String),
    #[error("bot session is missing: {0}")]
    SessionMissing(String),
    #[error("this bot is running; stop it before changing capability or deleting it")]
    Busy,
    #[error("session folder not found: {0}")]
    FolderNotFound(String),
    #[error("folder is not owned by the bot domain: {0}")]
    FolderScope(String),
    #[error("parent bot not found: {0}")]
    ParentNotFound(String),
    #[error("a bot cannot be placed under itself or one of its descendants")]
    ParentCycle,
    #[error("bot settings: {0}")]
    Settings(String),
}

impl From<crate::types::LitecodeError> for BotError {
    fn from(error: crate::types::LitecodeError) -> Self {
        Self::Session(error.to_string())
    }
}

/// One durable bot identity and its single ordinary session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BotRecord {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub created_at: i64,
    /// The bot's one ordinary session. Empty only while a create is being
    /// reserved in-process. Legacy registries called this `default_session_id`.
    #[serde(
        default,
        alias = "default_session_id",
        skip_serializing_if = "String::is_empty"
    )]
    pub session_id: String,
    /// The authoritative capability body. Resolved into tool selection,
    /// permissions, and the system prompt through `bot::resolve_agent_for_session`.
    #[serde(flatten)]
    pub body: BotBody,
    /// Single placement: ungrouped, a bot folder, or another bot.
    #[serde(default, skip_serializing_if = "BotPosition::is_none")]
    pub position: BotPosition,
    /// Manual order inside the bot group. New bots get max + 1.
    #[serde(default)]
    pub sort_order: i64,
}

/// The bot-owned capability body. This is the one authority for a bot's prompt,
/// tool selection, and allowed subagents; settings profiles are read only for
/// the one-time cutover.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BotBody {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system_prompt: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tools: HashMap<String, AgentToolBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_subagents: Vec<String>,
}

/// Where a bot lives in the product tree. Exactly one placement; variants are
/// mutually exclusive by construction instead of nullable sibling fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BotPosition {
    #[default]
    None,
    Folder {
        id: String,
    },
    Bot {
        id: String,
    },
}

impl BotPosition {
    pub fn folder_id(&self) -> Option<&str> {
        match self {
            Self::Folder { id } => Some(id),
            _ => None,
        }
    }

    fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// One change to the workspace bot directory. Forwarded to every websocket as
/// `bot/changed`; the UI never needs a manual refresh after create/edit/delete.
#[derive(Debug, Clone)]
pub enum BotEvent {
    Created(BotRecord),
    Updated(BotRecord),
    Removed {
        bot_id: String,
        session_id: Option<String>,
    },
}

/// Fields accepted by `bot/update`. Identity, folder, and body are
/// independent: a running bot may be renamed, but its body cannot change.
#[derive(Debug, Default)]
pub struct BotUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub body: Option<BotBody>,
    pub folder_id: Option<String>,
    pub clear_folder: bool,
    /// Move under another bot. Clears a folder placement; validated for cycles.
    pub parent_bot_id: Option<String>,
    /// Return a bot-under-bot row to the ungrouped root.
    pub clear_parent: bool,
    pub sort_order: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BotRegistryFile {
    version: u32,
    #[serde(default)]
    bots: Vec<BotRecord>,
}

/// Product-layer callback that starts a pending turn for a bot session.
///
/// Installed by serve state so this module does not own a second runtime or a
/// reference cycle back into `RuntimeHandle`.
pub type BotTurnRunner = Arc<dyn Fn(&str) + Send + Sync>;

/// Workspace-local bot directory and delivery facade.
pub struct BotService {
    service: Arc<SessionService>,
    /// Weak to avoid RuntimeHandle -> BotService -> RuntimeHandle.
    runtime: Option<Weak<RwLock<RuntimeHandle>>>,
    project: String,
    path: PathBuf,
    state: Mutex<Vec<BotRecord>>,
    events: broadcast::Sender<BotEvent>,
    turn_runner: OnceLock<BotTurnRunner>,
}

impl BotService {
    pub fn new(
        service: Arc<SessionService>,
        settings: Arc<SettingsWriter>,
        runtime: Option<Weak<RwLock<RuntimeHandle>>>,
        project: impl Into<String>,
        path: PathBuf,
    ) -> Result<Self, BotError> {
        // Loading also promotes the legacy `folder_id` key into `position`
        // and collects the old identity-profile ids for one-time cleanup.
        let loaded = load_registry(&path)?;
        let mut bots = loaded.bots;
        let mut changed = loaded.legacy_shape;
        let mut docs = Vec::new();

        for bot in &mut bots {
            // Folders created before bot folders existed are session folders.
            // Ungroup those bots instead of leaving a cross-domain placement.
            if bot
                .position
                .folder_id()
                .is_some_and(|folder| !is_bot_folder_id(folder))
            {
                bot.position = BotPosition::None;
                changed = true;
            }
        }

        // Placement is a tree edge. An old or hand-edited registry may point at
        // a missing bot or contain a cycle; ungroup that row rather than making
        // the sidebar recurse forever.
        let ids: HashSet<String> = bots.iter().map(|bot| bot.id.clone()).collect();
        let parents: HashMap<String, String> = bots
            .iter()
            .filter_map(|bot| match &bot.position {
                BotPosition::Bot { id } => Some((bot.id.clone(), id.clone())),
                _ => None,
            })
            .collect();
        for bot in &mut bots {
            let Some(parent) = parents.get(&bot.id).cloned() else {
                continue;
            };
            let mut cursor = Some(parent);
            let mut seen = HashSet::new();
            while let Some(id) = cursor {
                if id == bot.id || !ids.contains(&id) || !seen.insert(id.clone()) {
                    bot.position = BotPosition::None;
                    changed = true;
                    break;
                }
                cursor = parents.get(&id).cloned();
            }
        }

        // A record written before the built-in prompt expansion may still hold
        // a marker. Expand it in place; expanded text is not a marker, so this
        // pass is idempotent.
        for bot in &mut bots {
            let expanded = super::expand_builtin_prompt(&bot.body.system_prompt);
            if expanded != bot.body.system_prompt {
                bot.body.system_prompt = expanded;
                changed = true;
            }
        }

        // A record without a live session is reported, never deleted or
        // recreated here. Its children keep their exact positions. Only an
        // explicit bot delete may remove the record.
        for bot in &bots {
            if bot.session_id.is_empty() {
                tracing::warn!(bot_id = %bot.id, "bot record has no bound session");
                continue;
            }
            match service.meta(&bot.session_id) {
                Ok(_) => {}
                Err(crate::types::LitecodeError::SessionNotFound(_)) => {
                    tracing::warn!(
                        bot_id = %bot.id,
                        session_id = %bot.session_id,
                        "bot record points at a missing session"
                    );
                }
                Err(error) => return Err(BotError::Session(error.to_string())),
            }
        }

        // One-time identity migration: the bot session's agent id becomes the
        // bot id. Startup has no live turn, so this repair is unconditional.
        for bot in &bots {
            if bot.session_id.is_empty() {
                continue;
            }
            match service.meta(&bot.session_id) {
                Ok(meta) => {
                    if meta.agent_id != bot.id {
                        service.bind_bot_agent_id(&bot.session_id, bot.id.clone())?;
                    }
                }
                Err(crate::types::LitecodeError::SessionNotFound(_)) => {}
                Err(error) => return Err(BotError::Session(error.to_string())),
            }
        }

        // Delete the old identity profiles named by the v3 records. Never
        // delete a protected agent or one still referenced by another session.
        if !loaded.legacy_profile_ids.is_empty() {
            let settings_snapshot = settings
                .load_settings()
                .map_err(|error| BotError::Settings(error.to_string()))?;
            let sessions = service
                .list_sessions()
                .map_err(|error| BotError::Session(error.to_string()))?;
            for profile_id in loaded.legacy_profile_ids {
                if profile_id.is_empty()
                    || crate::config::schema::PROTECTED_AGENT_IDS.contains(&profile_id.as_str())
                    || !settings_snapshot.agents.contains_key(&profile_id)
                    || sessions.iter().any(|row| row.agent_id == profile_id)
                {
                    continue;
                }
                match settings.delete_agent(&profile_id) {
                    Ok(ack) => docs.extend(ack.docs),
                    Err(error) => tracing::warn!(
                        profile = %profile_id,
                        %error,
                        "legacy bot identity profile was not deleted"
                    ),
                }
            }
        }

        if changed {
            // Rewrite the pre-v4 shape only after the session rebind and legacy
            // profile cleanup succeeded. A failed save leaves the old file for
            // the next start to retry.
            save_registry(&path, &bots)?;
        }

        service.replace_bot_owners(
            bots.iter()
                .filter(|bot| !bot.session_id.is_empty())
                .map(bot_owner),
        );

        let (events, _) = broadcast::channel(64);
        let service = Self {
            service,
            runtime,
            project: project.into(),
            path,
            state: Mutex::new(bots),
            events,
            turn_runner: OnceLock::new(),
        };
        service.apply_docs(&docs);
        Ok(service)
    }

    /// Serve state installs the runtime-backed delivery hook once.
    pub fn set_turn_runner(&self, runner: BotTurnRunner) {
        let _ = self.turn_runner.set(runner);
    }

    pub fn service(&self) -> &Arc<SessionService> {
        &self.service
    }

    /// Subscribe to bot lifecycle changes for websocket forwarding.
    pub fn subscribe(&self) -> broadcast::Receiver<BotEvent> {
        self.events.subscribe()
    }

    /// Committed bots, ordered by the shared folder/order fields.
    pub fn list(&self) -> Vec<BotRecord> {
        let mut bots: Vec<BotRecord> = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|bot| !bot.session_id.is_empty())
            .cloned()
            .collect();
        sort_records(&mut bots);
        bots
    }

    /// Bots visible to one caller: self, ancestors, same-parent siblings, and
    /// the caller's own subtree. A cousin branch is deliberately absent.
    pub fn list_for_caller(&self, caller_id: &str) -> Vec<BotRecord> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let visible = visible_bot_ids(&state, caller_id);
        let mut bots: Vec<BotRecord> = state
            .iter()
            .filter(|bot| visible.contains(&bot.id) && !bot.session_id.is_empty())
            .cloned()
            .collect();
        sort_records(&mut bots);
        bots
    }

    /// Whether `target_id` is in the caller's visibility scope.
    pub fn can_see(&self, caller_id: &str, target_id: &str) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        visible_bot_ids(&state, caller_id).contains(target_id)
    }

    /// By id even when the binding is empty or its session is missing. Used by
    /// open/delete so a dangling record is reported or explicitly removable.
    fn find_by_id_any(&self, bot_id: &str) -> Option<BotRecord> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .find(|bot| bot.id == bot_id)
            .cloned()
    }

    /// The bot whose session is `session_id`.
    pub fn bot_for_session(&self, session_id: &str) -> Option<BotRecord> {
        if session_id.is_empty() {
            return None;
        }
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .find(|bot| bot.session_id == session_id)
            .cloned()
    }

    /// Find by exact id or case-insensitive name.
    pub fn find(&self, id_or_name: &str) -> Option<BotRecord> {
        let needle = id_or_name.trim();
        if needle.is_empty() {
            return None;
        }
        let folded = needle.to_lowercase();
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|bot| !bot.session_id.is_empty())
            .find(|bot| bot.id == needle || bot.name.to_lowercase() == folded)
            .cloned()
    }

    /// User-created bot: ungrouped, body supplied by the caller.
    pub async fn create_user_bot(
        &self,
        name: &str,
        description: &str,
        body: BotBody,
    ) -> Result<BotRecord, BotError> {
        self.create(name, description, body, BotPosition::None)
            .await
    }

    /// Bot-created bot: inherits a copy of the caller's body and is placed
    /// under the caller. It never gets a settings agent profile.
    pub async fn create_from_bot(
        &self,
        caller_session_id: &str,
        name: &str,
        description: &str,
    ) -> Result<BotRecord, BotError> {
        let caller = self
            .bot_for_session(caller_session_id)
            .ok_or(BotError::NotBotSession)?;
        self.create(
            name,
            description,
            caller.body.clone(),
            BotPosition::Bot {
                id: caller.id.clone(),
            },
        )
        .await
    }

    async fn create(
        &self,
        name: &str,
        description: &str,
        body: BotBody,
        position: BotPosition,
    ) -> Result<BotRecord, BotError> {
        let body = BotBody {
            system_prompt: super::expand_builtin_prompt(&body.system_prompt),
            ..body
        };
        let name = normalize_name(name)?;
        let description = normalize_description(description);

        // Reserve the identity before the async session create so concurrent
        // creates cannot mint two bots with the same name or exceed the quota.
        let record = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state
                .iter()
                .any(|bot| bot.name.to_lowercase() == name.to_lowercase())
            {
                return Err(BotError::NameTaken(name));
            }
            if state.len() >= MAX_BOTS_PER_WORKSPACE {
                return Err(BotError::QuotaExceeded);
            }
            let record = BotRecord {
                id: ulid::Ulid::new().to_string(),
                name,
                description,
                created_at: now_ms(),
                session_id: String::new(),
                body,
                position,
                sort_order: state.iter().map(|bot| bot.sort_order).max().unwrap_or(0) + 1,
            };
            state.push(record.clone());
            record
        };

        // A cancelled create (for example, a dropped RPC future) must not leave
        // a reservation or a session behind.
        let mut reservation = Reservation {
            service: self,
            id: record.id.clone(),
            session_id: None,
            committed: false,
        };

        // The session's agent id is the bot id itself; there is no profile.
        let model_ref = self.model_for(&record.id);
        let session_id = match self
            .service
            .open_session(&self.project, &record.id, model_ref.as_deref())
            .await
        {
            Ok(session_id) => session_id,
            Err(error) => return Err(error.into()),
        };
        reservation.session_id = Some(session_id.clone());

        let mut committed = record;
        committed.session_id = session_id;
        if let Err(error) = self.commit(committed.clone()) {
            if self.discard_session(&committed.session_id).await {
                reservation.session_id = None;
            }
            return Err(error);
        }
        reservation.committed = true;
        self.emit(BotEvent::Created(committed.clone()));
        Ok(committed)
    }

    /// Open a bot only when its one session still exists. Missing sessions are
    /// reported; a bot session is never recreated on read.
    pub fn open(&self, bot_id: &str) -> Result<BotRecord, BotError> {
        let record = self
            .find_by_id_any(bot_id)
            .ok_or_else(|| BotError::NotFound(bot_id.to_string()))?;
        if !self.session_exists(&record.session_id)? {
            return Err(BotError::SessionMissing(record.session_id));
        }
        Ok(record)
    }

    /// Validate one bot-under-bot placement: the parent must exist and the
    /// moving node must not appear anywhere in its ancestor chain.
    fn parent_position(&self, bot_id: &str, parent_id: &str) -> Result<BotPosition, BotError> {
        if bot_id == parent_id {
            return Err(BotError::ParentCycle);
        }
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.iter().any(|bot| bot.id == parent_id) {
            return Err(BotError::ParentNotFound(parent_id.to_string()));
        }
        let mut cursor = Some(parent_id.to_string());
        let mut seen = HashSet::new();
        while let Some(id) = cursor {
            if id == bot_id || !seen.insert(id.clone()) {
                return Err(BotError::ParentCycle);
            }
            cursor = state
                .iter()
                .find(|bot| bot.id == id)
                .and_then(|bot| match &bot.position {
                    BotPosition::Bot { id } => Some(id.clone()),
                    _ => None,
                });
        }
        Ok(BotPosition::Bot {
            id: parent_id.to_string(),
        })
    }

    /// Update identity, placement/order, and/or the authoritative body.
    /// Body changes are rejected while the bot's own turn is running.
    pub async fn update(&self, bot_id: &str, update: BotUpdate) -> Result<BotRecord, BotError> {
        let current = self
            .find(bot_id)
            .ok_or_else(|| BotError::NotFound(bot_id.to_string()))?;
        let mut next = current.clone();

        if let Some(name) = update.name {
            let name = normalize_name(&name)?;
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state
                .iter()
                .any(|bot| bot.id != current.id && bot.name.to_lowercase() == name.to_lowercase())
            {
                return Err(BotError::NameTaken(name));
            }
            drop(state);
            next.name = name;
        }
        if let Some(description) = update.description {
            next.description = normalize_description(&description);
        }
        if update.clear_folder || update.clear_parent {
            next.position = BotPosition::None;
        }
        if let Some(folder_id) = update.folder_id {
            if !self
                .service
                .list_folders()
                .map_err(|error| BotError::Session(error.to_string()))?
                .iter()
                .any(|folder| folder.id == folder_id)
            {
                return Err(BotError::FolderNotFound(folder_id));
            }
            if !is_bot_folder_id(&folder_id) {
                return Err(BotError::FolderScope(folder_id));
            }
            next.position = BotPosition::Folder { id: folder_id };
        }
        if let Some(parent_bot_id) = update.parent_bot_id {
            next.position = self.parent_position(&current.id, &parent_bot_id)?;
        }
        if let Some(sort_order) = update.sort_order {
            next.sort_order = sort_order;
        }

        if let Some(body) = update.body {
            if !current.session_id.is_empty()
                && self.service.is_session_busy_blocking(&current.session_id)
            {
                return Err(BotError::Busy);
            }
            next.body = BotBody {
                system_prompt: super::expand_builtin_prompt(&body.system_prompt),
                ..body
            };
        }

        self.commit(next.clone())?;
        self.emit(BotEvent::Updated(next.clone()));
        Ok(next)
    }

    /// Delete the bot and its one session. Children take its position. A
    /// running bot is never deleted silently.
    pub async fn delete(&self, bot_id: &str) -> Result<BotRecord, BotError> {
        let record = self
            .find_by_id_any(bot_id)
            .ok_or_else(|| BotError::NotFound(bot_id.to_string()))?;
        if !record.session_id.is_empty()
            && self.service.is_session_busy_blocking(&record.session_id)
        {
            return Err(BotError::Busy);
        }

        if !record.session_id.is_empty() && self.session_exists(&record.session_id)? {
            let hub = self.subagent_hub();
            let cleanup = |session_id: &str| {
                if let Some(hub) = &hub {
                    RemovedSessionCleanup::on_session_removed(hub.as_ref(), session_id);
                }
            };
            match self
                .service
                .delete_bot_session_tree(&cleanup, &record.session_id)
                .await
            {
                Ok(_) => {}
                Err(DeleteSessionError::AlreadyRunning)
                | Err(DeleteSessionError::DescendantRunning { .. }) => {
                    return Err(BotError::Busy);
                }
                Err(error) => return Err(BotError::Session(error.to_string())),
            }
        }

        let (removed, promoted) = self.remove_record_with_promotion(&record.id)?;
        for bot in promoted {
            self.emit(BotEvent::Updated(bot));
        }
        self.emit(BotEvent::Removed {
            bot_id: removed.id.clone(),
            session_id: (!removed.session_id.is_empty()).then(|| removed.session_id.clone()),
        });
        Ok(removed)
    }

    /// Session folders are shared. Deleting a folder ungroups bots that used it
    /// so the UI never keeps a dangling folder id.
    pub fn clear_folder(&self, folder_id: &str) {
        let updated = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let mut next = state.clone();
            let mut changed_ids = HashSet::new();
            for bot in &mut next {
                if bot.position.folder_id() == Some(folder_id) {
                    bot.position = BotPosition::None;
                    changed_ids.insert(bot.id.clone());
                }
            }
            if changed_ids.is_empty() {
                return;
            }
            if let Err(error) = save_registry(&self.path, &next) {
                tracing::warn!(
                    folder_id,
                    %error,
                    "could not ungroup bots after folder deletion"
                );
                return;
            }
            *state = next;
            state
                .iter()
                .filter(|bot| changed_ids.contains(&bot.id))
                .cloned()
                .collect::<Vec<_>>()
        };
        for bot in updated {
            self.emit(BotEvent::Updated(bot));
        }
    }

    /// Queue a message into the target bot's session and return immediately.
    ///
    /// The target turn is started by the installed runner when the session is
    /// idle; a busy session consumes the queue at its next request seam or when
    /// its current turn finishes. This call never waits for a reply.
    pub async fn send_from_bot(
        &self,
        caller_session_id: &str,
        target: &str,
        message: &str,
    ) -> Result<BotRecord, BotError> {
        let caller = self
            .bot_for_session(caller_session_id)
            .ok_or(BotError::NotBotSession)?;
        let message = message.trim();
        if message.is_empty() {
            return Err(BotError::EmptyMessage);
        }
        let target = self.resolve_target_for(&caller.id, target)?;
        if !self.session_exists(&target.session_id)? {
            return Err(BotError::SessionMissing(target.session_id));
        }

        let text = format!(
            "[from bot id={} name={}]\n{}",
            caller.id, caller.name, message
        );
        self.service.enqueue_user_input(
            &target.session_id,
            UserInput::from_bot(text, caller.id, caller.name),
        )?;
        if let Some(runner) = self.turn_runner.get() {
            runner(&target.session_id);
        }
        Ok(target)
    }

    /// Resolve a bot_send target inside the caller's visibility scope. An exact
    /// id wins; a name must be unique among visible bots. An out-of-scope
    /// target is reported exactly like a missing one.
    fn resolve_target_for(&self, caller_id: &str, target: &str) -> Result<BotRecord, BotError> {
        let needle = target.trim();
        if needle.is_empty() {
            return Err(BotError::NotFound(target.to_string()));
        }
        let visible = self.list_for_caller(caller_id);
        if let Some(bot) = visible.iter().find(|bot| bot.id == needle) {
            return Ok(bot.clone());
        }
        let folded = needle.to_lowercase();
        let matches: Vec<&BotRecord> = visible
            .iter()
            .filter(|bot| bot.name.to_lowercase() == folded)
            .collect();
        match matches.as_slice() {
            [bot] => Ok((*bot).clone()),
            [] => Err(BotError::NotFound(needle.to_string())),
            _ => Err(BotError::AmbiguousTarget(needle.to_string())),
        }
    }

    /// Durable existence check. status only sees sessions hydrated in this
    /// process, so a restart must not treat an on-disk session as gone.
    fn session_exists(&self, session_id: &str) -> Result<bool, BotError> {
        if session_id.is_empty() {
            return Ok(false);
        }
        match self.service.meta(session_id) {
            Ok(_) => Ok(true),
            Err(crate::types::LitecodeError::SessionNotFound(_)) => Ok(false),
            Err(error) => Err(BotError::Session(error.to_string())),
        }
    }

    fn apply_docs(&self, docs: &[DocId]) {
        if docs.is_empty() {
            return;
        }
        let Some(runtime) = self.runtime.as_ref().and_then(Weak::upgrade) else {
            return;
        };
        let mut runtime = runtime.write().unwrap_or_else(|error| error.into_inner());
        if let Err(error) = runtime.apply(docs) {
            tracing::warn!(%error, "bot settings: runtime apply failed");
        }
        runtime.sync_workspace_tool_readiness();
    }

    fn subagent_hub(&self) -> Option<Arc<crate::tools::subagent::SubagentHub>> {
        let runtime = self.runtime.as_ref().and_then(Weak::upgrade)?;
        let runtime = runtime.read().ok()?;
        Some(Arc::clone(&runtime.subagent_hub))
    }

    /// Seed the session with the first runnable model. The bot body has no
    /// model; only the session row does.
    fn model_for(&self, bot_id: &str) -> Option<String> {
        let runtime = self.runtime.as_ref().and_then(Weak::upgrade)?;
        let runtime = runtime.read().ok()?;
        crate::runtime::seed_model_ref(&runtime.resolved, bot_id)
    }

    /// Best-effort removal of a session opened for a bot that could not be
    /// recorded. The session is brand new and idle; cleanup failure is logged
    /// but never masks the registry error.
    async fn discard_session(&self, session_id: &str) -> bool {
        match self
            .service
            .delete_bot_session_tree(&|_: &str| {}, session_id)
            .await
        {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(
                    session_id,
                    %error,
                    "could not remove the unowned bot session"
                );
                false
            }
        }
    }

    fn commit(&self, record: BotRecord) -> Result<(), BotError> {
        let owner = bot_owner(&record);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let slot = state
            .iter_mut()
            .find(|bot| bot.id == record.id)
            .ok_or_else(|| BotError::NotFound(record.id.clone()))?;
        let previous = slot.clone();
        *slot = record;
        if let Err(error) = save_registry(&self.path, &state) {
            // Keep memory and disk agreeing: a failed write cannot leave an
            // address that the next restart will not know about.
            if let Some(slot) = state.iter_mut().find(|bot| bot.id == previous.id) {
                *slot = previous;
            }
            return Err(error);
        }
        self.service.upsert_bot_owner(owner);
        Ok(())
    }

    /// Remove the record and re-place its direct children at the removed
    /// bot's own position. The registry write is atomic; memory follows disk.
    fn remove_record_with_promotion(
        &self,
        bot_id: &str,
    ) -> Result<(BotRecord, Vec<BotRecord>), BotError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut next = state.clone();
        let index = next
            .iter()
            .position(|bot| bot.id == bot_id)
            .ok_or_else(|| BotError::NotFound(bot_id.to_string()))?;
        let removed = next.remove(index);
        let replacement = removed.position.clone();
        let mut promoted = Vec::new();
        for bot in &mut next {
            if bot.position
                == (BotPosition::Bot {
                    id: removed.id.clone(),
                })
            {
                bot.position = replacement.clone();
                promoted.push(bot.clone());
            }
        }
        if let Err(error) = save_registry(&self.path, &next) {
            return Err(error);
        }
        *state = next;
        self.service.remove_bot_owner(&removed.session_id);
        Ok((removed, promoted))
    }

    /// Remove an uncommitted in-memory reservation. The background cleanup
    /// still removes any session that was already opened.
    fn remove_reserved(&self, bot_id: &str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let removed_session = state
            .iter()
            .find(|bot| bot.id == bot_id)
            .map(|bot| bot.session_id.clone());
        state.retain(|bot| bot.id != bot_id);
        if let Err(error) = save_registry(&self.path, &state) {
            tracing::warn!(bot_id, %error, "could not rewrite bot registry after reservation removal");
        }
        if let Some(session_id) = removed_session {
            self.service.remove_bot_owner(&session_id);
        }
    }

    fn emit(&self, event: BotEvent) {
        let _ = self.events.send(event);
    }
}

/// Remove an uncommitted in-memory reservation on every exit path, including
/// async cancellation of the create call. A session delete is handed to a
/// background task because Drop cannot await.
struct Reservation<'a> {
    service: &'a BotService,
    id: String,
    session_id: Option<String>,
    committed: bool,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.service.remove_reserved(&self.id);
        let Some(session_id) = self.session_id.take() else {
            return;
        };
        let service = Arc::clone(self.service.service());
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(
                %session_id,
                "cancelled bot create left a session and no runtime is available to clean it up"
            );
            return;
        };
        handle.spawn(async move {
            if let Err(error) = service
                .delete_bot_session_tree(&|_: &str| {}, &session_id)
                .await
            {
                tracing::warn!(
                    %session_id,
                    %error,
                    "could not remove the cancelled bot session"
                );
            }
        });
    }
}

fn sort_records(bots: &mut [BotRecord]) {
    bots.sort_by(|left, right| {
        left.sort_order
            .cmp(&right.sort_order)
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn normalize_name(name: &str) -> Result<String, BotError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(BotError::EmptyName);
    }
    if name.chars().count() > MAX_BOT_NAME_CHARS {
        return Err(BotError::NameTooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(BotError::InvalidName);
    }
    Ok(name.to_string())
}

fn normalize_description(description: &str) -> String {
    description
        .trim()
        .chars()
        .take(MAX_BOT_DESCRIPTION_CHARS)
        .collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

struct LoadedRegistry {
    bots: Vec<BotRecord>,
    /// The file still uses a pre-v4 shape and must be rewritten.
    legacy_shape: bool,
    /// Identity-profile ids named by a v3 record. They are deleted once the
    /// session no longer references them.
    legacy_profile_ids: Vec<String>,
}

fn load_registry(path: &Path) -> Result<LoadedRegistry, BotError> {
    if !path.exists() {
        return Ok(LoadedRegistry {
            bots: Vec::new(),
            legacy_shape: false,
            legacy_profile_ids: Vec::new(),
        });
    }
    let text = fs::read_to_string(path)
        .map_err(|error| BotError::Registry(format!("read {}: {error}", path.display())))?;
    if text.trim().is_empty() {
        return Ok(LoadedRegistry {
            bots: Vec::new(),
            legacy_shape: false,
            legacy_profile_ids: Vec::new(),
        });
    }
    let mut value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| BotError::Registry(format!("parse {}: {error}", path.display())))?;
    let legacy_position = promote_legacy_folder_ids(&mut value);
    let legacy_profile_ids = collect_legacy_profile_ids(&value);
    let file: BotRegistryFile = serde_json::from_value(value)
        .map_err(|error| BotError::Registry(format!("parse {}: {error}", path.display())))?;
    let legacy_shape =
        file.version < BOT_REGISTRY_VERSION || legacy_position || !legacy_profile_ids.is_empty();
    Ok(LoadedRegistry {
        bots: file.bots,
        legacy_shape,
        legacy_profile_ids,
    })
}

/// Collect the old `profile_id` / `agent_id` values before serde drops them.
fn collect_legacy_profile_ids(file: &serde_json::Value) -> Vec<String> {
    let Some(bots) = file.get("bots").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for bot in bots {
        let Some(object) = bot.as_object() else {
            continue;
        };
        for key in ["profile_id", "agent_id"] {
            if let Some(id) = object.get(key).and_then(serde_json::Value::as_str) {
                if !id.is_empty() {
                    ids.push(id.to_string());
                    break;
                }
            }
        }
    }
    ids
}

/// Accept the pre-`position` registry shape. Returns true when a legacy
/// `folder_id` key was present, so the caller rewrites the file once.
fn promote_legacy_folder_ids(file: &mut serde_json::Value) -> bool {
    let Some(bots) = file
        .get_mut("bots")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return false;
    };
    let mut changed = false;
    for bot in bots {
        let Some(object) = bot.as_object_mut() else {
            continue;
        };
        let Some(folder) = object.remove("folder_id") else {
            continue;
        };
        changed = true;
        if object.contains_key("position") {
            continue;
        }
        if let Some(folder) = folder.as_str().filter(|folder| !folder.is_empty()) {
            object.insert(
                "position".to_string(),
                serde_json::json!({ "kind": "folder", "id": folder }),
            );
        }
    }
    changed
}

fn registry_prev_path(path: &Path) -> PathBuf {
    path.with_extension("json.prev")
}

/// Restore the file this failed write started from. `prev` is copied before
/// every replace, so a Windows remove+rename failure never falls back to an
/// older migration snapshot.
fn restore_registry(path: &Path, prev: &Path) {
    if !prev.exists() {
        return;
    }
    if let Err(error) = fs::copy(prev, path) {
        tracing::warn!(
            prev = %prev.display(),
            path = %path.display(),
            %error,
            "bot registry write rollback failed"
        );
    }
}

fn save_registry(path: &Path, bots: &[BotRecord]) -> Result<(), BotError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| BotError::Registry(format!("create {}: {error}", parent.display())))?;
    }
    let file = BotRegistryFile {
        version: BOT_REGISTRY_VERSION,
        bots: bots
            .iter()
            .filter(|bot| !bot.session_id.is_empty())
            .cloned()
            .collect(),
    };
    let body = serde_json::to_string_pretty(&file)
        .map_err(|error| BotError::Registry(format!("serialize bot registry: {error}")))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body)
        .map_err(|error| BotError::Registry(format!("write {}: {error}", tmp.display())))?;
    let prev = registry_prev_path(path);
    let had_original = path.exists();
    if had_original {
        fs::copy(path, &prev)
            .map_err(|error| BotError::Registry(format!("backup {}: {error}", prev.display())))?;
    }
    if let Err(error) = replace_registry(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        if had_original {
            restore_registry(path, &prev);
        }
        return Err(BotError::Registry(format!(
            "replace {}: {error}",
            path.display()
        )));
    }
    Ok(())
}

/// Replace the registry file. Windows cannot rename over an existing file, so
/// the fallback removes it first; a failure in that window is handled by the
/// caller restoring the per-write `prev` copy.
fn replace_registry(tmp: &Path, path: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if FAIL_REGISTRY_REPLACE.with(std::cell::Cell::get) {
        FAIL_REGISTRY_REPLACE.with(|flag| flag.set(false));
        let _ = fs::remove_file(path);
        return Err(std::io::Error::other("injected registry replace failure"));
    }
    match fs::rename(tmp, path) {
        Ok(()) => Ok(()),
        Err(first) => {
            if path.exists() {
                fs::remove_file(path)?;
            }
            fs::rename(tmp, path)
                .map_err(|second| std::io::Error::new(second.kind(), format!("{first}; {second}")))
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only seam: fail the next replace after the target file is removed,
    /// mirroring the Windows remove+rename window.
    static FAIL_REGISTRY_REPLACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnGuard;
    use crate::config::schema::{AgentProfile, AgentRole};
    use crate::session::SessionManager;
    use tempfile::TempDir;

    struct Fixture {
        dir: TempDir,
        manager: Arc<SessionManager>,
        settings: Arc<SettingsWriter>,
        bots: Arc<BotService>,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            dir.path().join("sessions.db").to_string_lossy().to_string(),
        ));
        let service = Arc::new(SessionService::new(Arc::clone(&manager)));
        let settings = Arc::new(SettingsWriter::with_path(
            dir.path().join("settings.db"),
            Arc::new(TurnGuard::new()),
        ));
        settings
            .write_agent(
                "default",
                AgentProfile {
                    role: AgentRole::Primary,
                    ..AgentProfile::default()
                },
            )
            .expect("template agent");
        let bots = Arc::new(
            BotService::new(
                service,
                Arc::clone(&settings),
                None,
                "/proj",
                dir.path().join("bots.json"),
            )
            .expect("bot service"),
        );
        Fixture {
            dir,
            manager,
            settings,
            bots,
        }
    }

    fn body(prompt: &str) -> BotBody {
        BotBody {
            system_prompt: prompt.into(),
            ..BotBody::default()
        }
    }

    fn body_with_tool(prompt: &str, tool: &str) -> BotBody {
        BotBody {
            system_prompt: prompt.into(),
            tools: HashMap::from([(
                tool.to_string(),
                AgentToolBinding {
                    enabled: true,
                    preset: None,
                    allowed_tools: None,
                },
            )]),
            ..BotBody::default()
        }
    }

    fn resolved(f: &Fixture) -> crate::config::ResolvedConfig {
        crate::config::ConfigManager::resolve_without_catalog(
            f.settings.load_settings().expect("settings"),
            crate::config::WorkspaceState::new(f.dir.path()),
        )
    }

    #[tokio::test]
    async fn create_persists_identity_and_session_without_a_profile() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Researcher", "finds papers", body("hello"))
            .await
            .expect("create");

        assert!(!bot.session_id.is_empty());
        assert_eq!(f.bots.bot_for_session(&bot.session_id), Some(bot.clone()));
        assert_eq!(f.bots.find("researcher"), Some(bot.clone()));
        assert_eq!(
            f.bots
                .service()
                .meta(&bot.session_id)
                .expect("session")
                .agent_id,
            bot.id
        );
        let settings = f.settings.load_settings().expect("settings");
        assert!(!settings.agents.contains_key(&bot.id));
    }

    #[tokio::test]
    async fn duplicate_names_and_quota_are_rejected() {
        let f = fixture();
        f.bots
            .create_user_bot("One", "", body(""))
            .await
            .expect("first");
        let error = f
            .bots
            .create_user_bot("one", "", body(""))
            .await
            .expect_err("duplicate");
        assert!(matches!(error, BotError::NameTaken(_)));

        for index in 1..MAX_BOTS_PER_WORKSPACE {
            f.bots
                .create_user_bot(&format!("Bot {index}"), "", body(""))
                .await
                .expect("under quota");
        }
        let error = f
            .bots
            .create_user_bot("Overflow", "", body(""))
            .await
            .expect_err("quota");
        assert!(matches!(error, BotError::QuotaExceeded));
    }

    #[tokio::test]
    async fn bot_created_bot_gets_the_callers_body_and_position() {
        let f = fixture();
        let parent = f
            .bots
            .create_user_bot("Parent", "", body_with_tool("parent prompt", "read"))
            .await
            .expect("parent");
        let child = f
            .bots
            .create_from_bot(&parent.session_id, "Child", "")
            .await
            .expect("child");

        assert_eq!(
            child.position,
            BotPosition::Bot {
                id: parent.id.clone()
            }
        );
        assert_eq!(child.body, parent.body);
        assert_eq!(
            f.bots
                .service()
                .meta(&child.session_id)
                .expect("child session")
                .agent_id,
            child.id
        );
        assert!(
            !f.settings
                .load_settings()
                .expect("settings")
                .agents
                .contains_key(&child.id)
        );
    }

    #[tokio::test]
    async fn deleting_a_parent_replaces_its_position_for_children() {
        let f = fixture();
        let folder_id = f
            .bots
            .service()
            .create_session_folder("Team", true)
            .expect("bot folder");
        let parent = f
            .bots
            .create_user_bot("Parent", "", body("parent"))
            .await
            .expect("parent");
        f.bots
            .update(
                &parent.id,
                BotUpdate {
                    folder_id: Some(folder_id.clone()),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect("move parent");
        let child = f
            .bots
            .create_from_bot(&parent.session_id, "Child", "")
            .await
            .expect("child");

        f.bots.delete(&parent.id).await.expect("delete parent");
        assert_eq!(
            f.bots.find(&child.id).expect("child").position,
            BotPosition::Folder {
                id: folder_id.clone()
            }
        );
        assert!(f.bots.service().status(&parent.session_id).is_none());
        assert!(f.bots.service().status(&child.session_id).is_some());

        let root = f
            .bots
            .create_user_bot("Root", "", body("root"))
            .await
            .expect("root");
        let root_child = f
            .bots
            .create_from_bot(&root.session_id, "Root child", "")
            .await
            .expect("root child");
        f.bots.delete(&root.id).await.expect("delete root");
        assert_eq!(
            f.bots.find(&root_child.id).expect("root child").position,
            BotPosition::None
        );
    }

    #[tokio::test]
    async fn missing_bot_session_errors_and_is_not_recreated() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Gone", "", body(""))
            .await
            .expect("bot");
        f.bots
            .service()
            .delete_bot_session_tree(&|_: &str| {}, &bot.session_id)
            .await
            .expect("delete session");

        let error = f.bots.open(&bot.id).expect_err("missing session");
        assert!(matches!(error, BotError::SessionMissing(_)));
        assert!(
            f.bots
                .service()
                .list_sessions()
                .expect("sessions")
                .is_empty(),
            "open must not recreate the session"
        );
    }

    #[tokio::test]
    async fn legacy_identity_profile_is_deleted_after_session_rebind() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Legacy", "", body("record body"))
            .await
            .expect("bot");
        f.manager
            .set_agent_id(&bot.session_id, "legacy_identity".into())
            .expect("old agent id");
        f.settings
            .write_agent(
                "legacy_identity",
                AgentProfile {
                    role: AgentRole::Primary,
                    system_prompt: "legacy profile body".into(),
                    ..AgentProfile::default()
                },
            )
            .expect("legacy profile");

        let registry = f.dir.path().join("bots.json");
        let mut file: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&registry).expect("registry")).expect("json");
        let row = file["bots"]
            .as_array_mut()
            .expect("bots")
            .iter_mut()
            .find(|row| row["id"] == bot.id)
            .expect("bot row");
        row.as_object_mut()
            .expect("record")
            .insert("profile_id".into(), serde_json::json!("legacy_identity"));
        fs::write(
            &registry,
            serde_json::to_string_pretty(&file).expect("json"),
        )
        .expect("write registry");

        let restarted = BotService::new(
            Arc::clone(f.bots.service()),
            Arc::clone(&f.settings),
            None,
            "/proj",
            registry.clone(),
        )
        .expect("restart");
        let opened = restarted.open(&bot.id).expect("open");
        assert_eq!(opened.body.system_prompt, "record body");
        assert_eq!(
            f.bots
                .service()
                .meta(&bot.session_id)
                .expect("session")
                .agent_id,
            bot.id
        );
        let settings = f.settings.load_settings().expect("settings");
        assert!(!settings.agents.contains_key("legacy_identity"));
        assert!(settings.agents.contains_key("default"));
        let resolution = crate::bot::resolve_agent_for_session(
            &resolved(&f),
            Some(&restarted),
            &bot.session_id,
            &bot.id,
        )
        .expect("bot body");
        assert_eq!(resolution.profile.system_prompt, "record body");
        assert!(resolution.profile.tools.is_empty());

        let after_first = fs::read_to_string(&registry).expect("registry after restart");
        let again = BotService::new(
            Arc::clone(f.bots.service()),
            Arc::clone(&f.settings),
            None,
            "/proj",
            registry.clone(),
        )
        .expect("second restart");
        assert_eq!(
            fs::read_to_string(&registry).expect("registry after second restart"),
            after_first,
            "the v4 migration must be idempotent"
        );
        assert_eq!(
            again.open(&bot.id).expect("open again").body.system_prompt,
            "record body"
        );
    }

    #[tokio::test]
    async fn resolver_reads_the_bot_record_and_falls_back_to_settings() {
        let f = fixture();
        let mut profile = f.settings.load_settings().expect("settings").agents["default"].clone();
        profile.system_prompt = "profile prompt".into();
        profile.tools.insert(
            "read".into(),
            AgentToolBinding {
                enabled: true,
                preset: None,
                allowed_tools: None,
            },
        );
        f.settings.write_agent("default", profile).expect("profile");
        let bot = f
            .bots
            .create_user_bot("Body owner", "", body_with_tool("bot prompt", "grep"))
            .await
            .expect("bot");
        let resolved = resolved(&f);

        let bot_resolution = crate::bot::resolve_agent_for_session(
            &resolved,
            Some(f.bots.as_ref()),
            &bot.session_id,
            &bot.id,
        )
        .expect("bot body");
        assert!(bot_resolution.is_bot());
        assert_eq!(bot_resolution.profile.system_prompt, "bot prompt");
        assert!(bot_resolution.profile.tools.contains_key("grep"));
        assert!(!bot_resolution.profile.tools.contains_key("read"));

        let normal = crate::bot::resolve_agent_for_session(
            &resolved,
            Some(f.bots.as_ref()),
            "not-a-bot-session",
            "default",
        )
        .expect("settings profile");
        assert!(!normal.is_bot());
        assert_eq!(normal.profile.system_prompt, "profile prompt");
        assert!(normal.profile.tools.contains_key("read"));
    }

    #[tokio::test]
    async fn builtin_prompt_is_owned_text_and_survives_restart() {
        let f = fixture();
        let created = f
            .bots
            .create_user_bot("Prompt owner", "", body("builtin:general"))
            .await
            .expect("create");
        let default_prompt = crate::config::global_db::DEFAULT_PROMPT;
        assert_eq!(created.body.system_prompt, default_prompt);
        assert!(!created.body.system_prompt.contains("subagent"));

        let registry = f.dir.path().join("bots.json");
        let before_restart = fs::read_to_string(&registry).expect("registry");
        for _ in 0..2 {
            let restarted = BotService::new(
                Arc::clone(f.bots.service()),
                Arc::clone(&f.settings),
                None,
                "/proj",
                registry.clone(),
            )
            .expect("restart");
            assert_eq!(
                restarted
                    .find(&created.id)
                    .expect("restarted bot")
                    .body
                    .system_prompt,
                default_prompt,
                "an expanded prompt must survive restart unchanged"
            );
        }
        assert_eq!(
            fs::read_to_string(&registry).expect("registry after restart"),
            before_restart,
            "an already expanded prompt must not be rewritten"
        );

        let resolution = crate::bot::resolve_agent_for_session(
            &resolved(&f),
            Some(f.bots.as_ref()),
            &created.session_id,
            &created.id,
        )
        .expect("bot body");
        let config = crate::config::bridge::agent_config_from_profile(&resolution.profile);
        let prompt = crate::context_pipeline::build_system_prompt(&created.id, &config, None);
        assert!(prompt.contains("General Purpose Agent"));
        assert!(!prompt.contains("subagent"));
    }

    #[tokio::test]
    async fn platform_segment_is_bot_session_only_and_uses_the_record() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Platform bot", "", body("body prompt"))
            .await
            .expect("bot");
        let segment = crate::bot::platform_prompt(&bot);
        assert!(segment.contains("\"Platform bot\""));
        assert!(segment.contains(&bot.id));
        assert!(segment.contains("[from bot id=<sender_id> name=<sender_name>]"));
        assert!(segment.contains("bot_send"));
        assert!(segment.contains("Cousins are not visible"));
        assert!(segment.contains("same-parent siblings"));
        assert!(segment.contains("Do not send a courtesy message"));

        let renamed = f
            .bots
            .update(
                &bot.id,
                BotUpdate {
                    name: Some("Renamed platform bot".into()),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect("rename");
        assert!(crate::bot::platform_prompt(&renamed).contains("Renamed platform bot"));

        let resolved = resolved(&f);
        let bot_resolution = crate::bot::resolve_agent_for_session(
            &resolved,
            Some(f.bots.as_ref()),
            &bot.session_id,
            &bot.id,
        )
        .expect("bot body");
        let bot_config = crate::config::bridge::agent_config_from_profile(&bot_resolution.profile);
        let bot_prompt = crate::context_pipeline::build_system_prompt_with_platform(
            &bot.id,
            &bot_config,
            None,
            Some(&segment),
        );
        assert!(bot_prompt.contains("You are bot \"Platform bot\""));
        assert!(bot_prompt.contains("[from bot id=<sender_id> name=<sender_name>]"));

        let default_profile = resolved.agents().get("default").expect("default");
        let normal_config = crate::config::bridge::agent_config_from_profile(default_profile);
        let normal_prompt =
            crate::context_pipeline::build_system_prompt("default", &normal_config, None);
        assert!(!normal_prompt.contains("You are bot"));
        assert!(!normal_prompt.contains("[from bot id="));

        let child = crate::bot::resolve_agent_for_session(
            &resolved,
            Some(f.bots.as_ref()),
            "child-session",
            "explore",
        )
        .expect("child profile");
        assert!(!child.is_bot(), "a subagent child is not a bot session");
        let child_config = crate::config::bridge::agent_config_from_profile(&child.profile);
        let child_prompt =
            crate::context_pipeline::build_system_prompt("explore", &child_config, None);
        assert!(!child_prompt.contains("You are bot"));
    }

    #[tokio::test]
    async fn step_budget_is_engine_level_for_bot_and_normal_sessions() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Long task", "", body(""))
            .await
            .expect("bot");
        let resolved = resolved(&f);
        assert_eq!(
            crate::bot::step_max_for_session(
                &resolved,
                Some(f.bots.as_ref()),
                &bot.session_id,
                &bot.id
            ),
            crate::bot::BOT_MAX_STEPS
        );
        assert!(crate::bot::BOT_MAX_STEPS >= 200_000);
        assert_ne!(crate::bot::BOT_MAX_STEPS, 50);
        assert_eq!(
            crate::bot::step_max_for_session(
                &resolved,
                Some(f.bots.as_ref()),
                "normal-session",
                "default"
            ),
            crate::agent::AGENT_MAX_STEPS
        );
    }

    #[tokio::test]
    async fn body_update_is_authoritative_across_restart() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Body authority", "", body("original"))
            .await
            .expect("create");
        f.bots
            .update(
                &bot.id,
                BotUpdate {
                    body: Some(body("edited after cutover")),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect("update");

        let restarted = BotService::new(
            Arc::clone(f.bots.service()),
            Arc::clone(&f.settings),
            None,
            "/proj",
            f.dir.path().join("bots.json"),
        )
        .expect("restart");
        assert_eq!(
            restarted.find(&bot.id).expect("bot").body.system_prompt,
            "edited after cutover"
        );
        assert!(
            !f.settings
                .load_settings()
                .expect("settings")
                .agents
                .contains_key(&bot.id),
            "bot updates never write a settings profile"
        );
    }

    #[tokio::test]
    async fn delete_removes_session_and_frees_quota() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Delete me", "", body(""))
            .await
            .expect("create");
        f.bots.delete(&bot.id).await.expect("delete");

        assert!(f.bots.list().is_empty());
        assert!(f.bots.service().status(&bot.session_id).is_none());
        assert!(
            !f.settings
                .load_settings()
                .expect("settings")
                .agents
                .contains_key(&bot.id)
        );

        for index in 0..MAX_BOTS_PER_WORKSPACE {
            f.bots
                .create_user_bot(&format!("Replacement {index}"), "", body(""))
                .await
                .expect("quota released");
        }
    }

    #[tokio::test]
    async fn ordinary_session_api_refuses_bot_delete_and_agent_switch() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Guarded", "", body(""))
            .await
            .expect("bot");
        let service = Arc::clone(f.bots.service());
        let meta_json = serde_json::to_string(&service.meta(&bot.session_id).expect("meta"))
            .expect("meta json");
        assert!(
            !meta_json.contains("Guarded"),
            "session storage must not contain a copy of the bot name: {meta_json}"
        );

        let error = service
            .delete_session_tree(&|_: &str| {}, &bot.session_id)
            .await
            .expect_err("ordinary delete must reject a bot session");
        assert!(matches!(
            error,
            DeleteSessionError::BotOwned { ref bot_id, .. } if bot_id == &bot.id
        ));
        assert_eq!(
            service
                .meta(&bot.session_id)
                .expect("session survives")
                .agent_id,
            bot.id
        );

        let error = service
            .set_agent_id(&bot.session_id, "default".into())
            .expect_err("ordinary agent switch must reject a bot session");
        assert!(matches!(
            error,
            crate::types::LitecodeError::BotOwnedSession(ref bot_id) if bot_id == &bot.id
        ));
        assert_eq!(
            service
                .meta(&bot.session_id)
                .expect("session identity survives")
                .agent_id,
            bot.id
        );

        let owner = service
            .bot_owner_for_session(&bot.session_id)
            .expect("shared owner query");
        assert_eq!(owner.bot_id, bot.id);
        assert_eq!(owner.name, "Guarded");

        f.bots
            .update(
                &bot.id,
                BotUpdate {
                    name: Some("Renamed without restart".into()),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect("rename");
        assert_eq!(
            service
                .bot_owner_for_session(&bot.session_id)
                .expect("owner after rename")
                .name,
            "Renamed without restart"
        );
        let fresh_service = SessionService::new(Arc::clone(&f.manager));
        assert_eq!(
            fresh_service
                .bot_owner_for_session(&bot.session_id)
                .expect("a fresh facade sees the shared owner index")
                .bot_id,
            bot.id
        );
        let error = fresh_service
            .delete_session_tree(&|_: &str| {}, &bot.session_id)
            .await
            .expect_err("a fresh facade also refuses bot delete");
        assert!(matches!(
            error,
            DeleteSessionError::BotOwned { ref bot_id, .. } if bot_id == &bot.id
        ));
        let error = service
            .remove_session(&bot.session_id)
            .expect_err("the raw service remove also refuses a bot session");
        assert!(matches!(
            error,
            crate::types::LitecodeError::BotOwnedSession(ref bot_id) if bot_id == &bot.id
        ));
        let error = f
            .manager
            .remove_session(&bot.session_id)
            .expect_err("the low-level remove primitive also refuses a bot session");
        assert!(matches!(
            error,
            crate::types::LitecodeError::BotOwnedSession(ref bot_id) if bot_id == &bot.id
        ));

        let normal = f
            .manager
            .open_session_sync("/proj", "default", None)
            .expect("normal session");
        let error = service
            .set_agent_id(&normal, bot.id.clone())
            .expect_err("a normal session cannot select a bot id");
        assert!(matches!(
            error,
            crate::types::LitecodeError::BotOwnedSession(ref bot_id) if bot_id == &bot.id
        ));

        f.bots.delete(&bot.id).await.expect("bot delete");
        assert!(
            service.bot_owner_for_session(&bot.session_id).is_none(),
            "bot delete must not leave an owner"
        );
        assert!(service.status(&bot.session_id).is_none());
        assert!(f.bots.find(&bot.id).is_none());

        service
            .delete_session_tree(&|_: &str| {}, &normal)
            .await
            .expect("ordinary session delete stays available");
    }

    #[tokio::test]
    async fn running_bot_and_running_child_block_delete() {
        let f = fixture();
        let parent = f
            .bots
            .create_user_bot("Parent", "", body(""))
            .await
            .expect("parent");
        let child_session = f
            .manager
            .test_child_session("/proj", &parent.session_id, "subagent-call")
            .expect("subagent child session");

        let parent_lease = f
            .manager
            .try_begin_operation(
                &parent.session_id,
                crate::session::manager::SessionOperationKind::Revert,
            )
            .expect("parent lease");
        let error = f.bots.delete(&parent.id).await.expect_err("parent busy");
        assert!(matches!(error, BotError::Busy));
        drop(parent_lease);

        let child_lease = f
            .manager
            .try_begin_operation(
                &child_session,
                crate::session::manager::SessionOperationKind::Revert,
            )
            .expect("child lease");
        let error = f
            .bots
            .delete(&parent.id)
            .await
            .expect_err("subagent child busy blocks parent delete");
        assert!(matches!(error, BotError::Busy));
        assert!(f.bots.find(&parent.id).is_some());
        assert!(f.bots.service().status(&child_session).is_some());
        drop(child_lease);
    }

    #[tokio::test]
    async fn restart_keeps_dangling_record_reports_and_does_not_rewrite_registry() {
        let f = fixture();
        let parent = f
            .bots
            .create_user_bot("Dangling parent", "", body(""))
            .await
            .expect("parent");
        let child = f
            .bots
            .create_from_bot(&parent.session_id, "Surviving child", "")
            .await
            .expect("child");
        f.manager
            .remove_session_unchecked(&parent.session_id)
            .expect("simulate externally deleted session");

        let registry = f.dir.path().join("bots.json");
        let before = fs::read_to_string(&registry).expect("registry before restart");
        let restarted = BotService::new(
            Arc::clone(f.bots.service()),
            Arc::clone(&f.settings),
            None,
            "/proj",
            registry.clone(),
        )
        .expect("restart reports the dangling record");
        assert!(
            restarted.find(&parent.id).is_some(),
            "startup must retain the record"
        );
        assert_eq!(
            restarted.find(&child.id).expect("child survives").position,
            BotPosition::Bot {
                id: parent.id.clone()
            },
            "startup must not move children of a dangling record"
        );
        assert_eq!(
            fs::read_to_string(&registry).expect("registry after restart"),
            before,
            "startup must not rewrite bots.json for a dangling record"
        );
        let error = restarted.open(&parent.id).expect_err("missing session");
        assert!(matches!(error, BotError::SessionMissing(_)));
        assert!(restarted.open(&child.id).is_ok());

        restarted.delete(&parent.id).await.expect("explicit delete");
        assert!(restarted.find(&parent.id).is_none());
        assert_eq!(
            restarted.find(&child.id).expect("child survives").position,
            BotPosition::None,
            "explicit delete still promotes children"
        );
    }

    #[tokio::test]
    async fn restart_keeps_an_empty_binding_until_explicit_delete() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("No binding", "", body(""))
            .await
            .expect("bot");
        let registry = f.dir.path().join("bots.json");
        let mut file: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&registry).expect("registry"))
                .expect("json");
        file["bots"]
            .as_array_mut()
            .expect("bots")
            .iter_mut()
            .find(|row| row["id"] == bot.id)
            .expect("row")
            .as_object_mut()
            .expect("object")
            .remove("session_id");
        fs::write(
            &registry,
            serde_json::to_string_pretty(&file).expect("json"),
        )
        .expect("write registry");
        let before = fs::read_to_string(&registry).expect("registry before restart");

        let restarted = BotService::new(
            Arc::clone(f.bots.service()),
            Arc::clone(&f.settings),
            None,
            "/proj",
            registry.clone(),
        )
        .expect("restart");
        assert!(
            matches!(restarted.open(&bot.id), Err(BotError::SessionMissing(_))),
            "an empty binding must report SessionMissing"
        );
        assert_eq!(
            fs::read_to_string(&registry).expect("registry after restart"),
            before,
            "startup must not rewrite an empty binding"
        );

        restarted.delete(&bot.id).await.expect("explicit delete");
        assert!(restarted.find(&bot.id).is_none());
    }

    #[tokio::test]
    async fn update_moves_to_folder_and_folder_delete_ungroups() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Folder bot", "", body(""))
            .await
            .expect("create");
        let folder_id = f
            .bots
            .service()
            .create_session_folder("Work", true)
            .expect("bot folder");
        assert!(is_bot_folder_id(&folder_id));
        let updated = f
            .bots
            .update(
                &bot.id,
                BotUpdate {
                    folder_id: Some(folder_id.clone()),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect("move");
        assert_eq!(
            updated.position,
            BotPosition::Folder {
                id: folder_id.clone()
            }
        );

        f.bots.clear_folder(&folder_id);
        assert_eq!(
            f.bots.find(&bot.id).expect("bot").position,
            BotPosition::None
        );
    }

    #[tokio::test]
    async fn update_rejects_a_session_folder() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Scoped bot", "", body(""))
            .await
            .expect("create");
        let session_folder = f
            .bots
            .service()
            .create_session_folder("Sessions", false)
            .expect("session folder");
        let error = f
            .bots
            .update(
                &bot.id,
                BotUpdate {
                    folder_id: Some(session_folder),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect_err("session folder must be rejected");
        assert!(matches!(error, BotError::FolderScope(_)));
    }

    #[tokio::test]
    async fn update_moves_under_a_bot_and_rejects_cycles() {
        let f = fixture();
        let parent = f
            .bots
            .create_user_bot("Parent", "", body("parent"))
            .await
            .expect("parent");
        let child = f
            .bots
            .create_from_bot(&parent.session_id, "Child", "")
            .await
            .expect("child");
        let grandchild = f
            .bots
            .create_from_bot(&child.session_id, "Grandchild", "")
            .await
            .expect("grandchild");

        let info = crate::client_protocol::protocol::BotInfo::from(child.clone());
        assert_eq!(
            info.position,
            BotPosition::Bot {
                id: parent.id.clone()
            }
        );
        let wire = serde_json::to_value(&info).expect("bot info");
        assert_eq!(wire["position"]["kind"], "bot");
        assert_eq!(wire["position"]["id"], parent.id);

        let self_cycle = f
            .bots
            .update(
                &parent.id,
                BotUpdate {
                    parent_bot_id: Some(parent.id.clone()),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect_err("self is not a parent");
        assert!(matches!(self_cycle, BotError::ParentCycle));

        let descendant_cycle = f
            .bots
            .update(
                &parent.id,
                BotUpdate {
                    parent_bot_id: Some(grandchild.id.clone()),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect_err("descendant is not a parent");
        assert!(matches!(descendant_cycle, BotError::ParentCycle));

        let missing = f
            .bots
            .update(
                &child.id,
                BotUpdate {
                    parent_bot_id: Some("missing-parent".into()),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect_err("parent must exist");
        assert!(matches!(missing, BotError::ParentNotFound(_)));

        f.bots
            .update(
                &child.id,
                BotUpdate {
                    clear_parent: true,
                    ..BotUpdate::default()
                },
            )
            .await
            .expect("unparent");
        assert_eq!(
            f.bots.find(&child.id).expect("child").position,
            BotPosition::None
        );
        assert_eq!(
            f.bots.find(&grandchild.id).expect("grandchild").position,
            BotPosition::Bot {
                id: child.id.clone()
            }
        );
    }

    #[tokio::test]
    async fn captured_turn_resolution_does_not_follow_body_edits() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Snapshot", "", body_with_tool("old prompt", "read"))
            .await
            .expect("bot");
        let resolved = resolved(&f);
        let before = crate::bot::resolve_agent_for_session(
            &resolved,
            Some(f.bots.as_ref()),
            &bot.session_id,
            &bot.id,
        )
        .expect("before");
        assert_eq!(before.profile.system_prompt, "old prompt");
        assert!(before.profile.tools.contains_key("read"));

        f.bots
            .update(
                &bot.id,
                BotUpdate {
                    body: Some(body_with_tool("new prompt", "grep")),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect("update");

        assert_eq!(before.profile.system_prompt, "old prompt");
        assert!(before.profile.tools.contains_key("read"));
        assert!(!before.profile.tools.contains_key("grep"));
        let after = crate::bot::resolve_agent_for_session(
            &resolved,
            Some(f.bots.as_ref()),
            &bot.session_id,
            &bot.id,
        )
        .expect("after");
        assert_eq!(after.profile.system_prompt, "new prompt");
        assert!(after.profile.tools.contains_key("grep"));
    }

    #[tokio::test]
    async fn editing_bots_json_is_not_hot_loaded() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Memory", "", body("old"))
            .await
            .expect("bot");
        let path = f.dir.path().join("bots.json");
        let mut disk = bot.clone();
        disk.name = "Disk only".into();
        disk.body = body("disk");
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "version": BOT_REGISTRY_VERSION,
                "bots": [disk],
            }))
            .expect("json"),
        )
        .expect("write registry");

        assert_eq!(f.bots.find(&bot.id).expect("memory").name, "Memory");
        let reloaded = BotService::new(
            Arc::clone(f.bots.service()),
            Arc::clone(&f.settings),
            None,
            "/proj",
            path,
        )
        .expect("reload");
        assert_eq!(reloaded.find(&bot.id).expect("reloaded").name, "Disk only");
        assert_eq!(
            reloaded.find(&bot.id).expect("reloaded").body.system_prompt,
            "disk"
        );
    }

    #[tokio::test]
    async fn bot_events_cover_create_update_and_delete() {
        let f = fixture();
        let mut events = f.bots.subscribe();
        let bot = f
            .bots
            .create_user_bot("Event bot", "", body(""))
            .await
            .expect("create");
        assert!(matches!(
            events.recv().await.expect("created"),
            BotEvent::Created(record) if record.id == bot.id
        ));

        f.bots
            .update(
                &bot.id,
                BotUpdate {
                    name: Some("Renamed".into()),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect("update");
        assert!(matches!(
            events.recv().await.expect("updated"),
            BotEvent::Updated(record) if record.name == "Renamed"
        ));

        f.bots.delete(&bot.id).await.expect("delete");
        assert!(matches!(
            events.recv().await.expect("removed"),
            BotEvent::Removed { bot_id, .. } if bot_id == bot.id
        ));
    }

    #[tokio::test]
    async fn send_queues_provenance_and_returns_without_waiting() {
        let f = fixture();
        let sender = f
            .bots
            .create_user_bot("Sender", "", body(""))
            .await
            .expect("sender");
        let target = f
            .bots
            .create_from_bot(&sender.session_id, "Target", "")
            .await
            .expect("target");

        let delivered = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&delivered);
        f.bots.set_turn_runner(Arc::new(move |session_id: &str| {
            seen.lock().unwrap().push(session_id.to_string());
        }));

        let sent = f
            .bots
            .send_from_bot(&sender.session_id, "target", "  hello  ")
            .await
            .expect("send");

        assert_eq!(sent.id, target.id);
        assert_eq!(
            delivered.lock().unwrap().as_slice(),
            std::slice::from_ref(&target.session_id)
        );
        let pending = f
            .bots
            .service()
            .pending_messages_snapshot(&target.session_id);
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].text,
            format!("[from bot id={} name=Sender]\nhello", sender.id)
        );
        assert_eq!(
            pending[0].sources,
            vec![crate::types::MessageSource::bot(
                sender.id.clone(),
                "Sender"
            )]
        );

        // A unique name resolves to the id; the receipt carries the target.
        let by_name = f
            .bots
            .send_from_bot(&sender.session_id, "TARGET", "again")
            .await
            .expect("name target");
        assert_eq!(by_name.id, target.id);
        assert_eq!(by_name.name, "Target");
    }

    #[tokio::test]
    async fn org_visibility_covers_self_ancestors_siblings_and_subtree_only() {
        let f = fixture();
        let root = f
            .bots
            .create_user_bot("Root", "", body("root"))
            .await
            .expect("root");
        let child_a = f
            .bots
            .create_from_bot(&root.session_id, "Child A", "")
            .await
            .expect("child a");
        let child_b = f
            .bots
            .create_from_bot(&root.session_id, "Child B", "")
            .await
            .expect("child b");
        let grand_a = f
            .bots
            .create_from_bot(&child_a.session_id, "Grand A", "")
            .await
            .expect("grand a");
        let other_root = f
            .bots
            .create_user_bot("Other Root", "", body("other"))
            .await
            .expect("other root");
        let other_child = f
            .bots
            .create_from_bot(&other_root.session_id, "Other Child", "")
            .await
            .expect("other child");

        let ids = |caller: &str| {
            let mut ids: Vec<String> = f
                .bots
                .list_for_caller(caller)
                .into_iter()
                .map(|bot| bot.id)
                .collect();
            ids.sort();
            ids
        };
        let mut expected_child_a = vec![
            root.id.clone(),
            child_a.id.clone(),
            child_b.id.clone(),
            grand_a.id.clone(),
        ];
        expected_child_a.sort();
        assert_eq!(ids(&child_a.id), expected_child_a);

        let mut expected_root = vec![
            root.id.clone(),
            child_a.id.clone(),
            child_b.id.clone(),
            grand_a.id.clone(),
            other_root.id.clone(),
        ];
        expected_root.sort();
        assert_eq!(ids(&root.id), expected_root);

        let expected_child_b = {
            let mut ids = vec![root.id.clone(), child_a.id.clone(), child_b.id.clone()];
            ids.sort();
            ids
        };
        assert_eq!(ids(&child_b.id), expected_child_b);
        assert!(!f.bots.can_see(&child_a.id, &other_root.id));
        assert!(!f.bots.can_see(&child_a.id, &other_child.id));
        assert!(!f.bots.can_see(&child_b.id, &grand_a.id));
        assert!(f.bots.can_see(&root.id, &other_root.id));
        assert!(f.bots.can_see(&other_root.id, &root.id));
        assert!(!f.bots.can_see(&root.id, &other_child.id));
        assert!(!f.bots.can_see(&other_root.id, &child_a.id));
        assert!(f.bots.can_see(&child_a.id, &child_b.id));
        assert!(f.bots.can_see(&grand_a.id, &root.id));

        f.bots
            .send_from_bot(&child_a.session_id, &child_b.id, "sibling")
            .await
            .expect("sibling send");
        f.bots
            .send_from_bot(&child_a.session_id, &root.id, "ancestor")
            .await
            .expect("ancestor send");
        let hidden = f
            .bots
            .send_from_bot(&child_a.session_id, &other_child.id, "cousin")
            .await
            .expect_err("cousin must be out of scope");
        assert!(matches!(hidden, BotError::NotFound(_)));
        let hidden_by_name = f
            .bots
            .send_from_bot(&child_a.session_id, "Other Child", "cousin")
            .await
            .expect_err("out-of-scope name");
        assert!(matches!(hidden_by_name, BotError::NotFound(_)));
        f.bots
            .send_from_bot(&root.session_id, &other_root.id, "other root")
            .await
            .expect("roots are peers");
        assert!(
            f.bots
                .send_from_bot(&root.session_id, &other_child.id, "other branch")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn duplicate_bot_names_are_rejected_for_bot_send() {
        let f = fixture();
        let sender = f
            .bots
            .create_user_bot("Sender", "", body(""))
            .await
            .expect("sender");
        let first = f
            .bots
            .create_from_bot(&sender.session_id, "Dup", "")
            .await
            .expect("first");
        let second = f
            .bots
            .create_from_bot(&sender.session_id, "Other", "")
            .await
            .expect("second");
        {
            let mut state = f.bots.state.lock().unwrap();
            let other = state
                .iter_mut()
                .find(|bot| bot.id == second.id)
                .expect("second");
            other.name = first.name.clone();
        }

        let error = f
            .bots
            .send_from_bot(&sender.session_id, "dup", "hello")
            .await
            .expect_err("ambiguous name");
        assert!(matches!(error, BotError::AmbiguousTarget(_)));
    }

    #[tokio::test]
    async fn failed_registry_write_removes_the_unowned_session() {
        let f = fixture();
        std::fs::create_dir(f.dir.path().join("bots.json")).expect("block registry path");

        let error = f
            .bots
            .create_user_bot("Orphan", "", body(""))
            .await
            .expect_err("registry write must fail");
        assert!(matches!(error, BotError::Registry(_)));
        assert!(f.bots.list().is_empty());
        assert!(
            f.bots
                .service()
                .list_sessions()
                .expect("sessions")
                .is_empty(),
            "failed create must not leave a session behind"
        );
        assert!(
            !f.settings
                .load_settings()
                .expect("settings")
                .agents
                .contains_key("Orphan")
        );
    }

    #[tokio::test]
    async fn failed_write_restores_this_write_not_the_first_snapshot() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Before", "", body(""))
            .await
            .expect("create");
        let before_update = fs::read_to_string(f.dir.path().join("bots.json")).expect("registry");

        f.bots
            .update(
                &bot.id,
                BotUpdate {
                    name: Some("After update".into()),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect("first successful write");
        let after_update = fs::read_to_string(f.dir.path().join("bots.json")).expect("registry");
        assert_ne!(after_update, before_update);

        FAIL_REGISTRY_REPLACE.with(|flag| flag.set(true));
        let error = f
            .bots
            .update(
                &bot.id,
                BotUpdate {
                    name: Some("Lost update".into()),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect_err("injected replace failure");
        assert!(matches!(error, BotError::Registry(_)));
        assert_eq!(f.bots.find(&bot.id).expect("bot").name, "After update");
        assert_eq!(
            fs::read_to_string(f.dir.path().join("bots.json")).expect("registry"),
            after_update
        );
    }
}
