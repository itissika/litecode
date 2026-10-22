//! JSON-RPC 2.0 wire types for the code-search worker.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engines::code_search::SearchHit;

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PingResult {
    pub ready: bool,
    /// Live worker inference device (`cuda-ort`, `cpu-ort`, `hash`). Empty before warmup.
    #[serde(default)]
    pub embed_device: String,
}

impl JsonRpcResponse {
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InitializeParams {
    pub workspace_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_db_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SetSessionDbParams {
    pub session_db_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchParams {
    pub query: String,
    #[serde(default)]
    pub glob: Option<String>,
    pub top_k: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionSearchParams {
    pub query: String,
    pub top_k: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionSearchResult {
    pub hits: Vec<crate::engines::session_search::SessionTextHit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshMode {
    Rebuild,
    Incremental,
}

/// Which corpus a `refresh` RPC should consume.
///
/// Human Refresh sends [`RefreshScope::All`]. Agent `code_search` / `session_search`
/// send only their own corpus so they do not mark the other busy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshScope {
    #[default]
    All,
    Code,
    Session,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RefreshParams {
    #[serde(default)]
    pub scope: RefreshScope,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RefreshResult {
    pub mode: RefreshMode,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NotifyFsChangesParams {
    pub paths: Vec<String>,
    pub deleted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_result_defaults_empty_device() {
        let back: PingResult =
            serde_json::from_value(serde_json::json!({ "ready": true })).unwrap();
        assert!(back.ready);
        assert!(back.embed_device.is_empty());
    }

    #[test]
    fn ping_result_roundtrip_cuda() {
        let v = serde_json::to_value(PingResult {
            ready: true,
            embed_device: "cuda-ort".into(),
        })
        .unwrap();
        let back: PingResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.embed_device, "cuda-ort");
    }

    #[test]
    fn refresh_params_empty_defaults_all() {
        let back: RefreshParams = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(back.scope, RefreshScope::All);
    }

    #[test]
    fn refresh_params_roundtrip_code_and_session() {
        for scope in [RefreshScope::Code, RefreshScope::Session, RefreshScope::All] {
            let v = serde_json::to_value(RefreshParams { scope }).unwrap();
            let back: RefreshParams = serde_json::from_value(v).unwrap();
            assert_eq!(back.scope, scope);
        }
    }
}
