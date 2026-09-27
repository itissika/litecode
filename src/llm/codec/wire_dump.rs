//! Dev-only raw capture of the LLM wire (Chat Completions and Responses).
//!
//! Enabled by `LITECODE_LLM_WIRE=<base>`; a no-op when unset, so the product
//! path never touches the filesystem. One session is one folder,
//! `<base>/<session_id>/`:
//!
//! - `summary.jsonl` — one JSON object per request. This is the file to scan,
//!   count, and tail. Every key below is always present (`null` when that
//!   fact did not occur).
//! - `meta.json` — session id and the retention caps, rewritten on each request.
//! - `<n>.<codec>.request.json` — exact body sent, plus `n`, `codec`, `url`,
//!   `session_id`, and `started_unix_ms`.
//! - `<n>.<codec>.response.sse.jsonl` — one JSON object per raw SSE line the
//!   reader delivered: `n`, `codec`, `ms` (milliseconds since that request's
//!   send), `line` (the line, unchanged).
//!
//! `summary.jsonl` fields: `n`, `codec`, `session_id`, `url`, `started_unix_ms`,
//! `ended_unix_ms`, `elapsed_ms`, `request_file`, `response_file`, `model`,
//! `sent`, `received`, `line`. `sent` counts the body (items, tool names,
//! reasoning replay, system-reminder occurrences). `received` counts the
//! stream, keeps the raw `usage` object, and lifts `tokens.input` /
//! `tokens.output` / `tokens.cached` / `tokens.reasoning` / `tokens.total`.
//! Cached tokens use the same field order as `chat_usage_to_responses`, but a
//! missing field stays `null` rather than becoming 0. The raw `usage` object
//! is still there when a sibling field disagrees.
//!
//! Retention is wide and only drops whole request pairs, oldest first:
//! [`KEEP_REQUESTS`] pairs per session, [`KEEP_SESSIONS`] session folders under
//! the base. `summary.jsonl` and `meta.json` stay. The session being written
//! is never removed. A call with no session id lands in `_unknown`.
//!
//! Unit tests are excluded even when the variable is set, so a `cargo test`
//! run cannot pad the folder a real session is being debugged in.
//!
//! The raw stream is a diagnosis channel only. What a client renders is the
//! session row the projection writes, so a dump explains a row rather than being
//! a second copy of it.
//!
//! `scripts/wire_watch.ps1` tails `summary.jsonl`. `serve_win.ps1 -Wire` /
//! `serve.sh --wire` turn capture on.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// Newest request pairs kept in one session folder.
const KEEP_REQUESTS: u64 = 400;
/// Newest session folders kept under the capture base.
const KEEP_SESSIONS: usize = 30;

/// Capture base, or `None` when disabled — `None` short-circuits all work.
fn base_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let raw = std::env::var("LITECODE_LLM_WIRE").ok()?;
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let dir = PathBuf::from(raw);
        match fs::create_dir_all(&dir) {
            Ok(()) => {
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

fn wire_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Folder name for a session. Characters that are not a path segment become `_`.
fn session_dir_name(session_id: Option<&str>) -> String {
    let raw = session_id.unwrap_or("").trim();
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

fn request_file_name(n: u64, codec: &str) -> String {
    format!("{n:04}.{codec}.request.json")
}

fn response_file_name(n: u64, codec: &str) -> String {
    format!("{n:04}.{codec}.response.sse.jsonl")
}

/// Leading request number on a raw capture file, if the name is one of ours.
fn leading_n(name: &str) -> Option<u64> {
    let (num, rest) = name.split_once('.')?;
    if !rest.ends_with(".request.json") && !rest.ends_with(".response.sse.jsonl") {
        return None;
    }
    if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    num.parse().ok()
}

fn next_n(dir: &Path) -> u64 {
    let mut max = 0u64;
    let Ok(entries) = fs::read_dir(dir) else {
        return 1;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if let Some(n) = leading_n(name) {
            max = max.max(n);
        }
    }
    max.saturating_add(1)
}

/// Drop the oldest request pairs so at most `keep` remain, ending at `newest`.
fn gc_request_pairs(dir: &Path, newest: u64, keep: u64) {
    if newest <= keep {
        return;
    }
    let drop_through = newest - keep;
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(n) = leading_n(name) else {
            continue;
        };
        if n <= drop_through {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn session_recency(dir: &Path) -> u64 {
    let meta_path = dir.join("meta.json");
    if let Ok(text) = fs::read_to_string(&meta_path)
        && let Ok(value) = serde_json::from_str::<Value>(&text)
        && let Some(ms) = value.get("updated_unix_ms").and_then(Value::as_u64)
    {
        return ms;
    }
    fs::metadata(dir)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Drop the oldest session folders beyond `keep`. `spare` is the folder name
/// currently being written and is left in place.
fn gc_session_dirs(base: &Path, spare: &str, keep: usize) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let recency = session_recency(&entry.path());
        dirs.push((recency, name, entry.path()));
    }
    if dirs.len() <= keep {
        return;
    }
    dirs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut overflow = dirs.len() - keep;
    for (_recency, name, path) in dirs {
        if overflow == 0 {
            break;
        }
        if name == spare {
            continue;
        }
        if fs::remove_dir_all(&path).is_ok() {
            overflow -= 1;
        }
    }
}

fn write_session_meta(dir: &Path, session_id: &str) {
    let path = dir.join("meta.json");
    let now = unix_ms();
    let created = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.get("created_unix_ms").and_then(Value::as_u64))
        .unwrap_or(now);
    let meta = json!({
        "session_id": session_id,
        "created_unix_ms": created,
        "updated_unix_ms": now,
        "keep_requests": KEEP_REQUESTS,
        "keep_sessions": KEEP_SESSIONS,
    });
    let _ = fs::write(path, serde_json::to_vec_pretty(&meta).unwrap_or_default());
}

/// One request plus the stream it produced.
pub(super) struct Capture {
    sse_path: PathBuf,
    summary_path: PathBuf,
    started: Instant,
    started_unix_ms: u64,
    n: u64,
    codec: String,
    session_id: Option<String>,
    url: String,
    request_file: String,
    response_file: String,
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
    response_id: Option<String>,
    response_model: Option<String>,
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
        let base = base_dir()?;
        let folder = session_dir_name(session_id);
        let dir = base.join(&folder);
        if let Err(error) = fs::create_dir_all(&dir) {
            tracing::warn!(
                path = %dir.display(),
                %error,
                "llm wire capture: session dir failed"
            );
            return None;
        }
        let _guard = wire_lock();
        let n = next_n(&dir);
        let request_file = request_file_name(n, codec);
        let response_file = response_file_name(n, codec);
        let started_unix_ms = unix_ms();
        let payload = serde_json::json!({
            "n": n,
            "codec": codec,
            "session_id": session_id,
            "url": url,
            "started_unix_ms": started_unix_ms,
            "body": body,
        });
        let bytes = serde_json::to_vec_pretty(&payload).unwrap_or_default();
        let request_path = dir.join(&request_file);
        if let Err(error) = fs::write(&request_path, bytes) {
            tracing::warn!(
                path = %request_path.display(),
                %error,
                "llm wire capture: request dump failed"
            );
            return None;
        }
        write_session_meta(&dir, session_id.unwrap_or("_unknown"));
        gc_request_pairs(&dir, n, KEEP_REQUESTS);
        gc_session_dirs(base, &folder, KEEP_SESSIONS);
        Some(Self {
            sse_path: dir.join(&response_file),
            summary_path: dir.join("summary.jsonl"),
            started: Instant::now(),
            started_unix_ms,
            n,
            codec: codec.to_string(),
            session_id: session_id.map(str::to_string),
            url: url.to_string(),
            request_file,
            response_file,
            sent: sent_stats(codec, body),
            stream: Mutex::new(StreamStats::default()),
        })
    }

    /// Append one raw stream line, exactly as the SSE reader delivered it.
    /// `ms` is milliseconds since this request's send, so a slow trickle stays
    /// distinguishable from a burst that lands in one read.
    pub(super) fn line(&self, line: &str) {
        let ms = self.started.elapsed().as_secs_f64() * 1000.0;
        let ms = (ms * 1000.0).round() / 1000.0;
        let record = json!({
            "n": self.n,
            "codec": self.codec,
            "ms": ms,
            "line": line,
        });
        append(&self.sse_path, &record.to_string());
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
        let Ok(stats) = self.stream.lock() else {
            return;
        };
        let received = stats.summary(elapsed_ms);
        let ended_unix_ms = self
            .started_unix_ms
            .saturating_add(elapsed_ms.round() as u64);
        let line = human_line(self.n, &self.codec, &self.sent, &received);
        let record = json!({
            "n": self.n,
            "codec": self.codec,
            "session_id": self.session_id,
            "url": self.url,
            "started_unix_ms": self.started_unix_ms,
            "ended_unix_ms": ended_unix_ms,
            "elapsed_ms": elapsed_ms.round() as u64,
            "request_file": self.request_file,
            "response_file": self.response_file,
            "model": self.sent.get("model").cloned().unwrap_or(Value::Null),
            "sent": self.sent,
            "received": received,
            "line": line,
        });
        let _guard = wire_lock();
        append(&self.summary_path, &record.to_string());
        tracing::info!(
            target: "litecode::wire",
            n = self.n,
            session_id = self.session_id.as_deref().unwrap_or_default(),
            "{line}"
        );
    }
}

impl StreamStats {
    fn fold(&mut self, line: &str, ms: f64) {
        self.lines += 1;
        self.last_ms = ms;
        // `sse_data_payload` hides the `[DONE]` sentinel; it is a terminal here.
        if line
            .trim_end_matches('\r')
            .strip_prefix("data:")
            .map(str::trim)
            == Some("[DONE]")
        {
            self.first_ms.get_or_insert(ms);
            self.terminal = Some("[DONE]".into());
            return;
        }
        let Some(data) = super::sse::sse_data_payload(line) else {
            return;
        };
        let data = data.trim();
        if data.is_empty() {
            return;
        }
        self.first_ms.get_or_insert(ms);
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            *self.events.entry("unparsed".into()).or_default() += 1;
            return;
        };
        note_response_identity(self, &value);
        if let Some(kind) = value.get("type").and_then(Value::as_str) {
            // Responses dialect: the event type is the unit.
            *self.events.entry(kind.to_string()).or_default() += 1;
            let delta_len = || {
                value
                    .get("delta")
                    .and_then(Value::as_str)
                    .map_or(0, |d| d.chars().count() as u64)
            };
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
        let Some(choice) = value.pointer("/choices/0") else {
            return;
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        let Some(delta) = choice.get("delta") else {
            return;
        };
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
            "tokens": tokens_of(self.usage.as_ref()),
            "response_id": self.response_id,
            "response_model": self.response_model,
            // No terminal event: the stream was cut, cancelled, or never opened.
            "terminal": self.terminal.as_deref().unwrap_or("none"),
        })
    }
}

fn note_response_identity(stats: &mut StreamStats, value: &Value) {
    if stats.response_id.is_none()
        && let Some(id) = value
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/response/id").and_then(Value::as_str))
            .filter(|id| !id.is_empty())
    {
        stats.response_id = Some(id.to_string());
    }
    if stats.response_model.is_none()
        && let Some(model) = value
            .get("model")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/response/model").and_then(Value::as_str))
            .filter(|model| !model.is_empty())
    {
        stats.response_model = Some(model.to_string());
    }
}

/// Lift the numbers a scan usually wants. Field order for `cached` matches
/// `chat_usage_to_responses`. A missing field stays null, including when a
/// later sibling would have been a number: the raw `usage` object is the
/// place to see that disagreement.
fn tokens_of(usage: Option<&Value>) -> Value {
    let Some(usage) = usage else {
        return json!({
            "input": null,
            "output": null,
            "cached": null,
            "reasoning": null,
            "total": null,
        });
    };
    let input =
        json_u64(usage.get("prompt_tokens")).or_else(|| json_u64(usage.get("input_tokens")));
    let output =
        json_u64(usage.get("completion_tokens")).or_else(|| json_u64(usage.get("output_tokens")));
    let cached = json_u64(usage.pointer("/prompt_tokens_details/cached_tokens"))
        .or_else(|| json_u64(usage.pointer("/input_tokens_details/cached_tokens")))
        .or_else(|| json_u64(usage.get("cached_tokens")))
        .or_else(|| json_u64(usage.get("prompt_cache_hit_tokens")));
    let reasoning = json_u64(usage.pointer("/completion_tokens_details/reasoning_tokens"))
        .or_else(|| json_u64(usage.pointer("/output_tokens_details/reasoning_tokens")));
    let total = json_u64(usage.get("total_tokens")).or_else(|| match (input, output) {
        (Some(input), Some(output)) => Some(input.saturating_add(output)),
        _ => None,
    });
    json!({
        "input": input,
        "output": output,
        "cached": cached,
        "reasoning": reasoning,
        "total": total,
    })
}

fn json_u64(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_i64().map(|n| n.max(0) as u64))
        .or_else(|| value.as_f64().map(|n| n.max(0.0) as u64))
}

/// Layered stats of the exact body sent. Reasoning replay is the headline: every
/// assistant turn either carried its reasoning or went out without it.
/// Both `assistant` and `reasoning` objects are always present so a scan does
/// not branch on codec to find a key.
fn sent_stats(codec: &str, body: &Value) -> Value {
    let rendered = body.to_string();
    let mut stats = json!({
        "model": body.get("model").cloned().unwrap_or(Value::Null),
        "body_bytes": rendered.len(),
        "temperature": body.get("temperature").cloned().unwrap_or(Value::Null),
        "max_output_tokens": body
            .get("max_output_tokens")
            .or_else(|| body.get("max_tokens"))
            .cloned()
            .unwrap_or(Value::Null),
        "stream": body.get("stream").cloned().unwrap_or(Value::Null),
        "tools": body.get("tools").and_then(Value::as_array).map_or(0, Vec::len),
        "tool_names": tool_names(body),
        "effort": body.get("reasoning_effort").or_else(|| body.pointer("/reasoning/effort")).cloned().unwrap_or(Value::Null),
        "system_reminders": count_needle(&rendered, "<system-reminder>"),
        "items": json!({}),
        "assistant": empty_assistant(),
        "reasoning": empty_reasoning(),
        "ids_on_wire": 0,
        "include": body.get("include").cloned().unwrap_or(Value::Null),
    });
    let layered = if codec == "chat" {
        chat_sent(body)
    } else {
        responses_sent(body)
    };
    if let (Value::Object(target), Value::Object(extra)) = (&mut stats, layered) {
        target.extend(extra);
    }
    stats
}

fn empty_assistant() -> Value {
    json!({
        "turns": 0,
        "with_tool_calls": 0,
        "reasoning_filled": 0,
        "reasoning_empty": 0,
        "reasoning_absent": 0,
        "reasoning_chars": 0,
    })
}

fn empty_reasoning() -> Value {
    json!({
        "items": 0,
        "encrypted": 0,
        "with_text": 0,
        "with_summary": 0,
        "chars": 0,
    })
}

fn tool_names(body: &Value) -> Vec<String> {
    body.get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .or_else(|| tool.pointer("/function/name").and_then(Value::as_str))
                .map(str::to_string)
        })
        .collect()
}

fn count_needle(haystack: &str, needle: &str) -> u64 {
    if needle.is_empty() {
        return 0;
    }
    haystack.matches(needle).count() as u64
}

fn chat_sent(body: &Value) -> Value {
    let mut roles: BTreeMap<String, u64> = BTreeMap::new();
    let (mut assistant, mut with_tools, mut filled, mut empty, mut absent, mut chars) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    for message in body
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
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
    for item in body
        .get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind = item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message");
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
        if item
            .get("encrypted_content")
            .and_then(Value::as_str)
            .is_some_and(|c| !c.is_empty())
        {
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
        "include": body.get("include").cloned().unwrap_or(Value::Null),
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
    let model = sent.get("model").and_then(Value::as_str).unwrap_or("-");
    let token = |key: &str| {
        received
            .pointer(&format!("/tokens/{key}"))
            .and_then(Value::as_u64)
            .map_or_else(|| "-".to_string(), |n| n.to_string())
    };
    let mut line = format!(
        "#{n:04} {codec} {status} {model} | sent: {} items, {replay}, reminders {}, ids {}, {}B | recv: ttfb {}ms, {}ms, reasoning {} / content {} chars, tool deltas {}, terminal {} | tokens in {} cached {} out {}",
        sent.get("items")
            .and_then(Value::as_object)
            .map_or(0, |items| items
                .values()
                .filter_map(Value::as_u64)
                .sum::<u64>()),
        get(sent, "/system_reminders"),
        get(sent, "/ids_on_wire"),
        get(sent, "/body_bytes"),
        received
            .get("first_byte_ms")
            .and_then(Value::as_u64)
            .map_or_else(|| "-".into(), |ms| ms.to_string()),
        get(received, "/elapsed_ms"),
        get(received, "/reasoning_chars"),
        get(received, "/content_chars"),
        get(received, "/tool_call_deltas"),
        received
            .get("terminal")
            .and_then(Value::as_str)
            .unwrap_or("none"),
        token("input"),
        token("cached"),
        token("output"),
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
    let mut buf = Vec::with_capacity(line.len() + 1);
    buf.extend_from_slice(line.as_bytes());
    buf.push(b'\n');
    let _ = file.write_all(&buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_body_reports_filled_and_empty_reasoning_per_assistant_turn() {
        let body = json!({
            "model": "m",
            "temperature": 0.3,
            "max_tokens": 128,
            "stream": true,
            "tools": [{"type": "function", "function": {"name": "bash"}}],
            "messages": [
                {"role": "system", "content": "s"},
                {"role": "user", "content": "<system-reminder>\nEnvironment\n</system-reminder>"},
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
        assert_eq!(sent["reasoning"]["items"], 0);
        assert_eq!(sent["tool_names"][0], "bash");
        assert_eq!(sent["system_reminders"], 1);
        assert_eq!(sent["temperature"], 0.3);
        assert_eq!(sent["max_output_tokens"], 128);
        assert_eq!(sent["stream"], true);
    }

    #[test]
    fn responses_body_reports_ciphertext_text_and_ids() {
        let body = json!({
            "model": "m",
            "tools": [{"type": "function", "name": "read"}],
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
        assert_eq!(sent["assistant"]["turns"], 0);
        assert_eq!(sent["tool_names"][0], "read");
    }

    #[test]
    fn stream_fold_counts_both_dialects_and_the_terminal() {
        let mut chat = StreamStats::default();
        chat.fold(
            r#"data: {"id":"chatcmpl_1","model":"deepseek-v4.1-flash","choices":[{"delta":{"reasoning_content":"abc"}}]}"#,
            10.0,
        );
        chat.fold(r#"data: {"choices":[{"delta":{"content":"hi","tool_calls":[{"index":0}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":1,"prompt_tokens_details":{"cached_tokens":0},"prompt_cache_hit_tokens":4}}"#, 20.0);
        chat.fold("data: [DONE]", 30.0);
        let summary = chat.summary(31.0);
        assert_eq!(summary["reasoning_chars"], 3);
        assert_eq!(summary["content_chars"], 2);
        assert_eq!(summary["tool_call_deltas"], 1);
        assert_eq!(summary["first_byte_ms"], 10);
        assert_eq!(summary["terminal"], "[DONE]");
        assert_eq!(summary["usage"]["prompt_tokens"], 5);
        assert_eq!(summary["response_id"], "chatcmpl_1");
        assert_eq!(summary["response_model"], "deepseek-v4.1-flash");
        // Present zero wins over a later sibling, matching the codec. The raw
        // usage object still carries prompt_cache_hit_tokens.
        assert_eq!(summary["tokens"]["input"], 5);
        assert_eq!(summary["tokens"]["cached"], 0);
        assert_eq!(summary["tokens"]["output"], 1);
        assert_eq!(summary["usage"]["prompt_cache_hit_tokens"], 4);

        let mut responses = StreamStats::default();
        responses.fold(
            r#"data: {"type":"response.created","response":{"id":"resp_1","model":"mimo-v2.6-flash"}}"#,
            1.0,
        );
        responses.fold(
            r#"data: {"type":"response.reasoning_summary_text.delta","delta":"ab"}"#,
            5.0,
        );
        responses.fold(
            r#"data: {"type":"response.completed","response":{"id":"resp_1","usage":{"input_tokens":9,"output_tokens":2,"input_tokens_details":{"cached_tokens":7}}}}"#,
            9.0,
        );
        let summary = responses.summary(10.0);
        assert_eq!(summary["reasoning_chars"], 2);
        assert_eq!(summary["terminal"], "response.completed");
        assert_eq!(summary["events"]["response.completed"], 1);
        assert_eq!(summary["usage"]["input_tokens"], 9);
        assert_eq!(summary["tokens"]["input"], 9);
        assert_eq!(summary["tokens"]["cached"], 7);
        assert_eq!(summary["tokens"]["output"], 2);
        assert_eq!(summary["response_id"], "resp_1");
        assert_eq!(summary["response_model"], "mimo-v2.6-flash");
    }

    #[test]
    fn tokens_stay_null_when_the_usage_object_has_no_cache_field() {
        let tokens = tokens_of(Some(&json!({"prompt_tokens": 10})));
        assert_eq!(tokens["input"], 10);
        assert!(tokens["cached"].is_null());
        assert!(tokens["output"].is_null());
        assert!(tokens_of(None)["input"].is_null());
    }

    #[test]
    fn session_names_and_file_names_point_at_their_contents() {
        assert_eq!(
            session_dir_name(Some("01M3HJWDQF2KY41R6FHZPR8BV2")),
            "01M3HJWDQF2KY41R6FHZPR8BV2"
        );
        assert_eq!(session_dir_name(Some("a/b")), "a_b");
        assert_eq!(session_dir_name(None), "_unknown");
        assert_eq!(session_dir_name(Some("..")), "_unknown");
        assert_eq!(request_file_name(7, "chat"), "0007.chat.request.json");
        assert_eq!(
            response_file_name(7, "responses"),
            "0007.responses.response.sse.jsonl"
        );
        assert_eq!(leading_n("0007.chat.request.json"), Some(7));
        assert_eq!(leading_n("0007.chat.response.sse.jsonl"), Some(7));
        assert_eq!(leading_n("summary.jsonl"), None);
        assert_eq!(leading_n("meta.json"), None);
    }

    #[test]
    fn gc_drops_oldest_pairs_and_keeps_the_summary() {
        let dir = tempfile::tempdir().unwrap();
        for n in 1..=5 {
            fs::write(dir.path().join(request_file_name(n, "chat")), b"{}").unwrap();
            fs::write(dir.path().join(response_file_name(n, "chat")), b"").unwrap();
        }
        fs::write(dir.path().join("summary.jsonl"), b"{}\n").unwrap();
        fs::write(dir.path().join("meta.json"), b"{}").unwrap();
        assert_eq!(next_n(dir.path()), 6);
        gc_request_pairs(dir.path(), 5, 3);
        let mut names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "0003.chat.request.json",
                "0003.chat.response.sse.jsonl",
                "0004.chat.request.json",
                "0004.chat.response.sse.jsonl",
                "0005.chat.request.json",
                "0005.chat.response.sse.jsonl",
                "meta.json",
                "summary.jsonl",
            ]
        );
    }

    #[test]
    fn gc_sessions_keeps_the_newest_and_the_one_being_written() {
        let base = tempfile::tempdir().unwrap();
        for (name, updated) in [("old", 1), ("mid", 2), ("live", 3)] {
            let dir = base.path().join(name);
            fs::create_dir(&dir).unwrap();
            fs::write(
                dir.join("meta.json"),
                json!({"session_id": name, "updated_unix_ms": updated}).to_string(),
            )
            .unwrap();
        }
        gc_session_dirs(base.path(), "live", 2);
        let mut names: Vec<_> = fs::read_dir(base.path())
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        assert_eq!(names, vec!["live", "mid"]);
    }
}
