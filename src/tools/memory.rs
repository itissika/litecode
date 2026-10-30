//! Session-scoped working memory tools.
//!
//! These are code-surface tools: one `run_code` program can batch several
//! atomic calls without a second round trip. The durable document lives in
//! `sessions.memory_json`; the tool layer only enforces the small contract.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::session::manager::SessionManager;
use crate::session::memory::{
    MEMORY_MAX_BATCH, MEMORY_MAX_QUERY_CHARS, MemoryEntry, MemoryKind, MemoryReject,
    MemoryUpsertRequest,
};
use crate::tool::Tool;
use crate::tool::outcome::ok_data;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

pub const MEMORY_LIST_TOOL: &str = "memory.list";
pub const MEMORY_UPSERT_TOOL: &str = "memory.upsert";
pub const MEMORY_DELETE_TOOL: &str = "memory.delete";

const KIND_NAMES: &[&str] = &["fact", "preference", "decision", "constraint"];

#[derive(Clone)]
struct SessionHandle {
    sessions: Option<Arc<SessionManager>>,
}

impl SessionHandle {
    fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            sessions: Some(sessions),
        }
    }

    fn cold() -> Self {
        Self { sessions: None }
    }
}

/// List the current session's memory.
pub struct MemoryListTool {
    handle: SessionHandle,
}

impl MemoryListTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            handle: SessionHandle::new(sessions),
        }
    }

    pub fn cold() -> Self {
        Self {
            handle: SessionHandle::cold(),
        }
    }
}

/// Add or update entries in the current session's memory.
pub struct MemoryUpsertTool {
    handle: SessionHandle,
}

impl MemoryUpsertTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            handle: SessionHandle::new(sessions),
        }
    }

    pub fn cold() -> Self {
        Self {
            handle: SessionHandle::cold(),
        }
    }
}

/// Remove entries from the current session's memory.
pub struct MemoryDeleteTool {
    handle: SessionHandle,
}

impl MemoryDeleteTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            handle: SessionHandle::new(sessions),
        }
    }

    pub fn cold() -> Self {
        Self {
            handle: SessionHandle::cold(),
        }
    }
}

impl Tool for MemoryListTool {
    fn name(&self) -> &str {
        MEMORY_LIST_TOOL
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        let sessions = self.handle.sessions.clone();
        Box::pin(async move {
            match tokio::task::spawn_blocking(move || run_list(sessions, input, execution)).await {
                Ok(result) => result,
                Err(error) => worker_error("memory.list", error),
            }
        })
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        code_surface_only()
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(10))
    }
}

impl Tool for MemoryUpsertTool {
    fn name(&self) -> &str {
        MEMORY_UPSERT_TOOL
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        let sessions = self.handle.sessions.clone();
        Box::pin(async move {
            match tokio::task::spawn_blocking(move || run_upsert(sessions, input, execution)).await
            {
                Ok(result) => result,
                Err(error) => worker_error("memory.upsert", error),
            }
        })
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        code_surface_only()
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(10))
    }
}

impl Tool for MemoryDeleteTool {
    fn name(&self) -> &str {
        MEMORY_DELETE_TOOL
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        let sessions = self.handle.sessions.clone();
        Box::pin(async move {
            match tokio::task::spawn_blocking(move || run_delete(sessions, input, execution)).await
            {
                Ok(result) => result,
                Err(error) => worker_error("memory.delete", error),
            }
        })
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        code_surface_only()
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(10))
    }
}

fn run_list(
    sessions: Option<Arc<SessionManager>>,
    input: Value,
    execution: ToolExecutionContext,
) -> ToolCallResult {
    if let Err(error) = validate_list_input(&input) {
        return coded("invalid_input", error);
    }
    let Some(manager) = sessions else {
        return coded(
            "unavailable",
            "memory.list is not bound to a session manager",
        );
    };
    let session_id = execution.session_id.trim();
    if session_id.is_empty() {
        return coded("invalid_input", "memory.list needs an active session");
    }
    let memory = match manager.load_memory(session_id) {
        Ok(memory) => memory,
        Err(error) => return coded("io", format!("load session memory: {error}")),
    };
    let query = input
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty());
    let kind = input
        .get("kind")
        .and_then(Value::as_str)
        .and_then(MemoryKind::parse);
    let entries: Vec<Value> = memory
        .entries
        .iter()
        .filter(|entry| entry.matches(query, kind))
        .map(entry_json)
        .collect();
    ok_data(json!({
        "entries": entries,
        "count": entries.len(),
        "total": memory.entries.len(),
        "limit": crate::session::memory::MEMORY_MAX_ENTRIES,
        "total_chars": memory.total_chars(),
    }))
}

fn run_upsert(
    sessions: Option<Arc<SessionManager>>,
    input: Value,
    execution: ToolExecutionContext,
) -> ToolCallResult {
    if let Err(error) = validate_upsert_input(&input) {
        return coded("invalid_input", error);
    }
    let Some(manager) = sessions else {
        return coded(
            "unavailable",
            "memory.upsert is not bound to a session manager",
        );
    };
    let session_id = execution.session_id.trim();
    if session_id.is_empty() {
        return coded("invalid_input", "memory.upsert needs an active session");
    }
    let requests = parse_upsert_requests(&input);
    let now = chrono::Utc::now().timestamp_millis();
    let (memory, result) =
        match manager.update_memory(session_id, |memory| Ok(memory.apply_upsert(&requests, now))) {
            Ok(value) => value,
            Err(error) => return coded("io", format!("save session memory: {error}")),
        };
    ok_data(json!({
        "added": result.added,
        "updated": result.updated,
        "rejected": rejected_json(&result.rejected),
        "count": memory.entries.len(),
        "total_chars": memory.total_chars(),
    }))
}

fn run_delete(
    sessions: Option<Arc<SessionManager>>,
    input: Value,
    execution: ToolExecutionContext,
) -> ToolCallResult {
    if let Err(error) = validate_delete_input(&input) {
        return coded("invalid_input", error);
    }
    let Some(manager) = sessions else {
        return coded(
            "unavailable",
            "memory.delete is not bound to a session manager",
        );
    };
    let session_id = execution.session_id.trim();
    if session_id.is_empty() {
        return coded("invalid_input", "memory.delete needs an active session");
    }
    let ids: Vec<String> = input["ids"]
        .as_array()
        .expect("validated")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    let (memory, result) =
        match manager.update_memory(session_id, |memory| Ok(memory.apply_delete(&ids))) {
            Ok(value) => value,
            Err(error) => return coded("io", format!("save session memory: {error}")),
        };
    ok_data(json!({
        "deleted": result.deleted,
        "rejected": rejected_json(&result.rejected),
        "count": memory.entries.len(),
    }))
}

fn validate_list_input(input: &Value) -> std::result::Result<(), String> {
    if let Some(value) = input.get("query") {
        let query = crate::tool::require_string_value(value, "query")?;
        if query.trim().chars().count() > MEMORY_MAX_QUERY_CHARS {
            return Err(format!(
                "query must be at most {MEMORY_MAX_QUERY_CHARS} characters"
            ));
        }
    }
    if let Some(value) = input.get("kind") {
        let kind = crate::tool::require_string_value(value, "kind")?;
        if MemoryKind::parse(kind).is_none() {
            return Err(crate::tool::must_be_one_of("kind", KIND_NAMES, kind));
        }
    }
    Ok(())
}

fn validate_upsert_input(input: &Value) -> std::result::Result<(), String> {
    let entries = match input.get("entries") {
        None => return Err(crate::tool::missing_parameter("entries")),
        Some(value) => value
            .as_array()
            .ok_or_else(|| crate::tool::expected_type("entries", "array", value))?,
    };
    if entries.is_empty() {
        return Err("entries must contain at least 1 item".into());
    }
    if entries.len() > MEMORY_MAX_BATCH {
        return Err(format!(
            "entries must contain at most {MEMORY_MAX_BATCH} items"
        ));
    }
    for (index, item) in entries.iter().enumerate() {
        let path = format!("entries[{index}]");
        if !item.is_object() {
            return Err(crate::tool::expected_type(&path, "object", item));
        }
        match item.get("text") {
            None => return Err(crate::tool::missing_parameter(&format!("{path}.text"))),
            Some(value) => {
                crate::tool::require_string_value(value, &format!("{path}.text"))?;
            }
        }
        if let Some(value) = item.get("id") {
            crate::tool::require_string_value(value, &format!("{path}.id"))?;
        }
        if let Some(value) = item.get("kind") {
            crate::tool::require_string_value(value, &format!("{path}.kind"))?;
        }
    }
    Ok(())
}

fn validate_delete_input(input: &Value) -> std::result::Result<(), String> {
    let ids = match input.get("ids") {
        None => return Err(crate::tool::missing_parameter("ids")),
        Some(value) => value
            .as_array()
            .ok_or_else(|| crate::tool::expected_type("ids", "array", value))?,
    };
    if ids.is_empty() {
        return Err("ids must contain at least 1 item".into());
    }
    if ids.len() > MEMORY_MAX_BATCH {
        return Err(format!("ids must contain at most {MEMORY_MAX_BATCH} items"));
    }
    for (index, value) in ids.iter().enumerate() {
        crate::tool::require_string_value(value, &format!("ids[{index}]"))?;
    }
    Ok(())
}

fn parse_upsert_requests(input: &Value) -> Vec<MemoryUpsertRequest> {
    input["entries"]
        .as_array()
        .expect("validated")
        .iter()
        .map(|item| MemoryUpsertRequest {
            id: item.get("id").and_then(Value::as_str).map(str::to_string),
            text: item["text"].as_str().unwrap_or_default().to_string(),
            kind: item.get("kind").and_then(Value::as_str).map(str::to_string),
        })
        .collect()
}

fn entry_json(entry: &MemoryEntry) -> Value {
    json!({
        "id": entry.id,
        "text": entry.text,
        "kind": entry.kind.as_str(),
        "updated_at": entry.updated_at,
    })
}

fn rejected_json(items: &[MemoryReject]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|item| {
                let mut value = json!({
                    "index": item.index,
                    "reason": item.reason,
                });
                if let Some(id) = &item.id {
                    value["id"] = json!(id);
                }
                value
            })
            .collect(),
    )
}

fn coded(code: &str, message: impl Into<String>) -> ToolCallResult {
    crate::tools::file_path::coded_error(code, message)
}

fn worker_error(tool: &str, error: tokio::task::JoinError) -> ToolCallResult {
    ToolCallResult::error_with_kind(
        crate::types::ToolErrorKind::Internal,
        format!("{tool} worker failed: {error}"),
        false,
    )
    .with_error_code("internal")
}

fn code_surface_only() -> ToolCallResult {
    coded(
        "unavailable",
        "memory tools are only callable from run_code with an active session",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolSignalLevel;

    fn ctx(session_id: &str) -> ToolExecutionContext {
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: std::env::temp_dir(),
            call_id: "memory-test".into(),
            cancel: tokio_util::sync::CancellationToken::new(),
            output_limit: 8000,
            session_id: session_id.to_string(),
            session: None,
        }
    }

    #[tokio::test]
    async fn upsert_list_delete_round_trip() {
        let manager = Arc::new(SessionManager::ephemeral_registry());
        let session_id = manager.open_session_sync("", "default", None).unwrap();
        let upsert = MemoryUpsertTool::new(Arc::clone(&manager));
        let list = MemoryListTool::new(Arc::clone(&manager));
        let delete = MemoryDeleteTool::new(Arc::clone(&manager));

        let result = upsert
            .execute(
                json!({"entries": [{"text": "use pnpm", "kind": "preference"}]}),
                ctx(&session_id),
            )
            .await;
        assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
        let data = result.data.expect("upsert data");
        assert_eq!(data["added"], json!(["m1"]));
        assert_eq!(data["count"], 1);

        // Same text, different case: update in place, no duplicate.
        let result = upsert
            .execute(json!({"entries": [{"text": "USE PNPM"}]}), ctx(&session_id))
            .await;
        let data = result.data.expect("upsert data");
        assert_eq!(data["added"], json!([]));
        assert_eq!(data["updated"], json!(["m1"]));
        assert_eq!(data["count"], 1);

        let result = list
            .execute(
                json!({"query": "pnpm", "kind": "preference"}),
                ctx(&session_id),
            )
            .await;
        let data = result.data.expect("list data");
        assert_eq!(data["count"], 1);
        assert_eq!(data["entries"][0]["id"], "m1");
        assert_eq!(data["entries"][0]["text"], "USE PNPM");

        let result = upsert
            .execute(
                json!({"entries": [{"text": "second"}, {"text": ""}]}),
                ctx(&session_id),
            )
            .await;
        let data = result.data.expect("upsert data");
        assert_eq!(data["added"], json!(["m2"]));
        assert_eq!(data["rejected"][0]["reason"], "empty_text");
        assert_eq!(data["count"], 2);

        let result = delete
            .execute(json!({"ids": ["m1", "m9"]}), ctx(&session_id))
            .await;
        let data = result.data.expect("delete data");
        assert_eq!(data["deleted"], json!(["m1"]));
        assert_eq!(data["rejected"][0]["reason"], "unknown_id");
        assert_eq!(data["count"], 1);

        let loaded = manager.load_memory(&session_id).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].id, "m2");
    }

    #[test]
    fn input_validation_rejects_bad_batch_shapes() {
        assert!(validate_upsert_input(&json!({"entries": []})).is_err());
        assert!(validate_upsert_input(&json!({"entries": [{"id": 1, "text": "x"}]})).is_err());
        assert!(validate_upsert_input(&json!({"entries": [{"text": 1}]})).is_err());
        assert!(validate_upsert_input(&json!({"entries": [{"text": "x"}]})).is_ok());
        assert!(validate_delete_input(&json!({"ids": []})).is_err());
        assert!(validate_delete_input(&json!({"ids": [1]})).is_err());
        assert!(validate_delete_input(&json!({"ids": ["m1"]})).is_ok());
        assert!(
            validate_list_input(&json!({"query": "x".repeat(MEMORY_MAX_QUERY_CHARS + 1)})).is_err()
        );
        assert!(validate_list_input(&json!({"kind": "nope"})).is_err());
        assert!(validate_list_input(&json!({"kind": "fact"})).is_ok());
    }
}
