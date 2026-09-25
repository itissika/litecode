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
//!
//! Monitor: every request also appends one summary line to `<run>/index.jsonl`
//! and logs it (target `litecode::wire`). The summary is layered — what we sent
//! (per role/type, reasoning replay filled vs empty, ciphertext, ids on the wire)
//! and what came back (status, first byte, duration, event counts, reasoning /
//! content / tool deltas, usage, terminal). It is written when the capture drops,
//! so errors and cancellations are summarized too. `scripts/wire_watch.ps1`
//! tails it; `serve_win.ps1 -Wire` / `serve.sh --wire` turn capture on.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

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
    index_path: PathBuf,
    started: Instant,
    n: u64,
    codec: String,
    session_id: Option<String>,
    sent: Value,
    stream: Mutex<StreamStats>,
}

/// What came back, folded line by line.
#[derive(Default)]
struct StreamStats {
    status: Option<u16>,
    error_body: Option<String>,
    lines: u64,
    first_ms: Option<f64>,
    last_ms: f64,
    events: BTreeMap<String, u64>,
    reasoning_chars: u64,
    content_chars: u64,
    tool_call_deltas: u64,
    finish_reason: Option<String>,
    usage: Option<Value>,
    terminal: Option<String>,
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
            index_path: dir.join("index.jsonl"),
            started: Instant::now(),
            n,
            codec: codec.to_string(),
            session_id: session_id.map(str::to_string),
            sent: sent_stats(codec, body),
            stream: Mutex::new(StreamStats::default()),
        })
    }

    /// Append one raw stream line, exactly as the SSE reader delivered it,
    /// prefixed with its arrival offset: `<ms since this request's send>\t<line>`.
    /// The offset is the whole point - it tells a slow trickle apart from a burst
    /// that lands in one read.
    pub(super) fn line(&self, line: &str) {
        let ms = self.started.elapsed().as_secs_f64() * 1000.0;
        append(&self.sse_path, &format!("{ms:.3}\t{line}"));
        if let Ok(mut stats) = self.stream.lock() {
            stats.fold(line, ms);
        }
    }

    /// The HTTP status the endpoint answered with.
    pub(super) fn status(&self, status: u16) {
        if let Ok(mut stats) = self.stream.lock() {
            stats.status = Some(status);
        }
    }

    /// The body of a non-success answer (truncated): the vendor's own reason.
    pub(super) fn error_body(&self, text: &str) {
        if let Ok(mut stats) = self.stream.lock() {
            stats.error_body = Some(text.chars().take(600).collect());
        }
    }
}

impl Drop for Capture {
    /// One summary per request, whatever the outcome.
    fn drop(&mut self) {
        let elapsed_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        let Ok(stats) = self.stream.lock() else { return };
        let received = stats.summary(elapsed_ms);
        let line = human_line(self.n, &self.codec, &self.sent, &received);
        let record = json!({
            "n": self.n,
            "codec": self.codec,
            "session_id": self.session_id,
            "sent": self.sent,
            "received": received,
            "line": line,
        });
        append(&self.index_path, &record.to_string());
        tracing::info!(target: "litecode::wire", n = self.n, session_id = self.session_id.as_deref().unwrap_or_default(), "{line}");
    }
}

impl StreamStats {
    fn fold(&mut self, line: &str, ms: f64) {
        self.lines += 1;
        self.last_ms = ms;
        // `sse_data_payload` hides the `[DONE]` sentinel; it is a terminal here.
        if line.trim_end_matches('\r').strip_prefix("data:").map(str::trim) == Some("[DONE]") {
            self.first_ms.get_or_insert(ms);
            self.terminal = Some("[DONE]".into());
            return;
        }
        let Some(data) = super::sse::sse_data_payload(line) else { return };
        let data = data.trim();
        if data.is_empty() {
            return;
        }
        self.first_ms.get_or_insert(ms);
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            *self.events.entry("unparsed".into()).or_default() += 1;
            return;
        };
        if let Some(kind) = value.get("type").and_then(Value::as_str) {
            // Responses dialect: the event type is the unit.
            *self.events.entry(kind.to_string()).or_default() += 1;
            let delta_len = || value.get("delta").and_then(Value::as_str).map_or(0, |d| d.chars().count() as u64);
            match kind {
                "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                    self.reasoning_chars += delta_len()
                }
                "response.output_text.delta" => self.content_chars += delta_len(),
                "response.function_call_arguments.delta" => self.tool_call_deltas += 1,
                "response.completed" | "response.incomplete" | "response.failed" | "error" => {
                    self.terminal = Some(kind.to_string());
                    if let Some(usage) = value.pointer("/response/usage").filter(|u| !u.is_null()) {
                        self.usage = Some(usage.clone());
                    }
                }
                _ => {}
            }
            return;
        }
        // Chat Completions dialect: one chunk, fields inside `choices[0].delta`.
        *self.events.entry("chunk".into()).or_default() += 1;
        if let Some(usage) = value.get("usage").filter(|u| !u.is_null()) {
            self.usage = Some(usage.clone());
        }
        let Some(choice) = value.pointer("/choices/0") else { return };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        let Some(delta) = choice.get("delta") else { return };
        for key in ["reasoning_content", "reasoning"] {
            if let Some(text) = delta.get(key).and_then(Value::as_str) {
                self.reasoning_chars += text.chars().count() as u64;
            }
        }
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            self.content_chars += text.chars().count() as u64;
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            self.tool_call_deltas += calls.len() as u64;
        }
    }

    fn summary(&self, elapsed_ms: f64) -> Value {
        json!({
            "status": self.status,
            "error_body": self.error_body,
            "first_byte_ms": self.first_ms.map(|ms| ms.round() as u64),
            "last_line_ms": self.last_ms.round() as u64,
            "elapsed_ms": elapsed_ms.round() as u64,
            "lines": self.lines,
            "events": self.events,
            "reasoning_chars": self.reasoning_chars,
            "content_chars": self.content_chars,
            "tool_call_deltas": self.tool_call_deltas,
            "finish_reason": self.finish_reason,
            "usage": self.usage,
            // No terminal event: the stream was cut, cancelled, or never opened.
            "terminal": self.terminal.as_deref().unwrap_or("none"),
        })
    }
}

/// Layered stats of the exact body sent. Reasoning replay is the headline: every
/// assistant turn either carried its reasoning or went out without it.
fn sent_stats(codec: &str, body: &Value) -> Value {
    let base = json!({
        "model": body.get("model"),
        "body_bytes": body.to_string().len(),
        "tools": body.get("tools").and_then(Value::as_array).map_or(0, Vec::len),
        "effort": body.get("reasoning_effort").or_else(|| body.pointer("/reasoning/effort")),
    });
    let layered = if codec == "chat" { chat_sent(body) } else { responses_sent(body) };
    let mut merged = base;
    if let (Value::Object(target), Value::Object(extra)) = (&mut merged, layered) {
        target.extend(extra);
    }
    merged
}

fn chat_sent(body: &Value) -> Value {
    let mut roles: BTreeMap<String, u64> = BTreeMap::new();
    let (mut assistant, mut with_tools, mut filled, mut empty, mut absent, mut chars) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    for message in body.get("messages").and_then(Value::as_array).into_iter().flatten() {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("?");
        *roles.entry(role.to_string()).or_default() += 1;
        if role != "assistant" {
            continue;
        }
        assistant += 1;
        if message.get("tool_calls").is_some() {
            with_tools += 1;
        }
        let reasoning = ["reasoning_content", "reasoning"]
            .iter()
            .find_map(|key| message.get(*key).and_then(Value::as_str));
        match reasoning {
            Some(text) if !text.is_empty() => {
                filled += 1;
                chars += text.chars().count() as u64;
            }
            Some(_) => empty += 1,
            None => absent += 1,
        }
    }
    json!({
        "items": roles,
        "assistant": {
            "turns": assistant,
            "with_tool_calls": with_tools,
            "reasoning_filled": filled,
            "reasoning_empty": empty,
            "reasoning_absent": absent,
            "reasoning_chars": chars,
        },
        // Chat has no item ids; tool_calls[].id is the call_id pairing key.
        "ids_on_wire": 0,
    })
}

fn responses_sent(body: &Value) -> Value {
    let mut kinds: BTreeMap<String, u64> = BTreeMap::new();
    let (mut reasoning, mut encrypted, mut with_text, mut with_summary, mut ids) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut chars = 0u64;
    for item in body.get("input").and_then(Value::as_array).into_iter().flatten() {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("message");
        let key = match (kind, item.get("role").and_then(Value::as_str)) {
            ("message", Some(role)) => format!("message:{role}"),
            (kind, _) => kind.to_string(),
        };
        *kinds.entry(key).or_default() += 1;
        if item.get("id").is_some() {
            ids += 1;
        }
        if kind != "reasoning" {
            continue;
        }
        reasoning += 1;
        if item.get("encrypted_content").and_then(Value::as_str).is_some_and(|c| !c.is_empty()) {
            encrypted += 1;
        }
        let text_len = |key: &str| -> u64 {
            item.get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .map(|text| text.chars().count() as u64)
                .sum()
        };
        let content = text_len("content");
        let summary = text_len("summary");
        if content > 0 {
            with_text += 1;
        }
        if summary > 0 {
            with_summary += 1;
        }
        chars += content + summary;
    }
    json!({
        "items": kinds,
        "reasoning": {
            "items": reasoning,
            "encrypted": encrypted,
            "with_text": with_text,
            "with_summary": with_summary,
            "chars": chars,
        },
        "ids_on_wire": ids,
        "include": body.get("include"),
    })
}

/// The one line a human scans while a session runs.
fn human_line(n: u64, codec: &str, sent: &Value, received: &Value) -> String {
    let get = |value: &Value, path: &str| value.pointer(path).and_then(Value::as_u64).unwrap_or(0);
    let replay = if codec == "chat" {
        format!(
            "asst {} (reasoning filled {} / empty {} / absent {}, {} chars)",
            get(sent, "/assistant/turns"),
            get(sent, "/assistant/reasoning_filled"),
            get(sent, "/assistant/reasoning_empty"),
            get(sent, "/assistant/reasoning_absent"),
            get(sent, "/assistant/reasoning_chars"),
        )
    } else {
        format!(
            "reasoning {} (cipher {} / text {} / summary {}, {} chars)",
            get(sent, "/reasoning/items"),
            get(sent, "/reasoning/encrypted"),
            get(sent, "/reasoning/with_text"),
            get(sent, "/reasoning/with_summary"),
            get(sent, "/reasoning/chars"),
        )
    };
    let status = received
        .get("status")
        .and_then(Value::as_u64)
        .map_or_else(|| "---".to_string(), |s| s.to_string());
    let mut line = format!(
        "#{n:04} {codec} {status} | sent: {} items, {replay}, ids {}, {}B | recv: ttfb {}ms, {}ms, reasoning {} / content {} chars, tool deltas {}, terminal {}",
        sent.get("items")
            .and_then(Value::as_object)
            .map_or(0, |items| items.values().filter_map(Value::as_u64).sum::<u64>()),
        get(sent, "/ids_on_wire"),
        get(sent, "/body_bytes"),
        received.get("first_byte_ms").and_then(Value::as_u64).map_or_else(|| "-".into(), |ms| ms.to_string()),
        get(received, "/elapsed_ms"),
        get(received, "/reasoning_chars"),
        get(received, "/content_chars"),
        get(received, "/tool_call_deltas"),
        received.get("terminal").and_then(Value::as_str).unwrap_or("none"),
    );
    if let Some(body) = received.get("error_body").and_then(Value::as_str) {
        line.push_str(&format!(" | error: {body}"));
    }
    line
}

/// Best-effort append: capture must never fail a turn.
fn append(path: &Path, line: &str) {
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = file.write_all(line.as_bytes());
    let _ = file.write_all(b"\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_body_reports_filled_and_empty_reasoning_per_assistant_turn() {
        let body = json!({
            "model": "m",
            "messages": [
                {"role": "system", "content": "s"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": null, "reasoning_content": "think", "tool_calls": [{"id": "call_1"}]},
                {"role": "tool", "tool_call_id": "call_1", "content": "ok"},
                {"role": "assistant", "content": "done", "reasoning_content": ""}
            ]
        });
        let sent = sent_stats("chat", &body);
        assert_eq!(sent["assistant"]["turns"], 2);
        assert_eq!(sent["assistant"]["reasoning_filled"], 1);
        assert_eq!(sent["assistant"]["reasoning_empty"], 1);
        assert_eq!(sent["assistant"]["reasoning_chars"], 5);
        assert_eq!(sent["items"]["tool"], 1);
    }

    #[test]
    fn responses_body_reports_ciphertext_text_and_ids() {
        let body = json!({
            "model": "m",
            "input": [
                {"type": "message", "role": "user", "content": []},
                {"type": "reasoning", "summary": [{"type": "summary_text", "text": "plan"}], "encrypted_content": "gAAAA"},
                {"type": "reasoning", "id": "rs_1", "summary": [], "content": [{"type": "reasoning_text", "text": "raw"}]},
                {"type": "function_call", "call_id": "c"}
            ]
        });
        let sent = sent_stats("responses", &body);
        assert_eq!(sent["reasoning"]["items"], 2);
        assert_eq!(sent["reasoning"]["encrypted"], 1);
        assert_eq!(sent["reasoning"]["with_text"], 1);
        assert_eq!(sent["reasoning"]["with_summary"], 1);
        assert_eq!(sent["ids_on_wire"], 1);
        assert_eq!(sent["items"]["message:user"], 1);
    }

    #[test]
    fn stream_fold_counts_both_dialects_and_the_terminal() {
        let mut chat = StreamStats::default();
        chat.fold(r#"data: {"choices":[{"delta":{"reasoning_content":"abc"}}]}"#, 10.0);
        chat.fold(r#"data: {"choices":[{"delta":{"content":"hi","tool_calls":[{"index":0}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5}}"#, 20.0);
        chat.fold("data: [DONE]", 30.0);
        let summary = chat.summary(31.0);
        assert_eq!(summary["reasoning_chars"], 3);
        assert_eq!(summary["content_chars"], 2);
        assert_eq!(summary["tool_call_deltas"], 1);
        assert_eq!(summary["first_byte_ms"], 10);
        assert_eq!(summary["terminal"], "[DONE]");
        assert_eq!(summary["usage"]["prompt_tokens"], 5);

        let mut responses = StreamStats::default();
        responses.fold(r#"data: {"type":"response.reasoning_summary_text.delta","delta":"ab"}"#, 5.0);
        responses.fold(r#"data: {"type":"response.completed","response":{"usage":{"input_tokens":9}}}"#, 9.0);
        let summary = responses.summary(10.0);
        assert_eq!(summary["reasoning_chars"], 2);
        assert_eq!(summary["terminal"], "response.completed");
        assert_eq!(summary["events"]["response.completed"], 1);
        assert_eq!(summary["usage"]["input_tokens"], 9);
    }
}
