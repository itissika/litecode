use std::sync::{Arc, RwLock};

use axum::extract::FromRef;
use tokio::sync::Mutex;

use crate::config::settings_writer::SettingsChangedEvent;
use crate::config::{ResolvedConfig, SettingsWriter, TurnGuard, WorkspaceState};
use crate::engines::WorkspaceEngines;
use crate::ide_base::IdeBaseHandle;
use crate::optional::EngineManager;
use crate::runtime::RuntimeHandle;
use crate::session::{SessionManager, WorkspaceWriteLease};
use crate::terminal::{TerminalHub, install_hub};
use crate::workspace::{WorkspaceService, WorkspaceWatcher};

#[derive(Clone)]
pub struct ServeState {
    /// Shared IDE service graph. Legacy field aliases below remain while
    /// adapters migrate to this single ownership boundary.
    pub ide: Arc<IdeBaseHandle>,
    pub runtime: Arc<RwLock<RuntimeHandle>>,
    pub engine_manager: Arc<EngineManager>,
    pub workspace_engines: Arc<WorkspaceEngines>,
    pub terminal_hub: Arc<TerminalHub>,
    pub session_id: Option<String>,
    pub auth_token: Option<String>,
    pub workspace: Arc<WorkspaceService>,
    pub turn_guard: Arc<TurnGuard>,
    pub sessions: Arc<SessionManager>,
    pub settings_writer: Arc<SettingsWriter>,
    pub watcher: Arc<Mutex<Option<Arc<WorkspaceWatcher>>>>,
    /// Cross-process exclusive write lease held for the lifetime of this serve process.
    pub workspace_lock: Arc<std::sync::Mutex<WorkspaceWriteLease>>,
}

impl ServeState {
    pub fn new(
        resolved: ResolvedConfig,
        agent_name: String,
        workspace_state: WorkspaceState,
        engine_manager: Arc<EngineManager>,
        workspace_engines: Arc<WorkspaceEngines>,
        session_id: Option<String>,
        auth_token: Option<String>,
        turn_guard: Arc<TurnGuard>,
        settings_writer: Arc<SettingsWriter>,
    ) -> anyhow::Result<Self> {
        let project = workspace_state.workspace_root.clone();
        Self::with_project(
            resolved,
            agent_name,
            workspace_state,
            engine_manager,
            workspace_engines,
            session_id,
            auth_token,
            project,
            turn_guard,
            settings_writer,
        )
    }

    /// Build serve state rooted at an explicit project path (tests).
    pub fn with_project(
        resolved: ResolvedConfig,
        agent_name: String,
        workspace_state: WorkspaceState,
        engine_manager: Arc<EngineManager>,
        workspace_engines: Arc<WorkspaceEngines>,
        session_id: Option<String>,
        auth_token: Option<String>,
        project: std::path::PathBuf,
        turn_guard: Arc<TurnGuard>,
        settings_writer: Arc<SettingsWriter>,
    ) -> anyhow::Result<Self> {
        // Snapshot the sessions DB path up front: `workspace_state` is moved
        // into `RuntimeHandle::new` below, so we cannot borrow it afterwards.
        let sessions_db_path = workspace_state.paths.sessions_db.clone();
        let snapshots_dir = workspace_state.paths.snapshots_dir.clone();
        let litecode_dir = sessions_db_path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| workspace_state.workspace_root.join(".litecode"));
        let workspace_lock = WorkspaceWriteLease::acquire(&litecode_dir).map_err(|e| {
            anyhow::anyhow!(
                "cannot lock workspace at {} ({e}). Close any other Litecode window using this folder, then retry.",
                litecode_dir.display()
            )
        })?;
        let session_data =
            crate::session::SessionData::open(&workspace_lock, &sessions_db_path).map_err(|e| {
                anyhow::anyhow!(
                    "cannot open sessions.db at {} ({e})",
                    sessions_db_path.display()
                )
            })?;
        match crate::session::snapshot::maintain_snapshots(
            &snapshots_dir,
            &session_data.list_session_ids_blocking().unwrap_or_default(),
        ) {
            Ok(report) if report.orphans_removed > 0 || report.stale_removed > 0 => {
                tracing::info!(
                    orphans = report.orphans_removed,
                    stale = report.stale_removed,
                    "snapshot maintenance"
                );
            }
            Err(e) => tracing::warn!(error = %e, "snapshot maintenance failed"),
            _ => {}
        }
        let workspace = WorkspaceService::new(project.clone())?;
        let terminal_hub = Arc::new(TerminalHub::new());
        install_hub(Arc::clone(&terminal_hub));
        let ide = IdeBaseHandle::new(
            Arc::clone(&workspace),
            Arc::clone(&workspace_engines),
            Arc::clone(&terminal_hub),
        );
        let settings_revision = settings_writer.revision_handle();
        let runtime = RuntimeHandle::new(
            resolved,
            agent_name,
            workspace_state,
            Arc::clone(&engine_manager),
            Arc::clone(&workspace_engines),
            Arc::clone(&ide),
            settings_revision,
            settings_writer.db_path().to_path_buf(),
        );
        let runtime = Arc::new(RwLock::new(runtime));
        settings_writer.set_runtime(Arc::clone(&runtime));
        let sessions = Arc::new(SessionManager::from_data(turn_guard.clone(), session_data));
        workspace_engines.set_session_reader(sessions.reader());
        crate::runtime::bash_auto_turn::install_idle_auto_turn(
            Arc::clone(&terminal_hub),
            Arc::clone(&runtime),
            Arc::clone(&sessions),
            project.clone(),
        );
        {
            let sessions = Arc::clone(&sessions);
            let hub = Arc::clone(&terminal_hub);
            terminal_hub
                .jobs
                .set_jobs_changed_handler(Arc::new(move |session_id: String| {
                    let snapshot = hub.jobs.wire_snapshot(&session_id);
                    let _ = sessions.publish_internal(
                        &session_id,
                        crate::runtime::observer::InternalEvent::BashJobs { snapshot },
                    );
                }));
        }
        Ok(Self {
            ide,
            runtime,
            engine_manager,
            workspace_engines,
            terminal_hub,
            session_id,
            auth_token,
            workspace,
            turn_guard: turn_guard.clone(),
            sessions,
            settings_writer,
            watcher: Arc::new(Mutex::new(None)),
            workspace_lock: Arc::new(std::sync::Mutex::new(workspace_lock)),
        })
    }

    pub fn settings_revision(&self) -> u64 {
        self.settings_writer.current_revision()
    }

    pub fn subscribe_settings(&self) -> tokio::sync::broadcast::Receiver<SettingsChangedEvent> {
        self.settings_writer.subscribe()
    }

    pub fn runtime_snapshot(&self) -> RuntimeHandle {
        self.runtime.read().expect("runtime lock").clone()
    }
}

impl FromRef<ServeState> for Arc<WorkspaceService> {
    fn from_ref(state: &ServeState) -> Self {
        state.workspace.clone()
    }
}

impl FromRef<ServeState> for Arc<SettingsWriter> {
    fn from_ref(state: &ServeState) -> Self {
        state.settings_writer.clone()
    }
}
