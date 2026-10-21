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
    ExitNotice, PipeDecision, PipeDetach, PipeRequest, RunningJob, WaitOutcome, command_preview,
    supervise_pipe,
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

/// A freshly created interactive session: the pty id plus the shell that backs
/// it (`bash` / `powershell` / …), so the client can label the tab before the
/// user has run anything.
#[derive(Debug, Clone)]
pub struct CreatedTerminal {
    pub id: String,
    pub shell: String,
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
    pub fn create(
        &self,
        caller: &ConnectionId,
        opts: CreateOptions,
    ) -> TerminalResult<CreatedTerminal> {
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
            false,
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
        Ok(CreatedTerminal {
            id,
            shell: shell::shell_label(&shell),
        })
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

    /// Drain in-memory PTY output (interactive sessions only).
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

    /// Kill a registered pipe job by pid. Human PTY ids are not in the job registry.
    pub fn kill(&self, id: &str) -> TerminalResult<SessionInfo> {
        self.kill_registered_job(id, false)
    }

    /// Kill from the bash card. Same process-tree kill; a detached job's mailbox
    /// notice stays (`user_killed`) for `reminder/bash_exit`.
    pub fn kill_from_ui(&self, id: &str) -> TerminalResult<SessionInfo> {
        self.kill_registered_job(id, true)
    }

    fn kill_registered_job(&self, id: &str, user_killed: bool) -> TerminalResult<SessionInfo> {
        if self.is_human_pty(id) {
            return Err(TerminalError::SessionNotFound(id.to_string()));
        }
        let killed = self
            .jobs
            .kill_registered(id, user_killed)
            .ok_or_else(|| TerminalError::SessionNotFound(id.to_string()))?;
        Ok(SessionInfo {
            id: killed.id,
            alive: killed.alive,
            exit_code: killed.exit_code,
            output_path: killed.output_path,
        })
    }

    fn is_human_pty(&self, id: &str) -> bool {
        self.sessions
            .lock()
            .expect("sessions lock")
            .get(id)
            .is_some_and(|entry| matches!(entry.owner, SessionOwner::Connection(_)))
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

/// Workspace-relative path for logs and reminder fields.
/// Outside the workspace, the path itself is shown with forward slashes.
pub fn display_output_path(path: &Path, workspace_root: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(workspace_root) {
        return rel.to_string_lossy().replace('\\', "/");
    }
    if let (Ok(path_abs), Ok(root_abs)) = (
        crate::config::path::canon_abs(path),
        crate::config::path::canon_abs(workspace_root),
    ) && let Ok(rel) = path_abs.strip_prefix(&root_abs)
    {
        return rel.to_string_lossy().replace('\\', "/");
    }
    path.display().to_string().replace('\\', "/")
}

/// 8 hex chars for a log file stem. Not an id, and never a `call_id`
/// (`:` is illegal in a Windows file name).
pub(crate) fn bash_nonce() -> String {
    let bytes = Uuid::new_v4().into_bytes();
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_output_path_strips_workspace_root() {
        let root = Path::new("C:/proj");
        let path = root
            .join(".litecode")
            .join("run")
            .join("bg_aaa.stdout");
        assert_eq!(
            display_output_path(&path, root),
            ".litecode/run/bg_aaa.stdout"
        );
    }

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
        let dir = tempfile::tempdir().unwrap();
        let jobs = AgentJobRegistry::new();
        let taken = "bg_aaaaaaaa";
        let stdout = dir
            .path()
            .join(".litecode")
            .join("run")
            .join(format!("{taken}.stdout"));
        std::fs::create_dir_all(stdout.parent().unwrap()).unwrap();
        std::fs::write(&stdout, b"").unwrap();
        let mut seen = Vec::new();
        for _ in 0..8 {
            let id = super::agent_jobs::alloc_pipe_job_id(&jobs, dir.path());
            assert_ne!(id, taken);
            assert!(id.starts_with("bg_"), "{id}");
            let nonce = &id["bg_".len()..];
            assert_eq!(nonce.len(), 8, "{id}");
            assert!(
                nonce.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
                "nonce must be lowercase hex: {id}"
            );
            seen.push(id);
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 8);
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
            .expect("human pty")
            .id;
        assert!(matches!(
            hub.kill(&id),
            Err(TerminalError::SessionNotFound(_))
        ));
        assert!(
            hub.session_info(&id).is_ok(),
            "human pty must stay open"
        );
        hub.close(&caller, &id).expect("close human pty");
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
}
