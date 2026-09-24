//! Dev-only raw capture of the LLM wire (Chat Completions and Responses).
//!
//! Enabled by `LITECODE_LLM_WIRE=<base>`; a no-op when unset, so the product
//! path never touches the filesystem. Each process writes one run folder,
//! `<base>/<utc-stamp>/`, holding per request `<nnn>-<codec>-request.json`
//! (URL + the exact body we sent), `<nnn>-<codec>-sse.jsonl` (every raw line as
//! it arrived, prefixed with its arrival offset as `<ms since that request's
//! send>\t`), and one `meta.json` pinning the run's wall-clock start. The offsets
//! are what separate a real stream from a burst: the line count alone cannot tell
//! a slow trickle from everything landing at once. Unit tests are excluded even
//! when the variable is set, so a `cargo test` run cannot pad the folder a real
//! session is being debugged in.
//!
//! Two different origins, so read the header before correlating: `meta.json`
//! stamps the run, while each `*-sse.jsonl` offset counts from that request's own
//! send. Within one file the offsets are self-consistent, which is what the
//! trickle-versus-burst question needs; converting an offset into a wall-clock
//! time means adding the file's own start, not the run's.
//!
//! This is the only place the upstream shape survives verbatim: the transcript
//! stores what the codec *synthesized* from the stream, which is exactly what
//! makes a vendor field switch (reasoning → content) indistinguishable from a
//! synthesis bug. The counter ascends in request order, so dumps line up with
//! the `LLM request built` lines in `.litecode/logs/litecode.log`.
//!
//! The raw stream is a diagnosis channel only. What a client renders is the
//! session row the projection writes, so a dump explains a row rather than being
//! a second copy of it.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Capture directory, or `None` when disabled — `None` short-circuits all work.
///
/// `LITECODE_LLM_WIRE` names a base directory; each process writes into its own
/// `<base>/<utc-stamp>` run folder, so a restart never appends to a previous
/// run's stream (the per-process counter starts at 1 again).
fn dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let raw = std::env::var("LITECODE_LLM_WIRE").ok()?;
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
        let dir = PathBuf::from(raw).join(stamp);
        match fs::create_dir_all(&dir) {
            Ok(()) => {
                write_meta(&dir);
                tracing::info!(dir = %dir.display(), "LLM wire capture enabled");
                Some(dir)
            }
            Err(error) => {
                tracing::warn!(
                    dir = %dir.display(),
                    %error,
                    "LITECODE_LLM_WIRE is not writable; capture off"
                );
                None
            }
        }
    })
    .as_ref()
}

/// Pin the run's wall-clock start. A `*-sse.jsonl` offset counts from its own
/// request's send, so this stamps the folder rather than any one stream.
fn write_meta(dir: &Path) {
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default();
    let meta = serde_json::json!({
        "run_started_unix_ms": unix_ms,
        "pid": std::process::id(),
    });
    let _ = fs::write(
        dir.join("meta.json"),
        serde_json::to_vec_pretty(&meta).unwrap_or_default(),
    );
}

/// One request plus the stream it produced.
pub(super) struct Capture {
    sse_path: PathBuf,
    started: Instant,
}

impl Capture {
    /// Dump the request body and open the SSE sink. `codec` names the protocol
    /// (`chat` / `responses`); `url` is the full request URL (no credentials);
    /// `session_id` ties the capture to a `.litecode/sessions.db` transcript.
    pub(super) fn start(
        codec: &str,
        url: &str,
        session_id: Option<&str>,
        body: &Value,
    ) -> Option<Self> {
        // Test streams are canned; capturing them would mix fixtures into the
        // folder an evidence trail is being read from.
        if cfg!(test) {
            return None;
        }
        let dir = dir()?;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let request_path = dir.join(format!("{n:04}-{codec}-request.json"));
        let payload = serde_json::json!({
            "url": url,
            "session_id": session_id,
            "body": body,
        });
        let bytes = serde_json::to_vec_pretty(&payload).unwrap_or_default();
        if let Err(error) = fs::write(&request_path, bytes) {
            tracing::warn!(
                path = %request_path.display(),
                %error,
                "llm wire capture: request dump failed"
            );
            return None;
        }
        Some(Self {
            sse_path: dir.join(format!("{n:04}-{codec}-sse.jsonl")),
            started: Instant::now(),
        })
    }

    /// Append one raw stream line, exactly as the SSE reader delivered it,
    /// prefixed with its arrival offset: `<ms since this request's send>\t<line>`.
    /// The offset is the whole point - it tells a slow trickle apart from a burst
    /// that lands in one read.
    pub(super) fn line(&self, line: &str) {
        let ms = self.started.elapsed().as_secs_f64() * 1000.0;
        append(&self.sse_path, &format!("{ms:.3}\t{line}"));
    }
}

/// Best-effort append: capture must never fail a turn.
fn append(path: &Path, line: &str) {
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = file.write_all(line.as_bytes());
    let _ = file.write_all(b"\n");
}
