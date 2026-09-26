//! portable-pty wrappers (always-on TerminalHub backend).

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};

use super::error::{TerminalError, TerminalResult};
use super::shell::ShellSpec;

pub(crate) const DEFAULT_COLS: u16 = 80;
pub(crate) const DEFAULT_ROWS: u16 = 24;

/// Env injected only into agent-driven PTY sessions (foreground exec + background
/// tasks) so TTY-interactive programs — git pager (`less`), git HTTPS credential
/// prompt — fail fast instead of blocking on a pseudo-terminal nobody types into.
/// Human interactive terminals pass `&[]` and keep normal pager/color behavior.
pub(crate) const AGENT_NON_INTERACTIVE_ENV: &[(&str, &str)] = &[
    ("GIT_PAGER", "cat"),
    ("PAGER", "cat"),
    ("GIT_TERMINAL_PROMPT", "0"),
];

/// Kill a child and its whole process group so no descendant outlives the call
/// (2.7). Falls back to plain `child.kill()` when the group kill is unavailable.
fn kill_process_group(child: &mut (dyn Child + Send + Sync)) -> TerminalResult<()> {
    let pid = child.process_id();
    // Kill the whole tree FIRST, while the parent still exists, then the direct
    // child. Killing the direct child first orphans its descendants (they keep
    // running and keep the PTY open), which made the timeout path block until the
    // orphan naturally exited — the 2.7 hang. Both steps are best-effort: the
    // tree kill is the primary mechanism, `child.kill()` a fallback.
    if let Some(pid) = pid {
        #[cfg(unix)]
        {
            let _ = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output();
        }
    }
    let _ = child.kill();
    Ok(())
}

/// Reply to DEC CPR / DSR cursor-position requests so an agent session
/// (no terminal emulator on the other end) does not block. Human interactive
/// sessions leave this off: xterm answers with the real cursor, and a fake
/// `1;1` reply races it and makes the shell redraw at the wrong place.
///
/// The reply shares the master writer with `PtySession::write`. It must never
/// block the reader thread behind a contended lock — a blocked write would
/// otherwise starve the reply *and* stall output processing (3.3). Try briefly,
/// then drop the reply and keep reading.
fn auto_reply_cpr(writer: &Mutex<Box<dyn Write + Send>>, chunk: &str) {
    if !chunk.contains("\u{1b}[6n") && !chunk.contains("\x1b[6n") {
        return;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    loop {
        match writer.try_lock() {
            Ok(mut w) => {
                let _ = w.write_all(b"\x1b[1;1R");
                let _ = w.flush();
                return;
            }
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(_) => return,
        }
    }
}

/// Longest valid UTF-8 prefix of `pending`. A trailing incomplete sequence
/// (at most 3 bytes, `error_len == None`) stays in `pending` for the next
/// read. A genuinely invalid byte (`error_len == Some`) is emitted as U+FFFD
/// so the reader cannot stall on it.
fn drain_utf8(pending: &mut Vec<u8>) -> String {
    let mut out = String::new();
    loop {
        if pending.is_empty() {
            break;
        }
        match std::str::from_utf8(pending) {
            Ok(text) => {
                out.push_str(text);
                pending.clear();
                break;
            }
            Err(err) => {
                let valid = err.valid_up_to();
                if valid > 0 {
                    out.push_str(std::str::from_utf8(&pending[..valid]).expect("valid utf-8"));
                }
                match err.error_len() {
                    None => {
                        pending.drain(..valid);
                        break;
                    }
                    Some(error_len) => {
                        out.push('\u{FFFD}');
                        pending.drain(..valid + error_len);
                    }
                }
            }
        }
    }
    out
}

/// EOF / read-error flush: remaining bytes are lossy so a torn sequence at
/// the end of the stream still reaches the consumer.
fn flush_utf8(pending: &mut Vec<u8>) -> String {
    let out = String::from_utf8_lossy(pending).into_owned();
    pending.clear();
    out
}

fn emit_chunk(
    chunk: &str,
    answer_cpr: bool,
    writer: &Mutex<Box<dyn Write + Send>>,
    buffer_output: bool,
    output: &Mutex<String>,
    on_data: &Option<Arc<dyn Fn(&str) + Send + Sync>>,
) {
    if chunk.is_empty() {
        return;
    }
    if answer_cpr {
        auto_reply_cpr(writer, chunk);
    }
    if buffer_output {
        output.lock().expect("output lock").push_str(chunk);
    }
    if let Some(cb) = on_data {
        cb(chunk);
    }
}

pub(crate) struct PtySession {
    pub id: String,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    output: Arc<Mutex<String>>,
    exit_code: Arc<Mutex<Option<u32>>>,
    alive: Arc<AtomicBool>,
    /// Agent background sessions: path of the tee'd output file (for `read`).
    pub output_path: Option<std::path::PathBuf>,
    /// Dropped on close to join the reader.
    _reader: Option<JoinHandle<()>>,
}

impl PtySession {
    pub fn spawn_interactive(
        id: String,
        shell: &ShellSpec,
        cwd: &Path,
        cols: u16,
        rows: u16,
        on_data: Option<Arc<dyn Fn(&str) + Send + Sync>>,
        on_exit: Option<Arc<dyn Fn(Option<u32>) + Send + Sync>>,
        env: &[(&str, &str)],
        answer_cpr: bool,
    ) -> TerminalResult<Self> {
        // 2.8 (REV-7): interactive sessions broadcast only and never buffer output
        // (`take_output` has no consumer for them), so memory stays bounded.
        spawn_inner(
            id, shell, cwd, cols, rows, on_data, on_exit, env, false, answer_cpr,
        )
    }

    pub fn write(&self, data: &[u8]) -> TerminalResult<()> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(TerminalError::Closed(self.id.clone()));
        }
        let mut w = self
            .writer
            .lock()
            .map_err(|_| TerminalError::Io("writer lock poisoned".into()))?;
        w.write_all(data)
            .map_err(|e| TerminalError::Io(e.to_string()))?;
        w.flush().map_err(|e| TerminalError::Io(e.to_string()))?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> TerminalResult<()> {
        let master = self
            .master
            .lock()
            .map_err(|_| TerminalError::Io("master lock poisoned".into()))?;
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| TerminalError::Io(e.to_string()))
    }

    pub fn take_output(&self) -> String {
        let mut buf = self.output.lock().expect("output lock");
        std::mem::take(&mut *buf)
    }

    pub fn exit_code(&self) -> Option<u32> {
        *self.exit_code.lock().expect("exit lock")
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn kill(&self) -> TerminalResult<()> {
        let mut child = self
            .child
            .lock()
            .map_err(|_| TerminalError::Io("child lock poisoned".into()))?;
        kill_process_group(child.as_mut())
    }

    pub fn try_reap(&self) {
        let mut child = match self.child.lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        if let Ok(Some(status)) = child.try_wait() {
            let code = status.exit_code();
            *self.exit_code.lock().expect("exit lock") = Some(code);
            self.alive.store(false, Ordering::SeqCst);
        }
    }

    fn wait_join(&mut self) {
        // Kill + reap so the pty master reaches EOF and the reader thread can exit.
        let _ = self.kill();
        for _ in 0..100 {
            self.try_reap();
            if !self.alive.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if let Some(handle) = self._reader.take() {
            // Bound the join: with the tree killed the reader exits promptly on
            // PTY EOF. A hard bound keeps every close path hang-free (2.7) even
            // if a descendant stubbornly holds the PTY open.
            for _ in 0..150 {
                if handle.is_finished() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                // Last resort: a pathological descendant still holds the PTY.
                // Do not block the caller; the reader exits on its own when the
                // PTY eventually closes.
                tracing::warn!(
                    "pty reader did not exit after close; detaching to avoid hang (2.7)"
                );
            }
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // 2.7: joining the reader prevents thread leaks on every close path
        // (interactive close, background kill, exec_once timeout).
        self.wait_join();
    }
}

fn spawn_inner(
    id: String,
    shell: &ShellSpec,
    cwd: &Path,
    cols: u16,
    rows: u16,
    on_data: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    on_exit: Option<Arc<dyn Fn(Option<u32>) + Send + Sync>>,
    env: &[(&str, &str)],
    buffer_output: bool,
    answer_cpr: bool,
) -> TerminalResult<PtySession> {
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| TerminalError::Spawn(e.to_string()))?;

    let mut cmd = CommandBuilder::new(&shell.program);
    for a in &shell.args {
        cmd.arg(a);
    }
    cmd.cwd(cwd);
    for (key, value) in env {
        cmd.env(key, value);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| TerminalError::Spawn(e.to_string()))?;

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| TerminalError::Io(e.to_string()))?;
    let writer = Arc::new(Mutex::new(
        pair.master
            .take_writer()
            .map_err(|e| TerminalError::Io(e.to_string()))?,
    ));

    let output = Arc::new(Mutex::new(String::new()));
    let exit_code = Arc::new(Mutex::new(None));
    let alive = Arc::new(AtomicBool::new(true));

    let output_r = Arc::clone(&output);
    let exit_r = Arc::clone(&exit_code);
    let alive_r = Arc::clone(&alive);
    let writer_r = Arc::clone(&writer);
    let id_r = id.clone();

    let reader = std::thread::Builder::new()
        .name(format!("pty-read-{id_r}"))
        .spawn(move || {
            let mut buf = [0u8; 4096];
            // Carry an incomplete UTF-8 sequence across read boundaries so a
            // multibyte character split by the 4096-byte read is not replaced
            // with U+FFFD on both sides.
            let mut pending: Vec<u8> = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        emit_chunk(
                            &flush_utf8(&mut pending),
                            answer_cpr,
                            &writer_r,
                            buffer_output,
                            &output_r,
                            &on_data,
                        );
                        break;
                    }
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        emit_chunk(
                            &drain_utf8(&mut pending),
                            answer_cpr,
                            &writer_r,
                            buffer_output,
                            &output_r,
                            &on_data,
                        );
                    }
                    Err(_) => {
                        emit_chunk(
                            &flush_utf8(&mut pending),
                            answer_cpr,
                            &writer_r,
                            buffer_output,
                            &output_r,
                            &on_data,
                        );
                        break;
                    }
                }
            }
            alive_r.store(false, Ordering::SeqCst);
            if let Some(ref cb) = on_exit {
                let code = *exit_r.lock().expect("exit lock");
                cb(code);
            }
        })
        .map_err(|e| TerminalError::Spawn(e.to_string()))?;

    Ok(PtySession {
        id,
        writer,
        master: Mutex::new(pair.master),
        child: Mutex::new(child),
        output,
        exit_code,
        alive,
        output_path: None,
        _reader: Some(reader),
    })
}

pub(crate) enum ExecFinish {
    Exited(Option<u32>),
    TimedOut,
    Cancelled,
}

/// One-shot: spawn shell -c / -Command, wait for exit (with timeout).
/// Output is teed through `sink` on the reader thread (no unbounded PTY buffer).
pub(crate) fn exec_once(
    shell: &ShellSpec,
    cwd: &Path,
    timeout: Duration,
    cancel: Option<&tokio_util::sync::CancellationToken>,
    sink: Arc<Mutex<super::tee::BoundedTee>>,
) -> TerminalResult<ExecFinish> {
    let id = "exec-once".to_string();
    let sink_data = Arc::clone(&sink);
    let on_data: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |chunk: &str| {
        if let Ok(mut tee) = sink_data.lock() {
            tee.push_raw(chunk);
        }
    });
    let session = spawn_inner(
        id,
        shell,
        cwd,
        DEFAULT_COLS,
        DEFAULT_ROWS,
        Some(on_data),
        None,
        AGENT_NON_INTERACTIVE_ENV,
        false,
        true,
    )?;
    let start = Instant::now();
    loop {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            let _ = session.kill();
            for _ in 0..50 {
                session.try_reap();
                if !session.is_alive() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            return Ok(ExecFinish::Cancelled);
        }
        session.try_reap();
        if !session.is_alive() {
            // The reader can hit EOF a hair before the child status is
            // collectable; wait briefly for the reap so callers never see a
            // spurious `None`/`-1` exit code for a successful command (3.3).
            for _ in 0..50 {
                session.try_reap();
                if session.exit_code().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            return Ok(ExecFinish::Exited(session.exit_code()));
        }
        if start.elapsed() > timeout {
            // 2.7: kill the process group and wait for it to be reaped so no
            // descendant or zombie survives the timeout. `session`'s Drop then
            // joins the reader thread.
            let _ = session.kill();
            for _ in 0..50 {
                session.try_reap();
                if !session.is_alive() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            return Ok(ExecFinish::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::{drain_utf8, flush_utf8};

    #[test]
    fn drain_utf8_keeps_a_cjk_character_split_across_chunks() {
        // "你" is E4 BD A0. The first read stops inside the sequence.
        let mut pending = vec![0xE4, 0xBD];
        assert_eq!(drain_utf8(&mut pending), "");
        assert_eq!(pending, vec![0xE4, 0xBD]);
        pending.push(0xA0);
        pending.extend_from_slice(b"ab");
        assert_eq!(drain_utf8(&mut pending), "你ab");
        assert!(pending.is_empty());
    }

    #[test]
    fn drain_utf8_replaces_an_invalid_byte_and_keeps_going() {
        let mut pending = b"a\xffb".to_vec();
        assert_eq!(drain_utf8(&mut pending), "a\u{FFFD}b");
        assert!(pending.is_empty());

        // A lead byte followed by a non-continuation must not stall the reader.
        let mut pending = vec![0xC0, b'A'];
        assert_eq!(drain_utf8(&mut pending), "\u{FFFD}A");
        assert!(pending.is_empty());
    }

    #[test]
    fn flush_utf8_emits_a_torn_tail_as_replacement() {
        let mut pending = vec![0xE4, 0xBD];
        assert_eq!(drain_utf8(&mut pending), "");
        assert_eq!(flush_utf8(&mut pending), "\u{FFFD}");
        assert!(pending.is_empty());
    }
}
