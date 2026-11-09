//! Session-scoped working memory tools.
//!
//! These are code-surface tools: one `run_code` program can batch several
//! atomic calls without a second round trip. The durable document lives in
//! `sessions.memory_json`; the tool layer only enforces the small contract.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::session::manager::SessionManager;
use crate::session::memory::{
    MEMORY_MAX_BATCH, MEMORY_MAX_ENTRIES, MemoryDeleteResult, MemoryEntry, MemoryReject,
    MemoryUpsertRequest, MemoryUpsertResult, SessionMemory,
};
use crate::tool::Tool;
use crate::tool::outcome::ok_data;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

pub const MEMORY_LIST_TOOL: &str = "memory.list";
pub const MEMORY_UPSERT_TOOL: &str = "memory.upsert";
pub const MEMORY_DELETE_TOOL: &str = "memory.delete";

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
    _input: Value,
    execution: ToolExecutionContext,
) -> ToolCallResult {
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
    ok_data(list_data(&memory))
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
    ok_data(upsert_data(&memory, result))
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
    ok_data(delete_data(&memory, result))
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
        })
        .collect()
}

/// Sole success shape for memory.list.
#[derive(Debug, Serialize)]
struct MemoryListData {
    entries: Vec<MemoryEntryData>,
    count: usize,
    total: usize,
    limit: usize,
    total_chars: usize,
}

/// Sole success shape for memory.upsert.
#[derive(Debug, Serialize)]
struct MemoryUpsertData {
    added: Vec<String>,
    updated: Vec<String>,
    rejected: Vec<RejectedData>,
    count: usize,
    total_chars: usize,
}

/// Sole success shape for memory.delete.
#[derive(Debug, Serialize)]
struct MemoryDeleteData {
    deleted: Vec<String>,
    rejected: Vec<RejectedData>,
    count: usize,
}

#[derive(Debug, Serialize)]
struct MemoryEntryData {
    id: String,
    text: String,
    updated_at: i64,
}

/// One rejected batch item. The id property is absent when the request carried none.
#[derive(Debug, Serialize)]
struct RejectedData {
    index: usize,
    reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

fn list_data(memory: &SessionMemory) -> MemoryListData {
    let entries: Vec<MemoryEntryData> = memory.entries.iter().map(entry_data).collect();
    MemoryListData {
        count: entries.len(),
        total: memory.entries.len(),
        limit: MEMORY_MAX_ENTRIES,
        total_chars: memory.total_chars(),
        entries,
    }
}

fn upsert_data(memory: &SessionMemory, result: MemoryUpsertResult) -> MemoryUpsertData {
    MemoryUpsertData {
        added: result.added,
        updated: result.updated,
        rejected: rejected_data(&result.rejected),
        count: memory.entries.len(),
        total_chars: memory.total_chars(),
    }
}

fn delete_data(memory: &SessionMemory, result: MemoryDeleteResult) -> MemoryDeleteData {
    MemoryDeleteData {
        deleted: result.deleted,
        rejected: rejected_data(&result.rejected),
        count: memory.entries.len(),
    }
}

fn entry_data(entry: &MemoryEntry) -> MemoryEntryData {
    MemoryEntryData {
        id: entry.id.clone(),
        text: entry.text.clone(),
        updated_at: entry.updated_at,
    }
}

fn rejected_data(items: &[MemoryReject]) -> Vec<RejectedData> {
    items
        .iter()
        .map(|item| RejectedData {
            index: item.index,
            reason: item.reason,
            id: item.id.clone(),
            detail: item.detail.clone(),
        })
        .collect()
}

/// Drift samples for the three memory tools. Each state comes from the same
/// apply_* operation the tool runs, then the same data function the tool uses.
#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    use crate::tool::code_contract::DriftCase;
    let mut memory = SessionMemory::default();
    memory.entries.push(MemoryEntry {
        id: "m1".into(),
        text: "use pnpm".into(),
        updated_at: 1,
    });
    memory.next_id = 2;

    // The "all optional present" state must carry every optional field on the
    // rejection itself: an existing id plus a limit that pushes it over.
    let mut upsert_present = SessionMemory::default();
    upsert_present.apply_upsert(
        &[MemoryUpsertRequest {
            id: None,
            text: "h".repeat(100),
        }],
        1,
    );
    upsert_present.apply_upsert(
        &(0..20)
            .map(|i| MemoryUpsertRequest {
                id: None,
                text: format!("{i:0>195}"),
            })
            .collect::<Vec<_>>(),
        1,
    );
    let upsert_present_result = upsert_present.apply_upsert(
        &[MemoryUpsertRequest {
            id: Some("m1".into()),
            text: "y".repeat(200),
        }],
        2,
    );
    let mut upsert_absent = SessionMemory::default();
    let upsert_absent_result = upsert_absent.apply_upsert(
        &[
            MemoryUpsertRequest {
                id: None,
                text: " ".into(),
            },
            MemoryUpsertRequest {
                id: None,
                text: "use pnpm".into(),
            },
        ],
        1,
    );

    let mut delete_present = memory.clone();
    let delete_present_result = delete_present.apply_delete(&["m1".to_string(), "m9".to_string()]);
    let mut delete_absent = memory.clone();
    let delete_absent_result = delete_absent.apply_delete(&["".to_string()]);

    vec![
        DriftCase {
            tool: MEMORY_LIST_TOOL,
            // memory.list has no optional property, so both states use the same shape.
            all_optional_present: ok_data(list_data(&memory)),
            all_optional_absent: ok_data(list_data(&memory)),
        },
        DriftCase {
            tool: MEMORY_UPSERT_TOOL,
            all_optional_present: ok_data(upsert_data(&upsert_present, upsert_present_result)),
            all_optional_absent: ok_data(upsert_data(&upsert_absent, upsert_absent_result)),
        },
        DriftCase {
            tool: MEMORY_DELETE_TOOL,
            all_optional_present: ok_data(delete_data(&delete_present, delete_present_result)),
            all_optional_absent: ok_data(delete_data(&delete_absent, delete_absent_result)),
        },
    ]
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
    use serde_json::json;

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
            .execute(json!({"entries": [{"text": "use pnpm"}]}), ctx(&session_id))
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

        let result = list.execute(json!({}), ctx(&session_id)).await;
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

        let result = list.execute(json!({}), ctx(&session_id)).await;
        let data = result.data.expect("full list data");
        assert_eq!(data["count"], 2);
        assert_eq!(data["total"], 2);
        assert_eq!(data["entries"].as_array().unwrap().len(), 2);

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
    }
}
