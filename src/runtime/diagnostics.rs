//! Observe-only diagnostics: panic crash logs, last-turn heartbeat, tool audit JSONL.
//!
//! Write failures never change control flow or tool results (warn only).

use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Tracing target for tool-audit lines (pairs with JSONL on disk).
pub const TOOL_AUDIT_TARGET: &str = "litecode.tool_audit";

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

pub fn diagnostics_root(data_root: &Path) -> PathBuf {
    data_root.join("diagnostics")
}

/// Sanitize like `wire_dump::session_dir_name`: illegal → `_`, empty → `_unknown`.
pub fn sanitize_session_id(session_id: &str) -> String {
    let raw = session_id.trim();
    if raw.is_empty() {
        return "_unknown".to_string();
    }
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out.chars().all(|c| c == '_' || c == '.') {
        "_unknown".to_string()
    } else {
        out
    }
}

pub fn session_diag_dir(data_root: &Path, session_id: &str) -> PathBuf {
    diagnostics_root(data_root).join(sanitize_session_id(session_id))
}

fn process_diag_dir(data_root: &Path) -> PathBuf {
    diagnostics_root(data_root).join("_process")
}

fn ensure_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn truncate_err(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

// ---------------------------------------------------------------------------
// last-turn.json
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LastTurnSnapshot {
    pub session_id: String,
    pub turn_id: String,
    pub step: u64,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_rel: Option<String>,
    pub updated_unix_ms: u64,
}

/// Overwrite `diagnostics/<sid>/last-turn.json`. Failures warn only.
pub fn write_last_turn(data_root: &Path, session_id: &str, snap: &LastTurnSnapshot) {
    let dir = session_diag_dir(data_root, session_id);
    if let Err(e) = ensure_dir(&dir) {
        tracing::warn!(error = %e, "diagnostics: mkdir for last-turn failed");
        return;
    }
    let path = dir.join("last-turn.json");
    match serde_json::to_vec_pretty(snap) {
        Ok(bytes) => {
            if let Err(e) = fs::write(&path, bytes) {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "diagnostics: write last-turn.json failed"
                );
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "diagnostics: serialize last-turn failed");
        }
    }
}

// ---------------------------------------------------------------------------
// tool-audit.jsonl
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEvent {
    pub ts_unix_ms: u64,
    pub phase: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u64>,
    pub tool: String,
    pub call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_outcome: Option<String>,
}

impl AuditEvent {
    pub fn new(
        phase: impl Into<String>,
        session_id: impl Into<String>,
        tool: impl Into<String>,
        call_id: impl Into<String>,
    ) -> Self {
        let tls = DiagContext::snapshot();
        Self {
            ts_unix_ms: unix_ms(),
            phase: phase.into(),
            session_id: session_id.into(),
            turn_id: tls.as_ref().map(|s| s.turn_id.clone()),
            step: tls.as_ref().map(|s| s.step),
            tool: tool.into(),
            call_id: call_id.into(),
            rule_id: None,
            ok: None,
            error_kind: None,
            error: None,
            auth_outcome: None,
        }
    }
}

/// Append one JSONL line. On failure: warn only — never affects tool results.
pub fn append_tool_audit(data_root: &Path, session_id: &str, event: &AuditEvent) {
    tracing::info!(
        target: TOOL_AUDIT_TARGET,
        phase = %event.phase,
        session_id = %event.session_id,
        turn_id = event.turn_id.as_deref().unwrap_or(""),
        step = event.step.unwrap_or(0),
        tool = %event.tool,
        call_id = %event.call_id,
        ok = ?event.ok,
        error_kind = event.error_kind.as_deref().unwrap_or(""),
        auth_outcome = event.auth_outcome.as_deref().unwrap_or(""),
        "tool_audit"
    );

    let dir = session_diag_dir(data_root, session_id);
    if let Err(e) = ensure_dir(&dir) {
        tracing::warn!(error = %e, "diagnostics: mkdir for tool-audit failed");
        return;
    }
    let path = dir.join("tool-audit.jsonl");
    let line = match serde_json::to_string(event) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "diagnostics: serialize audit event failed");
            return;
        }
    };
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, "{line}") {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "diagnostics: append tool-audit.jsonl failed"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "diagnostics: open tool-audit.jsonl failed"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// wire-note.txt
// ---------------------------------------------------------------------------

/// Resolve `$LITECODE_LLM_WIRE/<sanitized>/` if env is set.
pub fn wire_paths(session_id: &str) -> Option<(PathBuf, String)> {
    let raw = std::env::var("LITECODE_LLM_WIRE").ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let base = PathBuf::from(raw);
    let folder = sanitize_session_id(session_id);
    let abs = base.join(&folder);
    let rel = format!("{folder}/");
    Some((abs, rel))
}

/// Write plain-text wire pointer (no symlink). Failures warn only.
pub fn note_wire_pointer(data_root: &Path, session_id: &str) {
    let dir = session_diag_dir(data_root, session_id);
    if let Err(e) = ensure_dir(&dir) {
        tracing::warn!(error = %e, "diagnostics: mkdir for wire-note failed");
        return;
    }
    let path = dir.join("wire-note.txt");
    let body = match std::env::var("LITECODE_LLM_WIRE") {
        Ok(base) if !base.trim().is_empty() => {
            let folder = sanitize_session_id(session_id);
            let base = base.trim();
            let absolute = PathBuf::from(base).join(&folder);
            format!(
                "LITECODE_LLM_WIRE={base}\nsession_folder={folder}\nabsolute={}\n# copy summary.jsonl + latest *.response.sse.jsonl when filing a bug\n",
                absolute.display()
            )
        }
        _ => "wire: disabled\n".to_string(),
    };
    if let Err(e) = fs::write(&path, body) {
        tracing::warn!(
            path = %path.display(),
            error = %e,
            "diagnostics: write wire-note.txt failed"
        );
    }
}

// ---------------------------------------------------------------------------
// DiagContext TLS
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DiagSnapshot {
    pub data_root: PathBuf,
    pub session_id: String,
    pub turn_id: String,
    pub step: u64,
    pub last_tool: Option<String>,
    pub last_call_id: Option<String>,
}

struct DiagState {
    data_root: PathBuf,
    session_id: String,
    turn_id: String,
    step: u64,
    last_tool: Option<String>,
    last_call_id: Option<String>,
}

thread_local! {
    static DIAG: RefCell<Option<DiagState>> = const { RefCell::new(None) };
}

/// Process-wide fallback data_root for panic hook when TLS has no session.
static FALLBACK_DATA_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

pub struct DiagContext;

/// RAII guard: clears TLS on drop.
pub struct DiagGuard {
    _private: (),
}

impl Drop for DiagGuard {
    fn drop(&mut self) {
        DIAG.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

impl DiagContext {
    /// Enter turn diagnostics scope (call at `run_with_turn` entry).
    pub fn enter(
        data_root: PathBuf,
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
    ) -> DiagGuard {
        let session_id = session_id.into();
        let turn_id = turn_id.into();
        DIAG.with(|cell| {
            *cell.borrow_mut() = Some(DiagState {
                data_root,
                session_id,
                turn_id,
                step: 0,
                last_tool: None,
                last_call_id: None,
            });
        });
        DiagGuard { _private: () }
    }

    pub fn update_step(step: u64) {
        DIAG.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.step = step;
            }
        });
    }

    pub fn update_last_tool(tool: &str, call_id: &str) {
        DIAG.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.last_tool = Some(tool.to_string());
                state.last_call_id = Some(call_id.to_string());
            }
        });
    }

    pub fn snapshot() -> Option<DiagSnapshot> {
        DIAG.with(|cell| {
            cell.borrow().as_ref().map(|s| DiagSnapshot {
                data_root: s.data_root.clone(),
                session_id: s.session_id.clone(),
                turn_id: s.turn_id.clone(),
                step: s.step,
                last_tool: s.last_tool.clone(),
                last_call_id: s.last_call_id.clone(),
            })
        })
    }
}

// ---------------------------------------------------------------------------
// Panic hook + crash log
// ---------------------------------------------------------------------------

/// Install a chaining panic hook. Prefer TLS session dir; else `diagnostics/_process/`.
pub fn install_panic_hook(fallback_data_root: PathBuf) {
    if let Ok(mut slot) = FALLBACK_DATA_ROOT.lock() {
        *slot = Some(fallback_data_root);
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());
        let bt = std::backtrace::Backtrace::force_capture();
        let backtrace = format!("{bt}");
        let ctx = DiagContext::snapshot();
        let data_root = ctx
            .as_ref()
            .map(|c| c.data_root.clone())
            .or_else(|| FALLBACK_DATA_ROOT.lock().ok().and_then(|g| g.clone()));
        if let Some(root) = data_root {
            let sid = ctx.as_ref().map(|c| c.session_id.as_str());
            let _ = write_crash_log(
                &root,
                sid,
                &payload,
                &backtrace,
                Some(&location),
                ctx.as_ref(),
            );
        }
        previous(info);
    }));
}

/// Write `crash-<unix_secs>.log`. Testable without actually panicking the process.
pub fn write_crash_log(
    data_root: &Path,
    session_id: Option<&str>,
    payload: &str,
    backtrace: &str,
    location: Option<&str>,
    ctx: Option<&DiagSnapshot>,
) -> std::io::Result<PathBuf> {
    let dir = match session_id {
        Some(sid) if !sid.trim().is_empty() => session_diag_dir(data_root, sid),
        _ => process_diag_dir(data_root),
    };
    ensure_dir(&dir)?;
    let path = dir.join(format!("crash-{}.log", unix_secs()));
    let mut body = String::new();
    body.push_str(&format!("panic: {payload}\n"));
    if let Some(loc) = location {
        body.push_str(&format!("location: {loc}\n"));
    }
    if let Some(c) = ctx {
        body.push_str(&format!("session_id: {}\n", c.session_id));
        body.push_str(&format!("turn_id: {}\n", c.turn_id));
        body.push_str(&format!("step: {}\n", c.step));
        if let Some(t) = &c.last_tool {
            body.push_str(&format!("last_tool: {t}\n"));
        }
        if let Some(id) = &c.last_call_id {
            body.push_str(&format!("last_call_id: {id}\n"));
        }
    }
    body.push_str("\nbacktrace:\n");
    body.push_str(backtrace);
    body.push('\n');
    fs::write(&path, body)?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Helpers used by runtime / tool call sites
// ---------------------------------------------------------------------------

/// Human-readable phase label for last-turn.json (snake-ish; unknown → "unknown").
pub fn turn_phase_label(phase: &crate::runtime::observer::TurnPhase) -> String {
    use crate::runtime::observer::TurnPhase;
    match phase {
        TurnPhase::Idle => "idle".into(),
        TurnPhase::Starting => "starting".into(),
        TurnPhase::Compacting => "compacting".into(),
        TurnPhase::CallingLlm => "calling_llm".into(),
        TurnPhase::Streaming => "streaming".into(),
        TurnPhase::ExecutingTools => "executing_tools".into(),
        TurnPhase::AwaitingPermission { .. } => "awaiting_permission".into(),
        TurnPhase::Cancelling => "cancelling".into(),
        TurnPhase::Finalizing => "finalizing".into(),
        TurnPhase::Failed { .. } => "failed".into(),
    }
}

/// Best-effort last-turn heartbeat from runtime (observe-only).
pub fn heartbeat_last_turn(
    data_root: &Path,
    session_id: &str,
    turn_id: &str,
    step: u64,
    phase: &str,
    last_error: Option<&str>,
) {
    let tls = DiagContext::snapshot();
    let wire_rel = wire_paths(session_id).map(|(_, rel)| format!("wire/{rel}"));
    let snap = LastTurnSnapshot {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        step,
        phase: phase.to_string(),
        last_tool: tls.as_ref().and_then(|s| s.last_tool.clone()),
        last_call_id: tls.as_ref().and_then(|s| s.last_call_id.clone()),
        last_error: last_error.map(|e| truncate_err(e, 200)),
        wire_rel,
        updated_unix_ms: unix_ms(),
    };
    write_last_turn(data_root, session_id, &snap);
}

/// Emit a tool-audit event + refresh last_tool TLS / last-turn on start.
pub fn emit_tool_audit(data_root: &Path, event: &AuditEvent) {
    if event.phase == "start" {
        DiagContext::update_last_tool(&event.tool, &event.call_id);
        let tls = DiagContext::snapshot();
        let turn_id = event
            .turn_id
            .clone()
            .or_else(|| tls.as_ref().map(|s| s.turn_id.clone()))
            .unwrap_or_default();
        let step = event
            .step
            .or_else(|| tls.as_ref().map(|s| s.step))
            .unwrap_or(0);
        heartbeat_last_turn(
            data_root,
            &event.session_id,
            &turn_id,
            step,
            "executing_tools",
            None,
        );
    }
    append_tool_audit(data_root, &event.session_id, event);
}

/// Log wire capture path for TurnError / GiveUp (observe-only).
pub fn log_turn_error_wire(session_id: &str, turn_id: &str, message: &str) {
    match wire_paths(session_id) {
        Some((abs, rel)) => {
            tracing::error!(
                session_id = %session_id,
                turn_id = %turn_id,
                wire_dir = %abs.display(),
                wire_rel = %rel,
                error = %message,
                "turn error; wire capture path"
            );
        }
        None => {
            tracing::error!(
                session_id = %session_id,
                turn_id = %turn_id,
                wire_dir = ?Option::<&str>::None,
                wire_rel = ?Option::<&str>::None,
                error = %message,
                "turn error; wire capture disabled (LITECODE_LLM_WIRE unset)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_diag_dir_sanitizes() {
        let root = PathBuf::from("/tmp/lc-data");
        assert_eq!(session_diag_dir(&root, "a/b").file_name().unwrap(), "a_b");
        assert_eq!(session_diag_dir(&root, "").file_name().unwrap(), "_unknown");
        assert_eq!(
            session_diag_dir(&root, "   ").file_name().unwrap(),
            "_unknown"
        );
        assert_eq!(sanitize_session_id("ok-id_1"), "ok-id_1");
        assert_eq!(sanitize_session_id("x.y"), "x_y");
    }

    #[test]
    fn append_tool_audit_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let data_root = dir.path();
        let sid = "sess-audit";
        let e1 = AuditEvent {
            ts_unix_ms: 1,
            phase: "start".into(),
            session_id: sid.into(),
            turn_id: Some("t1".into()),
            step: Some(1),
            tool: "grep".into(),
            call_id: "c1".into(),
            rule_id: None,
            ok: None,
            error_kind: None,
            error: None,
            auth_outcome: None,
        };
        let e2 = AuditEvent {
            ts_unix_ms: 2,
            phase: "exec_done".into(),
            session_id: sid.into(),
            turn_id: Some("t1".into()),
            step: Some(1),
            tool: "grep".into(),
            call_id: "c1".into(),
            rule_id: None,
            ok: Some(true),
            error_kind: None,
            error: None,
            auth_outcome: None,
        };
        append_tool_audit(data_root, sid, &e1);
        append_tool_audit(data_root, sid, &e2);
        let path = session_diag_dir(data_root, sid).join("tool-audit.jsonl");
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 2);
        let back1: AuditEvent = serde_json::from_str(lines[0]).unwrap();
        let back2: AuditEvent = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(back1.phase, "start");
        assert_eq!(back1.tool, "grep");
        assert_eq!(back2.phase, "exec_done");
        assert_eq!(back2.ok, Some(true));
    }

    #[test]
    fn write_last_turn_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let data_root = dir.path();
        let sid = "sess-lt";
        let first = LastTurnSnapshot {
            session_id: sid.into(),
            turn_id: "t1".into(),
            step: 1,
            phase: "starting".into(),
            last_tool: None,
            last_call_id: None,
            last_error: None,
            wire_rel: None,
            updated_unix_ms: 10,
        };
        let second = LastTurnSnapshot {
            session_id: sid.into(),
            turn_id: "t1".into(),
            step: 2,
            phase: "executing_tools".into(),
            last_tool: Some("grep".into()),
            last_call_id: Some("c1".into()),
            last_error: None,
            wire_rel: None,
            updated_unix_ms: 20,
        };
        write_last_turn(data_root, sid, &first);
        write_last_turn(data_root, sid, &second);
        let path = session_diag_dir(data_root, sid).join("last-turn.json");
        let got: LastTurnSnapshot = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(got.step, 2);
        assert_eq!(got.phase, "executing_tools");
        assert_eq!(got.last_tool.as_deref(), Some("grep"));
        assert_eq!(got.updated_unix_ms, 20);
    }

    #[test]
    fn write_crash_log_content() {
        let dir = tempfile::tempdir().unwrap();
        let data_root = dir.path();
        let ctx = DiagSnapshot {
            data_root: data_root.to_path_buf(),
            session_id: "sess-crash".into(),
            turn_id: "turn-9".into(),
            step: 3,
            last_tool: Some("grep".into()),
            last_call_id: Some("call-1".into()),
        };
        let path = write_crash_log(
            data_root,
            Some("sess-crash"),
            "boom payload",
            "STACK_LINE_HERE",
            Some("file.rs:1:1"),
            Some(&ctx),
        )
        .unwrap();
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("crash-"), "{name}");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("boom payload"));
        assert!(text.contains("STACK_LINE_HERE"));
        assert!(text.contains("session_id: sess-crash"));
        assert!(text.contains("last_tool: grep"));
        assert!(text.contains("turn_id: turn-9"));
    }
}
