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

#[derive(Debug, Serialize, Deserialize)]
pub struct RefreshResult {
    pub mode: RefreshMode,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NotifyFsChangesParams {
    pub paths: Vec<String>,
    pub deleted: bool,
}
