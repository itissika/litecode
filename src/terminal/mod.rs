//! TerminalHub — process-scoped PTY session registry (foundation).

mod agent_jobs;
mod ansi;
mod error;
mod pty;
mod shell;
mod tee;

pub use ansi::strip_ansi;
pub use tee::{FILE_MAX, INLINE_FULL, INLINE_HEAD, INLINE_TAIL, TeeCapture};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::mpsc;
use uuid::Uuid;

pub use agent_jobs::{
    AgentJobRecord, AgentJobRegistry, BashJobWire, BashJobsSnapshot, BashTailView, BashWaitWire,
    ExitNotice, RunningJob, WaitOutcome, command_preview,
};
pub use error::{TerminalError, TerminalResult};
pub use shell::{ShellSpec, default_shell, shell_command};

use shell::resolve_cwd;

/// UTF-8 lossy payload push (human WS) + exit notification.
#[derive(Debug, Clone)]
pub struct TerminalEvent {
    pub id: String,
    pub kind: TerminalEventKind,
}

#[derive(Debug, Clone)]
pub enum TerminalEventKind {
    Data(String),
    Exit { code: Option<u32> },
}

/// Server-generated identity for one WebSocket connection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectionId(String);

impl ConnectionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SessionOwner {
    Connection(ConnectionId),
    Agent,
}

struct SessionEntry {
    owner: SessionOwner,
    session: Arc<pty::PtySession>,
    exit_sent: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub alive: bool,
    pub exit_code: Option<u32>,
    /// Background agent sessions tee PTY output here for `read` tool consumption.
    pub output_path: Option<PathBuf>,
}

/// Result of [`TerminalHub::spawn_command`].
#[derive(Debug, Clone)]
pub struct SpawnCommandResult {
    pub id: String,
    pub output_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ExecResult {
    /// In-memory window: full stdout when small, else head+tail concatenated.
    pub output: String,
    pub exit_code: Option<u32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub capture: TeeCapture,
}

/// Create options for an interactive human session.
#[derive(Debug, Clone)]
pub struct CreateOptions {
    pub cols: u16,
    pub rows: u16,
    pub cwd: Option<PathBuf>,
}

impl Default for CreateOptions {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            cwd: None,
        }
    }
}

static GLOBAL_HUB: OnceLock<Arc<TerminalHub>> = OnceLock::new();

/// Install the process-global hub (serve boot). Idempotent first-wins.
pub fn install_hub(hub: Arc<TerminalHub>) {
    let _ = GLOBAL_HUB.set(hub);
}

/// Returns the global TerminalHub instance.
///
/// This function is intended for WebSocket terminal event forwarding and
/// CLI one-shot commands only. Agent tools should use the injected
/// `TerminalHub` from `IdeBaseHandle` instead.
#[doc(hidden)]
pub fn hub() -> Arc<TerminalHub> {
    GLOBAL_HUB
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(TerminalHub::new()))
}

pub struct TerminalHub {
    sessions: Arc<Mutex<HashMap<String, SessionEntry>>>,
    connections: Arc<Mutex<HashMap<ConnectionId, mpsc::UnboundedSender<TerminalEvent>>>>,
    pub jobs: Arc<agent_jobs::AgentJobRegistry>,
}

impl Default for TerminalHub {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalHub {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            connections: Arc::new(Mutex::new(HashMap::new())),
            jobs: Arc::new(agent_jobs::AgentJobRegistry::new()),
        }
    }

    pub fn attach_connection(&self, id: ConnectionId) -> mpsc::UnboundedReceiver<TerminalEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.connections
            .lock()
            .expect("connections lock")
            .insert(id, tx);
        rx
    }

    /// Remove and terminate every interactive terminal owned by a disconnected WS.
    pub fn disconnect(&self, caller: &ConnectionId) {
        self.connections
            .lock()
            .expect("connections lock")
            .remove(caller);

        let removed = {
            let mut sessions = self.sessions.lock().expect("sessions lock");
            let ids = sessions
                .iter()
                .filter(|(_, entry)| entry.owner == SessionOwner::Connection(caller.clone()))
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| sessions.remove(&id))
                .collect::<Vec<_>>()
        };
        for entry in removed {
            entry.exit_sent.store(true, Ordering::Release);
            let _ = entry.session.kill();
            entry.session.try_reap();
        }
    }

    /// Human PTY ids stay unguessable (defense-in-depth; authorization uses owner).
    fn next_id(prefix: &str) -> String {
        format!("{prefix}_{}", Uuid::new_v4())
    }

    /// Agent bash ids: `bg_` / `bash_` + 8 hex chars. Not sequential; collision
    /// is retried against live sessions, job records, and leftover output files.
    fn next_agent_id(prefix: &str) -> String {
        let uuid = Uuid::new_v4();
        let bytes = uuid.as_bytes();
        format!(
            "{prefix}_{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3]
        )
    }

    fn agent_id_in_use(&self, id: &str) -> bool {
        if self.jobs.get(id).is_some() {
            return true;
        }
        self.sessions
            .lock()
            .expect("sessions lock")
            .contains_key(id)
    }

    fn alloc_agent_id(&self, prefix: &str, workspace_root: &Path) -> String {
        for _ in 0..16 {
            let id = Self::next_agent_id(prefix);
            if self.agent_id_in_use(&id) {
                continue;
            }
            let path = workspace_root
                .join(".litecode")
                .join("bash")
                .join(format!("{id}.output"));
            if path.exists() {
                continue;
            }
            return id;
        }
        Self::next_id(prefix)
    }

    fn start_reaper(
        &self,
        id: String,
        owner: SessionOwner,
        session: Arc<pty::PtySession>,
        exit_sent: Arc<AtomicBool>,
    ) {
        let sessions = Arc::clone(&self.sessions);
        let connections = Arc::clone(&self.connections);
        let jobs = Arc::clone(&self.jobs);
        let _ = std::thread::Builder::new()
            .name(format!("pty-reap-{id}"))
            .spawn(move || {
                while session.is_alive() {
                    session.try_reap();
                    if session.is_alive() {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                }
                session.try_reap();
                let code = session.exit_code();
                if exit_sent.swap(true, Ordering::AcqRel) {
                    return;
                }
                if matches!(owner, SessionOwner::Agent) {
                    jobs.finish(&id, code);
                }
                {
                    let mut sessions = sessions.lock().expect("sessions lock");
                    let same_entry = sessions
                        .get(&id)
                        .is_some_and(|entry| Arc::ptr_eq(&entry.exit_sent, &exit_sent));
                    if same_entry {
                        sessions.remove(&id);
                    }
                }
                if let SessionOwner::Connection(connection_id) = owner {
                    let tx = connections
                        .lock()
                        .expect("connections lock")
                        .get(&connection_id)
                        .cloned();
                    if let Some(tx) = tx {
                        let _ = tx.send(TerminalEvent {
                            id,
                            kind: TerminalEventKind::Exit { code },
                        });
                    }
                }
            });
    }

    /// Interactive shell session owned by one WebSocket connection.
    pub fn create(&self, caller: &ConnectionId, opts: CreateOptions) -> TerminalResult<String> {
        if !self
            .connections
            .lock()
            .expect("connections lock")
            .contains_key(caller)
        {
            return Err(TerminalError::Closed("connection".into()));
        }

        let id = Self::next_id("term");
        let cwd = resolve_cwd(opts.cwd.as_deref());
        let shell = shell::default_shell();
        let exit_sent = Arc::new(AtomicBool::new(false));

        let connections = Arc::clone(&self.connections);
        let id_data = id.clone();
        let owner_data = caller.clone();
        let exit_sent_data = Arc::clone(&exit_sent);
        let on_data: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |chunk: &str| {
            if exit_sent_data.load(Ordering::Acquire) {
                return;
            }
            let tx = connections
                .lock()
                .expect("connections lock")
                .get(&owner_data)
                .cloned();
            if let Some(tx) = tx {
                let _ = tx.send(TerminalEvent {
                    id: id_data.clone(),
                    kind: TerminalEventKind::Data(chunk.to_string()),
                });
            }
        });

        let sessions = Arc::clone(&self.sessions);
        let connections = Arc::clone(&self.connections);
        let id_exit = id.clone();
        let owner_exit = caller.clone();
        let exit_sent_cb = Arc::clone(&exit_sent);
        let on_exit: Arc<dyn Fn(Option<u32>) + Send + Sync> = Arc::new(move |code| {
            if exit_sent_cb.swap(true, Ordering::AcqRel) {
                return;
            }
            {
                let mut sessions = sessions.lock().expect("sessions lock");
                let same_entry = sessions
                    .get(&id_exit)
                    .is_some_and(|entry| Arc::ptr_eq(&entry.exit_sent, &exit_sent_cb));
                if same_entry {
                    sessions.remove(&id_exit);
                }
            }
            let tx = connections
                .lock()
                .expect("connections lock")
                .get(&owner_exit)
                .cloned();
            if let Some(tx) = tx {
                let _ = tx.send(TerminalEvent {
                    id: id_exit.clone(),
                    kind: TerminalEventKind::Exit { code },
                });
            }
        });
        let session = Arc::new(pty::PtySession::spawn_interactive(
            id.clone(),
            &shell,
            &cwd,
            opts.cols.max(1),
            opts.rows.max(1),
            Some(on_data),
            Some(on_exit),
            &[],
        )?);
        self.sessions.lock().expect("sessions lock").insert(
            id.clone(),
            SessionEntry {
                owner: SessionOwner::Connection(caller.clone()),
                session: Arc::clone(&session),
                exit_sent: Arc::clone(&exit_sent),
            },
        );
        // The reader can observe exit before a very short-lived process is inserted.
        if exit_sent.load(Ordering::Acquire) {
            let mut sessions = self.sessions.lock().expect("sessions lock");
            let same_entry = sessions
                .get(&id)
                .is_some_and(|entry| Arc::ptr_eq(&entry.exit_sent, &exit_sent));
            if same_entry {
                sessions.remove(&id);
            }
        }
        self.start_reaper(
            id.clone(),
            SessionOwner::Connection(caller.clone()),
            session,
            exit_sent,
        );
        Ok(id)
    }

    fn owned_session(
        &self,
        caller: &ConnectionId,
        id: &str,
    ) -> TerminalResult<Arc<pty::PtySession>> {
        let sessions = self.sessions.lock().expect("sessions lock");
        sessions
            .get(id)
            .filter(|entry| entry.owner == SessionOwner::Connection(caller.clone()))
            .map(|entry| Arc::clone(&entry.session))
            // Foreign and absent IDs intentionally have the same response.
            .ok_or_else(|| TerminalError::SessionNotFound(id.to_string()))
    }

    pub fn write(&self, caller: &ConnectionId, id: &str, data: &[u8]) -> TerminalResult<()> {
        let session = self.owned_session(caller, id)?;
        session.write(data)
    }

    pub fn resize(
        &self,
        caller: &ConnectionId,
        id: &str,
        cols: u16,
        rows: u16,
    ) -> TerminalResult<()> {
        let session = self.owned_session(caller, id)?;
        session.resize(cols.max(1), rows.max(1))
    }

    pub fn close(&self, caller: &ConnectionId, id: &str) -> TerminalResult<()> {
        let entry = {
            let mut sessions = self.sessions.lock().expect("sessions lock");
            let owned = sessions
                .get(id)
                .is_some_and(|entry| entry.owner == SessionOwner::Connection(caller.clone()));
            if !owned {
                return Err(TerminalError::SessionNotFound(id.to_string()));
            }
            sessions
                .remove(id)
                .expect("owned session disappeared while locked")
        };
        let _ = entry.session.kill();
        entry.session.try_reap();
        let code = entry.session.exit_code();
        if !entry.exit_sent.swap(true, Ordering::AcqRel) {
            let tx = self
                .connections
                .lock()
                .expect("connections lock")
                .get(caller)
                .cloned();
            if let Some(tx) = tx {
                let _ = tx.send(TerminalEvent {
                    id: id.to_string(),
                    kind: TerminalEventKind::Exit { code },
                });
            }
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let sessions = self
            .sessions
            .lock()
            .expect("sessions lock")
            .values()
            .map(|entry| Arc::clone(&entry.session))
            .collect::<Vec<_>>();
        sessions
            .into_iter()
            .map(|s| {
                s.try_reap();
                SessionInfo {
                    id: s.id.clone(),
                    alive: s.is_alive(),
                    exit_code: s.exit_code(),
                    output_path: s.output_path.clone(),
                }
            })
            .collect()
    }

    /// Drain in-memory PTY output (interactive/exec_once buffer only; bg tees to file).
    pub fn take_output(&self, id: &str) -> TerminalResult<String> {
        let session = self
            .sessions
            .lock()
            .expect("sessions lock")
            .get(id)
            .map(|entry| Arc::clone(&entry.session))
            .ok_or_else(|| TerminalError::SessionNotFound(id.to_string()))?;
        session.try_reap();
        Ok(session.take_output())
    }

    pub fn session_info(&self, id: &str) -> TerminalResult<SessionInfo> {
        let session = self
            .sessions
            .lock()
            .expect("sessions lock")
            .get(id)
            .map(|entry| Arc::clone(&entry.session))
            .ok_or_else(|| TerminalError::SessionNotFound(id.to_string()))?;
        session.try_reap();
        Ok(SessionInfo {
            id: session.id.clone(),
            alive: session.is_alive(),
            exit_code: session.exit_code(),
            output_path: session.output_path.clone(),
        })
    }

    /// One-shot command via default shell (agent foreground bash).
    ///
    /// Always tees stripped stdout to `.litecode/bash/<id>.output`. Timeout and
    /// cancel complete as `Ok` with flags set so the tool can point at the log.
    pub fn exec_once(
        &self,
        command: &str,
        workdir: Option<&Path>,
        timeout: Duration,
        cancel: Option<&tokio_util::sync::CancellationToken>,
        workspace_root: &Path,
    ) -> TerminalResult<ExecResult> {
        let cwd = resolve_cwd(workdir);
        let shell = shell::shell_command(command);
        let id = self.alloc_agent_id("bash", workspace_root);
        let output_path = agent_output_path(workspace_root, &id)?;
        let tee = Arc::new(Mutex::new(tee::BoundedTee::create(output_path)?));
        let finish = pty::exec_once(&shell, &cwd, timeout, cancel, Arc::clone(&tee))?;
        let capture = {
            let mut g = tee
                .lock()
                .map_err(|_| TerminalError::Io("bash tee lock poisoned".into()))?;
            let _ = g.flush_file();
            g.snapshot_capture()
        };
        let output = capture.snapshot();
        match finish {
            pty::ExecFinish::Exited(exit_code) => Ok(ExecResult {
                output,
                exit_code,
                timed_out: false,
                cancelled: false,
                capture,
            }),
            pty::ExecFinish::TimedOut => Ok(ExecResult {
                output,
                exit_code: None,
                timed_out: true,
                cancelled: false,
                capture,
            }),
            pty::ExecFinish::Cancelled => Ok(ExecResult {
                output,
                exit_code: None,
                timed_out: false,
                cancelled: true,
                capture,
            }),
        }
    }

    /// Background / detachable agent command. Tees PTY output to a workspace file.
    pub fn spawn_command(
        &self,
        command: &str,
        workdir: Option<&Path>,
        workspace_root: &Path,
        session_id: &str,
        call_id: &str,
    ) -> TerminalResult<SpawnCommandResult> {
        let id = self.alloc_agent_id("bg", workspace_root);
        let cwd = resolve_cwd(workdir);
        let shell = shell::shell_command(command);
        let output_path = agent_output_path(workspace_root, &id)?;
        let tee = Arc::new(Mutex::new(tee::BoundedTee::create(output_path.clone())?));
        let session_key = if session_id.is_empty() {
            "_"
        } else {
            session_id
        };

        let tee_data = Arc::clone(&tee);
        let on_data: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |chunk: &str| {
            if let Ok(mut t) = tee_data.lock() {
                t.push_raw(chunk);
            }
        });
        let tee_exit = Arc::clone(&tee);
        let sessions = Arc::clone(&self.sessions);
        let jobs = Arc::clone(&self.jobs);
        let exit_sent = Arc::new(AtomicBool::new(false));
        let exit_sent_cb = Arc::clone(&exit_sent);
        let id_exit = id.clone();
        let on_exit: Arc<dyn Fn(Option<u32>) + Send + Sync> = Arc::new(move |code| {
            if let Ok(mut t) = tee_exit.lock() {
                let footer = match code {
                    Some(c) => format!("\n\n[exited with code {c}]\n"),
                    None => "\n\n[exited]\n".to_string(),
                };
                t.push_raw(&footer);
                let _ = t.flush_file();
            }
            if code.is_none() {
                return;
            }
            if !exit_sent_cb.swap(true, Ordering::AcqRel) {
                jobs.finish(&id_exit, code);
                let mut sessions = sessions.lock().expect("sessions lock");
                let same_entry = sessions
                    .get(&id_exit)
                    .is_some_and(|entry| Arc::ptr_eq(&entry.exit_sent, &exit_sent_cb));
                if same_entry {
                    sessions.remove(&id_exit);
                }
            }
        });
        self.jobs.insert(
            id.clone(),
            AgentJobRecord {
                session_id: session_key.to_string(),
                call_id: call_id.to_string(),
                command_preview: command_preview(command),
                output_path: output_path.clone(),
                tee: Arc::clone(&tee),
                alive: true,
                exit_code: None,
                started_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64,
                user_killed: false,
            },
        );
        let mut session = match pty::PtySession::spawn_interactive(
            id.clone(),
            &shell,
            &cwd,
            80,
            24,
            Some(on_data),
            Some(on_exit),
            pty::AGENT_NON_INTERACTIVE_ENV,
        ) {
            Ok(s) => s,
            Err(e) => {
                self.jobs.finish(&id, None);
                return Err(e);
            }
        };
        session.output_path = Some(output_path.clone());
        let session = Arc::new(session);
        self.sessions.lock().expect("sessions lock").insert(
            id.clone(),
            SessionEntry {
                owner: SessionOwner::Agent,
                session: Arc::clone(&session),
                exit_sent: Arc::clone(&exit_sent),
            },
        );
        if exit_sent.load(Ordering::Acquire) {
            let mut sessions = self.sessions.lock().expect("sessions lock");
            let same_entry = sessions
                .get(&id)
                .is_some_and(|entry| Arc::ptr_eq(&entry.exit_sent, &exit_sent));
            if same_entry {
                sessions.remove(&id);
            }
        }
        self.start_reaper(id.clone(), SessionOwner::Agent, session, exit_sent);
        Ok(SpawnCommandResult { id, output_path })
    }

    /// Kill a background/command session (agent `kill_shell`).
    pub fn kill(&self, id: &str) -> TerminalResult<SessionInfo> {
        self.kill_with(id, false)
    }

    /// Kill from the bash card Kill control (human).
    pub fn kill_from_ui(&self, id: &str) -> TerminalResult<SessionInfo> {
        self.kill_with(id, true)
    }

    fn kill_with(&self, id: &str, user_killed: bool) -> TerminalResult<SessionInfo> {
        if user_killed {
            self.jobs.mark_user_kill(id);
        }
        // Snapshot the session handle, then release the table lock: the kill +
        // reap poll below can take up to ~1s and must not block every other
        // terminal operation (3.3).
        let session = {
            let sessions = self.sessions.lock().expect("sessions lock");
            sessions
                .get(id)
                .filter(|entry| entry.owner == SessionOwner::Agent)
                .map(|entry| Arc::clone(&entry.session))
        };
        let Some(session) = session else {
            // PTY row may already be gone (reaper/on_exit); still seal the job
            // so a human Kill reaches the agent mailbox.
            return self.finish_job_without_pty(id, user_killed);
        };
        if session.is_alive() {
            let _ = session.kill();
            for _ in 0..50 {
                session.try_reap();
                if !session.is_alive() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if session.is_alive() {
                let _ = session.kill();
                session.try_reap();
            }
        }
        let info = SessionInfo {
            id: session.id.clone(),
            alive: session.is_alive(),
            exit_code: session.exit_code(),
            output_path: session.output_path.clone(),
        };
        // close_agent / reaper may set exit_sent before on_exit runs, which
        // skips jobs.finish. Mark the registry here so wait_shell and running
        // lists observe the killed job immediately.
        if user_killed {
            self.jobs.finish_user_killed(id, info.exit_code);
        } else {
            self.jobs.finish(id, info.exit_code);
        }
        Ok(info)
    }

    fn finish_job_without_pty(&self, id: &str, user_killed: bool) -> TerminalResult<SessionInfo> {
        let Some((_, exit_code, _, output_path)) = self.jobs.get(id) else {
            return Err(TerminalError::SessionNotFound(id.to_string()));
        };
        if user_killed {
            self.jobs.finish_user_killed(id, exit_code);
        } else {
            self.jobs.finish(id, exit_code);
        }
        Ok(SessionInfo {
            id: id.to_string(),
            alive: false,
            exit_code,
            output_path: Some(output_path),
        })
    }

    /// Remove an Agent-owned background session; interactive WS sessions are inaccessible.
    pub fn close_agent(&self, id: &str) -> TerminalResult<()> {
        let entry = {
            let mut sessions = self.sessions.lock().expect("sessions lock");
            let is_agent = sessions
                .get(id)
                .is_some_and(|entry| entry.owner == SessionOwner::Agent);
            if !is_agent {
                return Err(TerminalError::SessionNotFound(id.to_string()));
            }
            sessions
                .remove(id)
                .expect("agent session disappeared while locked")
        };
        entry.exit_sent.store(true, Ordering::Release);
        let _ = entry.session.kill();
        entry.session.try_reap();
        Ok(())
    }

    /// Remove a finished session from the registry (output file is retained for `read`).
    pub fn remove_if_dead(&self, id: &str) {
        let snapshot = self
            .sessions
            .lock()
            .expect("sessions lock")
            .get(id)
            .map(|entry| (Arc::clone(&entry.session), Arc::clone(&entry.exit_sent)));
        let Some((session, generation)) = snapshot else {
            return;
        };
        session.try_reap();
        if !session.is_alive() {
            let mut sessions = self.sessions.lock().expect("sessions lock");
            let same_entry = sessions
                .get(id)
                .is_some_and(|entry| Arc::ptr_eq(&entry.exit_sent, &generation));
            if same_entry {
                sessions.remove(id);
            }
        }
    }
}

/// `<workspace>/.litecode/bash/<id>.output` — inside the workspace so `read` (Safe) can open it.
pub(crate) fn agent_output_path(workspace_root: &Path, id: &str) -> TerminalResult<PathBuf> {
    let bash_dir = workspace_root.join(".litecode").join("bash");
    std::fs::create_dir_all(&bash_dir).map_err(|e| TerminalError::Io(e.to_string()))?;
    Ok(bash_dir.join(format!("{id}.output")))
}

#[cfg(test)]
mod tests {
    use super::*;

    // G6: ids are unguessable random strings (not sequential/guessable), and a
    // cross-session kill/close on an id the caller cannot know is rejected.
    #[test]
    fn ids_are_unguessable_uuid_and_distinct() {
        let a = TerminalHub::next_id("term");
        let b = TerminalHub::next_id("term");
        // Format is `prefix_<uuid>` — not a small sequential integer.
        assert!(a.starts_with("term_"));
        assert!(b.starts_with("term_"));
        assert_ne!(a, b);
        let nonce_a = &a["term_".len()..];
        let nonce_b = &b["term_".len()..];
        assert!(nonce_a.len() >= 16, "nonce too short: {nonce_a}");
        assert!(nonce_b.len() >= 16);
        // A UUID cannot be trivially enumerated by incrementing.
        assert!(nonce_a.parse::<u64>().is_err(), "nonce must not be numeric");
    }

    #[test]
    fn agent_bash_ids_are_short_hex_not_sequential() {
        let a = TerminalHub::next_agent_id("bg");
        let b = TerminalHub::next_agent_id("bg");
        assert_ne!(a, b);
        for id in [&a, &b] {
            assert!(id.starts_with("bg_"), "{id}");
            let nonce = &id["bg_".len()..];
            assert_eq!(nonce.len(), 8, "{id}");
            assert!(
                nonce.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
                "nonce must be lowercase hex: {id}"
            );
        }
        let hub = TerminalHub::new();
        let dir = tempfile::tempdir().unwrap();
        let taken = "bg_aaaaaaaa";
        std::fs::create_dir_all(dir.path().join(".litecode").join("bash")).unwrap();
        std::fs::write(
            dir.path()
                .join(".litecode")
                .join("bash")
                .join(format!("{taken}.output")),
            b"",
        )
        .unwrap();
        // alloc skips leftover files; a colliding draw is retried (or falls back).
        for _ in 0..8 {
            let id = hub.alloc_agent_id("bg", dir.path());
            assert_ne!(id, taken);
            assert!(id.starts_with("bg_"));
        }
    }

    #[test]
    fn kill_rejects_human_pty_id() {
        let hub = TerminalHub::new();
        let caller = ConnectionId::new();
        let _events = hub.attach_connection(caller.clone());
        let id = hub
            .create(
                &caller,
                CreateOptions {
                    cols: 80,
                    rows: 24,
                    cwd: None,
                },
            )
            .expect("human pty");
        assert!(matches!(
            hub.kill(&id),
            Err(TerminalError::SessionNotFound(_))
        ));
    }

    #[test]
    fn kill_unknown_or_guessed_id_is_rejected() {
        let hub = TerminalHub::new();
        let caller = ConnectionId::new();
        let _events = hub.attach_connection(caller.clone());
        // A guessed id (even well-formed) that is not a registered session is rejected.
        let guess = TerminalHub::next_id("term");
        assert!(matches!(
            hub.kill(&guess),
            Err(TerminalError::SessionNotFound(_))
        ));
        assert!(matches!(
            hub.close(&caller, &guess),
            Err(TerminalError::SessionNotFound(_))
        ));
    }

    #[test]
    fn exec_once_timeout_kills_process_group_without_leak() {
        // A long-running sleep is killed at the timeout; the call returns Timeout
        // and the session is fully reaped (no lingering process / reader thread).
        let hub = TerminalHub::new();
        let dir = tempfile::tempdir().unwrap();
        let root = crate::config::path::canon_abs(dir.path()).unwrap();
        crate::config::workspace::set_runtime_paths(
            crate::config::resolved::WorkspacePaths::for_legacy_root(&root),
        );

        // Short sleep on purpose: the test must complete in seconds. If the
        // process-group kill regressed, the bounded join (pty.rs wait_join) still
        // caps the runtime instead of blocking until the child exits naturally.
        #[cfg(windows)]
        let command: &str = if crate::config::git_install::find_git_bash().is_some() {
            "sleep 5"
        } else {
            "Start-Sleep -Seconds 5"
        };
        #[cfg(not(windows))]
        let command = "sleep 5";

        let result = hub.exec_once(
            command,
            Some(&root),
            Duration::from_millis(500),
            None,
            &root,
        );
        assert!(
            result.as_ref().is_ok_and(|r| r.timed_out),
            "expected timeout, got: {result:?}"
        );
        crate::config::workspace::clear_runtime_paths();
    }

    #[test]
    fn exec_once_timeout_preserves_partial_output() {
        let hub = TerminalHub::new();
        let dir = tempfile::tempdir().unwrap();
        let root = crate::config::path::canon_abs(dir.path()).unwrap();
        crate::config::workspace::set_runtime_paths(
            crate::config::resolved::WorkspacePaths::for_legacy_root(&root),
        );

        // Print a marker, flush, then sleep past the timeout so we can assert
        // the pre-kill stdout survived into TerminalError::Timeout.
        #[cfg(windows)]
        let command: &str = if crate::config::git_install::find_git_bash().is_some() {
            "printf 'PARTIAL_MARKER\\n'; sleep 5"
        } else {
            "Write-Output 'PARTIAL_MARKER'; Start-Sleep -Seconds 5"
        };
        #[cfg(not(windows))]
        let command = "printf 'PARTIAL_MARKER\\n'; sleep 5";

        let result = hub.exec_once(
            command,
            Some(&root),
            Duration::from_millis(800),
            None,
            &root,
        );
        match result {
            Ok(exec) if exec.timed_out => {
                assert!(
                    exec.output.contains("PARTIAL_MARKER"),
                    "expected partial stdout before kill, got: {:?}",
                    exec.output
                );
            }
            other => panic!("expected timed-out ExecResult, got: {other:?}"),
        }
        crate::config::workspace::clear_runtime_paths();
    }
}
