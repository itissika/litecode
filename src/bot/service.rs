//! Workspace-local bot registry and product operations.
//!
//! The registry is a small JSON file next to `sessions.db`. It is the only
//! authority for bot identity; sessions stay ordinary sessions and the session
//! model never learns about bots. Each bot owns one ordinary AgentProfile; the
//! profile itself lives in the normal settings store and is hidden from normal
//! agent lists by its `bot_` id prefix.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::config::DocId;
use crate::config::schema::{AgentProfile, AgentRole};
use crate::config::settings_writer::SettingsWriter;
use crate::runtime::RuntimeHandle;
use crate::session::SessionService;
use crate::session::delete::{DeleteSessionError, RemovedSessionCleanup};
use crate::types::UserInput;

/// Current on-disk registry shape.
const BOT_REGISTRY_VERSION: u32 = 1;
/// Hard cap for one workspace. User-created and bot-created bots share it.
pub const MAX_BOTS_PER_WORKSPACE: usize = 16;
/// A user creates depth 0; that bot may create one depth-1 bot, which may not
/// create further bots.
pub const MAX_BOT_DEPTH: u32 = 1;
/// Bot-owned agent profiles use this reserved id prefix. Normal primary-agent
/// pickers and Settings -> Agents filter the prefix out; `get_agent` still
/// resolves the profile by id for the bot editor.
pub const BOT_PROFILE_PREFIX: &str = "bot_";
/// Bot folders are session-folder rows with this reserved id prefix. Sessions
/// never move into one; bots never move into an unprefixed folder.
pub const BOT_FOLDER_PREFIX: &str = "bot-folder-";
const MAX_BOT_NAME_CHARS: usize = 48;
const MAX_BOT_DESCRIPTION_CHARS: usize = 280;

/// A profile id owned by a bot rather than exposed as a user primary agent.
pub fn is_bot_profile_id(id: &str) -> bool {
    id.starts_with(BOT_PROFILE_PREFIX)
}

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
    #[error("this workspace already has {MAX_BOTS_PER_WORKSPACE} bots")]
    QuotaExceeded,
    #[error("bot creation depth is limited to {max}")]
    DepthExceeded { max: u32 },
    #[error("this session is not a bot session")]
    NotBotSession,
    #[error("bot registry: {0}")]
    Registry(String),
    #[error("session: {0}")]
    Session(String),
    #[error("agent template not found: {0}")]
    TemplateNotFound(String),
    #[error("bot agent profile not found: {0}")]
    ProfileNotFound(String),
    #[error("bot template must be a primary agent")]
    InvalidTemplate,
    #[error("this bot is running; stop it before changing capability or deleting it")]
    Busy,
    #[error("session folder not found: {0}")]
    FolderNotFound(String),
    #[error("folder is not owned by the bot domain: {0}")]
    FolderScope(String),
    #[error("bot settings: {0}")]
    Settings(String),
    #[error("bot does not own its agent profile")]
    ProfileNotOwned,
}

impl From<crate::types::LitecodeError> for BotError {
    fn from(error: crate::types::LitecodeError) -> Self {
        Self::Session(error.to_string())
    }
}

/// One durable bot identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotRecord {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Bot-owned AgentProfile id. Legacy registries called this `agent_id`;
    /// serde accepts that name so an existing workspace upgrades in place.
    #[serde(default, alias = "agent_id")]
    pub profile_id: String,
    /// `None` means the user created this bot. A bot-created bot points at its
    /// creator, which is also how depth is audited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_bot_id: Option<String>,
    #[serde(default)]
    pub depth: u32,
    pub created_at: i64,
    /// Ordinary session driven by `SessionService`. Empty only while a create
    /// is being reserved in-process.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_session_id: String,
    /// Bot-scoped sidebar folder id (`bot-folder-...`). Bot folders share the
    /// `session_folders` table but never mix with session folders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    /// Manual order inside the bot group. New bots get max + 1.
    #[serde(default)]
    pub sort_order: i64,
}

/// One change to the workspace bot directory. Forwarded to every websocket as
/// `bot/changed`; the UI never needs a manual refresh after create/edit/delete.
#[derive(Debug, Clone)]
pub enum BotEvent {
    Created(BotRecord),
    Updated(BotRecord),
    Removed {
        bot_id: String,
        default_session_id: Option<String>,
    },
}

/// Fields accepted by `bot/update`. Identity, folder, and capability are
/// independent: a running bot may be renamed, but its profile cannot change.
#[derive(Debug, Default)]
pub struct BotUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub profile: Option<AgentProfile>,
    pub folder_id: Option<String>,
    pub clear_folder: bool,
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
    /// Weak to avoid SettingsWriter -> runtime -> BotService -> SettingsWriter.
    settings: Weak<SettingsWriter>,
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
        let mut bots = load_registry(&path)?;
        let mut docs = Vec::new();
        let mut changed = false;

        // Upgrade legacy bots whose profile id still points at a user template.
        // The profile is copied once per bot; shared templates are never
        // mutated or deleted by bot lifecycle.
        let mut used_profiles: HashSet<String> = settings
            .load_settings()
            .map_err(|error| BotError::Settings(error.to_string()))?
            .agents
            .keys()
            .cloned()
            .collect();
        for bot in &mut bots {
            // Folders created before bot folders existed are session folders.
            // Ungroup those bots instead of leaving a cross-domain placement.
            if bot
                .folder_id
                .as_deref()
                .is_some_and(|folder| !is_bot_folder_id(folder))
            {
                bot.folder_id = None;
                changed = true;
            }
            if bot.profile_id.is_empty() {
                continue;
            }
            let loaded = settings
                .load_settings()
                .map_err(|error| BotError::Settings(error.to_string()))?;
            // An owned id is derived from this bot's id. Any other id (an old
            // shared template, or even another bot's profile) must be copied.
            if owns_profile(bot) && loaded.agents.contains_key(&bot.profile_id) {
                continue;
            }
            let template = loaded
                .agents
                .get(&bot.profile_id)
                .cloned()
                .or_else(|| loaded.agents.get("default").cloned());
            let Some(template) = template else {
                tracing::warn!(
                    bot_id = %bot.id,
                    template = %bot.profile_id,
                    "bot profile migration skipped: template is missing"
                );
                continue;
            };
            if template.role != AgentRole::Primary {
                tracing::warn!(
                    bot_id = %bot.id,
                    template = %bot.profile_id,
                    "bot profile migration skipped: template is not primary"
                );
                continue;
            }
            let profile_id = unique_profile_id(&bot.id, &used_profiles);
            let mut profile = template;
            profile.role = AgentRole::Primary;
            match settings.write_bot_agent(&profile_id, profile) {
                Ok(ack) => {
                    docs.extend(ack.docs);
                    if !bot.default_session_id.is_empty()
                        && service.meta(&bot.default_session_id).is_ok()
                        && let Err(error) =
                            service.set_agent_id(&bot.default_session_id, profile_id.clone())
                    {
                        tracing::warn!(
                            bot_id = %bot.id,
                            session_id = %bot.default_session_id,
                            %error,
                            "bot profile migrated but the default session could not be rebound"
                        );
                    }
                    bot.profile_id = profile_id.clone();
                    used_profiles.insert(profile_id);
                    changed = true;
                }
                Err(error) => {
                    tracing::warn!(
                        bot_id = %bot.id,
                        template = %bot.profile_id,
                        %error,
                        "bot profile migration failed; keeping the old profile id"
                    );
                }
            }
        }
        if changed {
            save_registry(&path, &bots)?;
        }

        let (events, _) = broadcast::channel(64);
        let service = Self {
            service,
            settings: Arc::downgrade(&settings),
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
            .filter(|bot| !bot.default_session_id.is_empty())
            .cloned()
            .collect();
        sort_records(&mut bots);
        bots
    }

    /// The bot whose default session is `session_id`.
    pub fn bot_for_session(&self, session_id: &str) -> Option<BotRecord> {
        if session_id.is_empty() {
            return None;
        }
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .find(|bot| bot.default_session_id == session_id)
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
            .filter(|bot| !bot.default_session_id.is_empty())
            .find(|bot| bot.id == needle || bot.name.to_lowercase() == folded)
            .cloned()
    }

    /// User-created bot: depth 0, no owner, copied from a visible primary
    /// agent template.
    pub async fn create_user_bot(
        &self,
        name: &str,
        description: &str,
        template_agent_id: &str,
    ) -> Result<BotRecord, BotError> {
        self.create(name, description, template_agent_id, None)
            .await
    }

    /// Bot-created bot: inherits a copy of the caller's owned profile and
    /// records the owner. It never shares the caller's profile id.
    pub async fn create_from_bot(
        &self,
        caller_session_id: &str,
        name: &str,
        description: &str,
    ) -> Result<BotRecord, BotError> {
        let caller = self
            .bot_for_session(caller_session_id)
            .ok_or(BotError::NotBotSession)?;
        if caller.depth >= MAX_BOT_DEPTH {
            return Err(BotError::DepthExceeded { max: MAX_BOT_DEPTH });
        }
        self.create(name, description, &caller.profile_id, Some(&caller))
            .await
    }

    async fn create(
        &self,
        name: &str,
        description: &str,
        template_agent_id: &str,
        owner: Option<&BotRecord>,
    ) -> Result<BotRecord, BotError> {
        let name = normalize_name(name)?;
        let description = normalize_description(description);
        let depth = owner.map(|bot| bot.depth + 1).unwrap_or(0);
        if depth > MAX_BOT_DEPTH {
            return Err(BotError::DepthExceeded { max: MAX_BOT_DEPTH });
        }

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
                profile_id: String::new(),
                owner_bot_id: owner.map(|bot| bot.id.clone()),
                depth,
                created_at: now_ms(),
                default_session_id: String::new(),
                folder_id: None,
                sort_order: state.iter().map(|bot| bot.sort_order).max().unwrap_or(0) + 1,
            };
            state.push(record.clone());
            record
        };

        // A cancelled create (for example, a dropped RPC future) must not leave
        // a reservation, a profile, or a session behind.
        let mut reservation = Reservation {
            service: self,
            id: record.id.clone(),
            profile_id: None,
            session_id: None,
            committed: false,
        };

        let allow_bot_template = owner.is_some();
        let (profile_id, profile) =
            self.create_owned_profile(&record.id, template_agent_id, allow_bot_template)?;
        reservation.profile_id = Some(profile_id.clone());

        let model_ref = self.model_for_profile(&profile_id, &profile);
        let session_id = match self
            .service
            .open_session(&self.project, &profile_id, model_ref.as_deref())
            .await
        {
            Ok(session_id) => session_id,
            Err(error) => {
                if self.discard_profile(&profile_id) {
                    reservation.profile_id = None;
                }
                return Err(error.into());
            }
        };
        reservation.session_id = Some(session_id.clone());

        let mut committed = record;
        committed.profile_id = profile_id;
        committed.default_session_id = session_id;
        if let Err(error) = self.commit(committed.clone()) {
            if self.discard_session(&committed.default_session_id).await {
                reservation.session_id = None;
            }
            if self.discard_profile(&committed.profile_id) {
                reservation.profile_id = None;
            }
            return Err(error);
        }
        reservation.committed = true;
        self.emit(BotEvent::Created(committed.clone()));
        Ok(committed)
    }

    /// Ensure a bot still has a live default session, recreating it if the user
    /// deleted the old one. Returns the refreshed bot.
    pub async fn ensure_default_session(&self, bot_id: &str) -> Result<BotRecord, BotError> {
        let record = self
            .find(bot_id)
            .ok_or_else(|| BotError::NotFound(bot_id.to_string()))?;
        if self.session_exists(&record.default_session_id)? {
            self.rebind_session_agent(&record)?;
            return Ok(record);
        }
        if !owns_profile(&record) {
            return Err(BotError::ProfileNotOwned);
        }
        let profile = self
            .load_profile(&record.profile_id)?
            .ok_or_else(|| BotError::ProfileNotFound(record.profile_id.clone()))?;
        let model_ref = self.model_for_profile(&record.profile_id, &profile);
        let session_id = self
            .service
            .open_session(&self.project, &record.profile_id, model_ref.as_deref())
            .await?;
        let mut updated = record;
        updated.default_session_id = session_id;
        if let Err(error) = self.commit(updated.clone()) {
            self.discard_session(&updated.default_session_id).await;
            return Err(error);
        }
        self.emit(BotEvent::Updated(updated.clone()));
        Ok(updated)
    }

    /// Update identity, folder/order, and/or the owned capability profile.
    /// Capability changes are rejected while the bot's own turn is running.
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
        if update.clear_folder {
            next.folder_id = None;
        } else if let Some(folder_id) = update.folder_id {
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
            next.folder_id = Some(folder_id);
        }
        if let Some(sort_order) = update.sort_order {
            next.sort_order = sort_order;
        }

        let Some(profile_update) = update.profile else {
            self.commit(next.clone())?;
            self.emit(BotEvent::Updated(next.clone()));
            return Ok(next);
        };

        if !owns_profile(&current) {
            return Err(BotError::ProfileNotOwned);
        }
        if self
            .service
            .is_session_busy_blocking(&current.default_session_id)
        {
            return Err(BotError::Busy);
        }
        if self.session_exists(&current.default_session_id)? {
            self.rebind_session_agent(&current)?;
        }
        let profile_id = current.profile_id.clone();
        let old_profile = self
            .load_profile(&profile_id)?
            .ok_or_else(|| BotError::ProfileNotFound(profile_id.clone()))?;
        let old_session_model = if current.default_session_id.is_empty() {
            None
        } else {
            self.service
                .session_model_id(&current.default_session_id)
                .or_else(|| {
                    self.service
                        .meta(&current.default_session_id)
                        .ok()
                        .and_then(|meta| meta.model_id)
                })
        };

        let mut new_profile = profile_update;
        new_profile.role = AgentRole::Primary;
        self.write_profile(&profile_id, new_profile.clone())?;
        let saved_profile = self.load_profile(&profile_id)?.unwrap_or(new_profile);
        let model_changed = saved_profile.model_ref != old_profile.model_ref;
        if model_changed
            && !current.default_session_id.is_empty()
            && self.session_exists(&current.default_session_id)?
        {
            let model = normalized_model(&saved_profile);
            if let Err(error) = self
                .service
                .set_session_model_id(&current.default_session_id, model)
            {
                let _ = self.write_profile(&profile_id, old_profile.clone());
                return Err(error.into());
            }
        }

        if let Err(error) = self.commit(next.clone()) {
            let _ = self.write_profile(&profile_id, old_profile);
            if model_changed && !current.default_session_id.is_empty() {
                let _ = self
                    .service
                    .set_session_model_id(&current.default_session_id, old_session_model);
            }
            return Err(error);
        }
        self.emit(BotEvent::Updated(next.clone()));
        Ok(next)
    }

    /// Delete the bot, its owned profile, and its default session. A running
    /// bot is never deleted silently.
    pub async fn delete(&self, bot_id: &str) -> Result<BotRecord, BotError> {
        let record = self
            .find(bot_id)
            .ok_or_else(|| BotError::NotFound(bot_id.to_string()))?;
        if !record.default_session_id.is_empty()
            && self
                .service
                .is_session_busy_blocking(&record.default_session_id)
        {
            return Err(BotError::Busy);
        }

        let owned_profile = owns_profile(&record);
        let saved_profile = if owned_profile {
            self.load_profile(&record.profile_id)?
        } else {
            None
        };

        if !record.default_session_id.is_empty()
            && self.session_exists(&record.default_session_id)?
        {
            let hub = self.subagent_hub();
            let cleanup = |session_id: &str| {
                if let Some(hub) = &hub {
                    RemovedSessionCleanup::on_session_removed(hub.as_ref(), session_id);
                }
            };
            match self
                .service
                .delete_session_tree(&cleanup, &record.default_session_id)
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

        if owned_profile && saved_profile.is_some() {
            self.delete_profile(&record.profile_id)?;
        }
        if let Err(error) = self.remove_record(&record.id) {
            if let Some(profile) = saved_profile {
                let _ = self.write_profile(&record.profile_id, profile);
            }
            return Err(error);
        }
        self.emit(BotEvent::Removed {
            bot_id: record.id.clone(),
            default_session_id: (!record.default_session_id.is_empty())
                .then(|| record.default_session_id.clone()),
        });
        Ok(record)
    }

    /// Session folders are shared. Deleting a folder ungroups bots that used it
    /// so the UI never keeps a dangling folder id.
    pub fn clear_folder(&self, folder_id: &str) {
        let updated = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let mut next = state.clone();
            let mut changed_ids = HashSet::new();
            for bot in &mut next {
                if bot.folder_id.as_deref() == Some(folder_id) {
                    bot.folder_id = None;
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
        let target = self
            .find(target)
            .ok_or_else(|| BotError::NotFound(target.to_string()))?;
        let target = self.ensure_default_session(&target.id).await?;

        let text = format!("[from bot {}]\n{}", caller.name, message);
        self.service
            .enqueue_user_input(&target.default_session_id, UserInput::text(text))?;
        if let Some(runner) = self.turn_runner.get() {
            runner(&target.default_session_id);
        }
        Ok(target)
    }

    /// Heal a default session that still points at a shared template (an old
    /// registry whose migration could not run, or an externally edited row).
    fn rebind_session_agent(&self, bot: &BotRecord) -> Result<(), BotError> {
        let meta = self.service.meta(&bot.default_session_id)?;
        if meta.agent_id != bot.profile_id
            && !self
                .service
                .is_session_busy_blocking(&bot.default_session_id)
        {
            self.service
                .set_agent_id(&bot.default_session_id, bot.profile_id.clone())?;
        }
        Ok(())
    }

    /// Durable existence check. status only sees sessions hydrated in this
    /// process, so a restart must not treat an on-disk default session as gone.
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

    /// Load the bot-owned profile from the normal settings store.
    fn load_profile(&self, profile_id: &str) -> Result<Option<AgentProfile>, BotError> {
        let settings = self.settings()?;
        let loaded = settings
            .load_settings()
            .map_err(|error| BotError::Settings(error.to_string()))?;
        Ok(loaded.agents.get(profile_id).cloned())
    }

    /// Copy a template profile into a new bot-owned profile. The template is
    /// never mutated or shared.
    fn create_owned_profile(
        &self,
        bot_id: &str,
        template_agent_id: &str,
        allow_bot_template: bool,
    ) -> Result<(String, AgentProfile), BotError> {
        if !allow_bot_template && is_bot_profile_id(template_agent_id) {
            return Err(BotError::InvalidTemplate);
        }
        let settings = self.settings()?;
        let loaded = settings
            .load_settings()
            .map_err(|error| BotError::Settings(error.to_string()))?;
        let template = loaded
            .agents
            .get(template_agent_id)
            .cloned()
            .ok_or_else(|| BotError::TemplateNotFound(template_agent_id.to_string()))?;
        if template.role != AgentRole::Primary {
            return Err(BotError::InvalidTemplate);
        }
        let used: HashSet<String> = loaded.agents.keys().cloned().collect();
        let profile_id = unique_profile_id(bot_id, &used);
        let mut profile = template;
        profile.role = AgentRole::Primary;
        let ack = settings
            .write_bot_agent(&profile_id, profile.clone())
            .map_err(|error| BotError::Settings(error.to_string()))?;
        self.apply_docs(&ack.docs);
        // A read-back can fail transiently after a successful commit. Keep the
        // profile we just wrote so the create path still owns a cleanup handle.
        let saved = settings
            .load_settings()
            .ok()
            .and_then(|loaded| loaded.agents.get(&profile_id).cloned())
            .unwrap_or(profile);
        Ok((profile_id, saved))
    }

    fn write_profile(&self, profile_id: &str, profile: AgentProfile) -> Result<(), BotError> {
        if !is_bot_profile_id(profile_id) {
            return Err(BotError::ProfileNotOwned);
        }
        let settings = self.settings()?;
        let ack = settings
            .write_bot_agent(profile_id, profile)
            .map_err(|error| BotError::Settings(error.to_string()))?;
        self.apply_docs(&ack.docs);
        Ok(())
    }

    fn delete_profile(&self, profile_id: &str) -> Result<(), BotError> {
        if !is_bot_profile_id(profile_id) {
            return Err(BotError::ProfileNotOwned);
        }
        let settings = self.settings()?;
        let ack = settings
            .delete_bot_agent(profile_id)
            .map_err(|error| BotError::Settings(error.to_string()))?;
        self.apply_docs(&ack.docs);
        Ok(())
    }

    fn settings(&self) -> Result<Arc<SettingsWriter>, BotError> {
        self.settings
            .upgrade()
            .ok_or_else(|| BotError::Settings("settings writer is not available".into()))
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

    fn model_for_profile(&self, profile_id: &str, profile: &AgentProfile) -> Option<String> {
        normalized_model(profile).or_else(|| {
            let runtime = self.runtime.as_ref().and_then(Weak::upgrade)?;
            let runtime = runtime.read().ok()?;
            crate::runtime::seed_model_ref(&runtime.resolved, profile_id)
        })
    }

    /// Best-effort removal of a session opened for a bot that could not be
    /// recorded. The session is brand new and idle, so this is the rollback
    /// half of "identity + default session"; cleanup failure is logged but
    /// never masks the registry error.
    async fn discard_session(&self, session_id: &str) -> bool {
        match self
            .service
            .delete_session_tree(&|_: &str| {}, session_id)
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

    fn discard_profile(&self, profile_id: &str) -> bool {
        match self.delete_profile(profile_id) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    profile_id,
                    %error,
                    "could not remove the unowned bot profile"
                );
                false
            }
        }
    }

    fn commit(&self, record: BotRecord) -> Result<(), BotError> {
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
        Ok(())
    }

    fn remove_record(&self, bot_id: &str) -> Result<BotRecord, BotError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let index = state
            .iter()
            .position(|bot| bot.id == bot_id)
            .ok_or_else(|| BotError::NotFound(bot_id.to_string()))?;
        let removed = state.remove(index);
        if let Err(error) = save_registry(&self.path, &state) {
            state.insert(index, removed.clone());
            return Err(error);
        }
        Ok(removed)
    }

    /// Remove an uncommitted in-memory reservation and its best-effort
    /// profile/session cleanup.
    fn remove_reserved(&self, bot_id: &str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.retain(|bot| bot.id != bot_id);
        if let Err(error) = save_registry(&self.path, &state) {
            tracing::warn!(bot_id, %error, "could not rewrite bot registry after reservation removal");
        }
    }

    fn emit(&self, event: BotEvent) {
        let _ = self.events.send(event);
    }
}

/// Remove an uncommitted in-memory reservation on every exit path, including
/// async cancellation of the create call. A session delete is handed to a
/// background task because Drop cannot await; a profile delete is synchronous.
struct Reservation<'a> {
    service: &'a BotService,
    id: String,
    profile_id: Option<String>,
    session_id: Option<String>,
    committed: bool,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.service.remove_reserved(&self.id);
        if let Some(profile_id) = self.profile_id.take() {
            self.service.discard_profile(&profile_id);
        }
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
                .delete_session_tree(&|_: &str| {}, &session_id)
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

/// True when the profile id is derived from this bot's own id. A legacy
/// shared template or another bot's profile is not owned.
fn owns_profile(bot: &BotRecord) -> bool {
    let base = format!("{BOT_PROFILE_PREFIX}{}", bot.id.to_lowercase());
    bot.profile_id == base
        || bot
            .profile_id
            .strip_prefix(&base)
            .is_some_and(|rest| rest.starts_with('_'))
}

fn unique_profile_id(bot_id: &str, used: &HashSet<String>) -> String {
    let base = format!("{BOT_PROFILE_PREFIX}{}", bot_id.to_lowercase());
    if !used.contains(&base) {
        return base;
    }
    for suffix in 2.. {
        let candidate = format!("{base}_{suffix}");
        if !used.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("unbounded bot profile id suffixes")
}

fn normalized_model(profile: &AgentProfile) -> Option<String> {
    let model = profile.model_ref.trim();
    (!model.is_empty()).then(|| model.to_string())
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

fn load_registry(path: &Path) -> Result<Vec<BotRecord>, BotError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path)
        .map_err(|error| BotError::Registry(format!("read {}: {error}", path.display())))?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let file: BotRegistryFile = serde_json::from_str(&text)
        .map_err(|error| BotError::Registry(format!("parse {}: {error}", path.display())))?;
    Ok(file.bots)
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
            .filter(|bot| !bot.default_session_id.is_empty())
            .cloned()
            .collect(),
    };
    let body = serde_json::to_string_pretty(&file)
        .map_err(|error| BotError::Registry(format!("serialize bot registry: {error}")))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body)
        .map_err(|error| BotError::Registry(format!("write {}: {error}", tmp.display())))?;
    if let Err(first) = fs::rename(&tmp, path) {
        // Windows cannot replace an existing file with rename.
        if path.exists() {
            let _ = fs::remove_file(path);
        }
        fs::rename(&tmp, path).map_err(|_| {
            let _ = fs::remove_file(&tmp);
            BotError::Registry(format!("replace {}: {first}", path.display()))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnGuard;
    use crate::session::SessionManager;
    use tempfile::TempDir;

    struct Fixture {
        dir: TempDir,
        settings: Arc<SettingsWriter>,
        bots: Arc<BotService>,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            dir.path().join("sessions.db").to_string_lossy().to_string(),
        ));
        let service = Arc::new(SessionService::new(manager));
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
            settings,
            bots,
        }
    }

    #[tokio::test]
    async fn create_persists_identity_and_default_session() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Researcher", "finds papers", "default")
            .await
            .expect("create");

        assert!(!bot.default_session_id.is_empty());
        assert!(is_bot_profile_id(&bot.profile_id));
        assert_eq!(
            f.bots.bot_for_session(&bot.default_session_id),
            Some(bot.clone())
        );
        assert_eq!(f.bots.find("researcher"), Some(bot.clone()));
        assert!(
            f.settings
                .load_settings()
                .expect("settings")
                .agents
                .contains_key(&bot.profile_id)
        );

        let reloaded = BotService::new(
            Arc::clone(f.bots.service()),
            Arc::clone(&f.settings),
            None,
            "/proj",
            f.dir.path().join("bots.json"),
        )
        .expect("reload");
        assert_eq!(reloaded.list(), vec![bot]);
    }

    #[tokio::test]
    async fn duplicate_names_and_quota_are_rejected() {
        let f = fixture();
        f.bots
            .create_user_bot("One", "", "default")
            .await
            .expect("first");
        let error = f
            .bots
            .create_user_bot("one", "", "default")
            .await
            .expect_err("duplicate");
        assert!(matches!(error, BotError::NameTaken(_)));

        for index in 1..MAX_BOTS_PER_WORKSPACE {
            f.bots
                .create_user_bot(&format!("Bot {index}"), "", "default")
                .await
                .expect("under quota");
        }
        let error = f
            .bots
            .create_user_bot("Overflow", "", "default")
            .await
            .expect_err("quota");
        assert!(matches!(error, BotError::QuotaExceeded));
    }

    #[tokio::test]
    async fn each_bot_owns_an_independent_profile() {
        let f = fixture();
        let first = f
            .bots
            .create_user_bot("First", "", "default")
            .await
            .expect("first");
        let second = f
            .bots
            .create_user_bot("Second", "", "default")
            .await
            .expect("second");
        assert_ne!(first.profile_id, second.profile_id);

        let mut edit =
            f.settings.load_settings().expect("settings").agents[&first.profile_id].clone();
        edit.system_prompt = "only first changes".into();
        f.bots
            .update(
                &first.id,
                BotUpdate {
                    profile: Some(edit),
                    ..BotUpdate::default()
                },
            )
            .await
            .expect("update");

        let settings = f.settings.load_settings().expect("settings");
        assert_eq!(
            settings.agents[&first.profile_id].system_prompt,
            "only first changes"
        );
        assert_ne!(
            settings.agents[&second.profile_id].system_prompt,
            "only first changes"
        );
    }

    #[tokio::test]
    async fn delete_removes_session_profile_and_frees_quota() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Delete me", "", "default")
            .await
            .expect("create");
        f.bots.delete(&bot.id).await.expect("delete");

        assert!(f.bots.list().is_empty());
        assert!(f.bots.service().status(&bot.default_session_id).is_none());
        assert!(
            !f.settings
                .load_settings()
                .expect("settings")
                .agents
                .contains_key(&bot.profile_id)
        );

        for index in 0..MAX_BOTS_PER_WORKSPACE {
            f.bots
                .create_user_bot(&format!("Replacement {index}"), "", "default")
                .await
                .expect("quota released");
        }
    }

    #[tokio::test]
    async fn update_moves_to_folder_and_folder_delete_ungroups() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Folder bot", "", "default")
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
        assert_eq!(updated.folder_id.as_deref(), Some(folder_id.as_str()));

        f.bots.clear_folder(&folder_id);
        let moved = f.bots.find(&bot.id).expect("bot");
        assert!(moved.folder_id.is_none());
    }

    #[tokio::test]
    async fn update_rejects_a_session_folder() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Scoped bot", "", "default")
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
    async fn deleted_default_session_is_recreated_on_open() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Recreate", "", "default")
            .await
            .expect("create");
        f.bots
            .service()
            .delete_session_tree(&|_: &str| {}, &bot.default_session_id)
            .await
            .expect("delete default session");

        let refreshed = f
            .bots
            .ensure_default_session(&bot.id)
            .await
            .expect("recreate");
        assert_ne!(refreshed.default_session_id, bot.default_session_id);
        assert!(
            f.bots
                .service()
                .status(&refreshed.default_session_id)
                .is_some()
        );
        assert_eq!(
            f.bots.find(&bot.id).expect("bot").default_session_id,
            refreshed.default_session_id
        );
    }

    #[tokio::test]
    async fn bot_events_cover_create_update_and_delete() {
        let f = fixture();
        let mut events = f.bots.subscribe();
        let bot = f
            .bots
            .create_user_bot("Event bot", "", "default")
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
            .create_user_bot("Sender", "", "default")
            .await
            .expect("sender");
        let target = f
            .bots
            .create_user_bot("Target", "", "default")
            .await
            .expect("target");

        let delivered = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&delivered);
        f.bots.set_turn_runner(Arc::new(move |session_id: &str| {
            seen.lock().unwrap().push(session_id.to_string());
        }));

        let sent = f
            .bots
            .send_from_bot(&sender.default_session_id, "target", "  hello  ")
            .await
            .expect("send");

        assert_eq!(sent.id, target.id);
        assert_eq!(
            delivered.lock().unwrap().as_slice(),
            std::slice::from_ref(&target.default_session_id)
        );
        let pending = f
            .bots
            .service()
            .pending_messages_snapshot(&target.default_session_id);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text, "[from bot Sender]\nhello");
    }

    #[tokio::test]
    async fn bot_created_bots_stop_at_depth_one() {
        let f = fixture();
        let user_bot = f
            .bots
            .create_user_bot("Parent", "", "default")
            .await
            .expect("user bot");

        assert!(matches!(
            f.bots
                .create_from_bot("not-a-bot-session", "Child", "")
                .await,
            Err(BotError::NotBotSession)
        ));

        let child = f
            .bots
            .create_from_bot(&user_bot.default_session_id, "Child", "")
            .await
            .expect("child");
        assert_eq!(child.depth, 1);
        assert_eq!(child.owner_bot_id.as_deref(), Some(user_bot.id.as_str()));
        assert_ne!(child.profile_id, user_bot.profile_id);

        let error = f
            .bots
            .create_from_bot(&child.default_session_id, "Grandchild", "")
            .await
            .expect_err("depth");
        assert!(matches!(error, BotError::DepthExceeded { .. }));
    }

    #[tokio::test]
    async fn legacy_bot_is_migrated_to_an_owned_profile() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            dir.path().join("sessions.db").to_string_lossy().to_string(),
        ));
        let service = Arc::new(SessionService::new(manager));
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
            .expect("template");
        let registry = dir.path().join("bots.json");
        fs::write(
            &registry,
            serde_json::to_string_pretty(&serde_json::json!({
                "version": 1,
                "bots": [{
                    "id": "legacy",
                    "name": "Legacy",
                    "agent_id": "default",
                    "depth": 0,
                    "created_at": 1,
                    "default_session_id": "missing"
                }]
            }))
            .expect("json"),
        )
        .expect("registry");

        let bots = BotService::new(service, Arc::clone(&settings), None, "/proj", registry)
            .expect("migrate");
        let record = bots.find("legacy").expect("legacy bot");
        assert!(is_bot_profile_id(&record.profile_id));
        assert!(
            settings
                .load_settings()
                .expect("settings")
                .agents
                .contains_key(&record.profile_id)
        );
    }

    #[tokio::test]
    async fn restart_finds_the_durable_default_session_instead_of_minting_one() {
        let dir = tempfile::tempdir().expect("tempdir");
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
            .expect("template");
        let db_path = dir.path().join("sessions.db").to_string_lossy().to_string();
        let (bot_id, session_id) = {
            let manager = Arc::new(SessionManager::new_for_test(
                Arc::new(TurnGuard::new()),
                db_path.clone(),
            ));
            let service = Arc::new(SessionService::new(manager));
            let bots = Arc::new(
                BotService::new(
                    Arc::clone(&service),
                    Arc::clone(&settings),
                    None,
                    "/proj",
                    dir.path().join("bots.json"),
                )
                .expect("bots"),
            );
            let bot = bots
                .create_user_bot("Restart", "", "default")
                .await
                .expect("create");
            (bot.id, bot.default_session_id)
        };

        // A fresh manager has no hydrated records; only the durable row exists.
        let manager = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            db_path,
        ));
        let service = Arc::new(SessionService::new(Arc::clone(&manager)));
        let bots = BotService::new(
            Arc::clone(&service),
            Arc::clone(&settings),
            None,
            "/proj",
            dir.path().join("bots.json"),
        )
        .expect("restarted bots");
        assert!(service.status(&session_id).is_none());
        let reopened = bots.ensure_default_session(&bot_id).await.expect("reopen");
        assert_eq!(reopened.default_session_id, session_id);
        assert_eq!(
            service
                .list_sessions()
                .expect("sessions")
                .iter()
                .filter(|row| row.agent_id == reopened.profile_id)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn migration_rebinds_the_default_session_to_the_owned_profile() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Legacy session", "", "default")
            .await
            .expect("create");
        let registry = f.dir.path().join("bots.json");
        let mut file: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&registry).expect("registry")).expect("json");
        file["bots"][0]["profile_id"] = serde_json::json!("default");
        fs::write(
            &registry,
            serde_json::to_string_pretty(&file).expect("json"),
        )
        .expect("write");

        let migrated = BotService::new(
            Arc::clone(f.bots.service()),
            Arc::clone(&f.settings),
            None,
            "/proj",
            registry,
        )
        .expect("migrate");
        let migrated = migrated.find(&bot.id).expect("bot");
        assert_ne!(migrated.profile_id, "default");
        assert_eq!(
            f.bots
                .service()
                .agent_id(&bot.default_session_id)
                .as_deref(),
            Some(migrated.profile_id.as_str())
        );
    }

    #[tokio::test]
    async fn failed_registry_write_removes_the_unowned_session_and_profile() {
        let f = fixture();
        // A directory at the registry path makes the atomic replace fail after
        // the default session and bot profile have already been created.
        std::fs::create_dir(f.dir.path().join("bots.json")).expect("block registry path");

        let error = f
            .bots
            .create_user_bot("Orphan", "", "default")
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
            f.settings
                .load_settings()
                .expect("settings")
                .agents
                .keys()
                .all(|id| !is_bot_profile_id(id)),
            "failed create must not leave a bot profile behind"
        );
    }
}
