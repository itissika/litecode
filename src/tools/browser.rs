use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};

use crate::context_pipeline::Context;
use crate::tool::outcome::{ToolOutcome, ok_data};
use crate::tool::schema_validate::{require_nonempty_string_trimmed, require_string};
use crate::tool::trait_::{Tool, ToolExecutionContext, ToolSurface};
use crate::types::ToolCallResult;

pub const BROWSER_CARD_ID: &str = "browser";
pub const BROWSER_CODE_TOOL_NAMES: &[&str] = &[
    "browser_open",
    "browser_close",
    "browser_list",
    "browser_read",
    "browser_click",
    "browser_type",
];

const MAX_URL_LENGTH: usize = 8_000;
const MAX_PAGE_ID_LENGTH: usize = 100;
const MAX_TYPE_LENGTH: usize = 4_000;
const DEFAULT_READ_CHARS: u64 = 12_000;
const MIN_READ_CHARS: u64 = 1_000;
const MAX_READ_CHARS: u64 = 30_000;

#[derive(Debug)]
struct BrowserCallError {
    code: &'static str,
    message: String,
    retryable: bool,
}

impl BrowserCallError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: "unavailable",
            message: message.into(),
            retryable: false,
        }
    }
}

fn http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        })
        .clone()
}

/// Loopback-only HTTP client for the Electron main-process bridge.
#[derive(Clone)]
pub struct BrowserClient {
    endpoint: Option<String>,
    token: Option<String>,
    http: reqwest::Client,
}

impl BrowserClient {
    pub fn from_env() -> Self {
        let endpoint = std::env::var("LITECODE_BROWSER_BRIDGE_URL")
            .ok()
            .and_then(|raw| loopback_endpoint(&raw));
        let token = std::env::var("LITECODE_BROWSER_BRIDGE_TOKEN")
            .ok()
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty());
        Self {
            endpoint,
            token,
            http: http_client(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, BrowserCallError> {
        self.send(false, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value, BrowserCallError> {
        self.send(true, path, Some(body)).await
    }

    async fn send(
        &self,
        post: bool,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, BrowserCallError> {
        let Some(endpoint) = self.endpoint.as_deref() else {
            return Err(BrowserCallError::unavailable(
                "Browser tools need the LiteCode desktop app in a local session",
            ));
        };
        let Some(token) = self.token.as_deref() else {
            return Err(BrowserCallError::unavailable(
                "Browser bridge token is missing; restart the LiteCode desktop app",
            ));
        };
        let url = format!("{endpoint}{path}");
        let mut request = if post {
            self.http.post(url)
        } else {
            self.http.get(url)
        }
        .bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(transport_error)?;
        let status = response.status();
        let body = response.bytes().await.map_err(transport_error)?;
        let payload: Value = serde_json::from_slice(&body).map_err(|_| {
            BrowserCallError::unavailable("Browser bridge returned an invalid response")
        })?;
        if !status.is_success() || payload.get("ok").and_then(Value::as_bool) != Some(true) {
            let raw_code = payload.get("code").and_then(Value::as_str).unwrap_or("");
            let code = match raw_code {
                "invalid_input" => "invalid_input",
                "permission_denied" => "permission_denied",
                "not_found" => "not_found",
                "conflict" | "stale" => "conflict",
                "timeout" => "timeout",
                "unauthorized" | "internal" | "unavailable" | "" => "unavailable",
                _ => "unavailable",
            };
            let message = payload
                .get("message")
                .and_then(Value::as_str)
                .filter(|message| !message.is_empty())
                .unwrap_or("Browser bridge request failed")
                .to_string();
            return Err(BrowserCallError {
                code,
                message,
                retryable: matches!(code, "conflict" | "timeout"),
            });
        }
        Ok(payload.get("data").cloned().unwrap_or(Value::Null))
    }
}

fn transport_error(error: reqwest::Error) -> BrowserCallError {
    BrowserCallError {
        code: if error.is_timeout() {
            "timeout"
        } else {
            "unavailable"
        },
        message: format!("Browser bridge request failed: {error}"),
        retryable: error.is_timeout() || error.is_connect(),
    }
}

/// Keep the environment from turning a browser tool into an arbitrary HTTP client.
fn loopback_endpoint(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw.trim()).ok()?;
    if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") {
        return None;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    if !url.path().is_empty() && url.path() != "/" {
        return None;
    }
    if url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    let port = url.port()?;
    Some(format!("http://127.0.0.1:{port}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserOp {
    Open,
    Close,
    List,
    Read,
    Click,
    Type,
}

impl BrowserOp {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "browser_open" => Some(Self::Open),
            "browser_close" => Some(Self::Close),
            "browser_list" => Some(Self::List),
            "browser_read" => Some(Self::Read),
            "browser_click" => Some(Self::Click),
            "browser_type" => Some(Self::Type),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Open => "browser_open",
            Self::Close => "browser_close",
            Self::List => "browser_list",
            Self::Read => "browser_read",
            Self::Click => "browser_click",
            Self::Type => "browser_type",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Open => include_str!("descriptions/browser_open.md"),
            Self::Close => include_str!("descriptions/browser_close.md"),
            Self::List => include_str!("descriptions/browser_list.md"),
            Self::Read => include_str!("descriptions/browser_read.md"),
            Self::Click => include_str!("descriptions/browser_click.md"),
            Self::Type => include_str!("descriptions/browser_type.md"),
        }
    }

    fn schema(self) -> Value {
        match self {
            Self::Open => json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "http(s) URL to open in a visible browser tab."
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
            Self::Close => json!({
                "type": "object",
                "properties": {
                    "page_id": {
                        "type": "string",
                        "description": "A browser page id returned by browser_open or browser_list."
                    }
                },
                "required": ["page_id"],
                "additionalProperties": false
            }),
            Self::List => json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            Self::Read => json!({
                "type": "object",
                "properties": {
                    "page_id": {
                        "type": "string",
                        "description": "Optional page id. Omit for the active visible tab."
                    },
                    "max_chars": {
                        "type": "integer",
                        "minimum": MIN_READ_CHARS,
                        "maximum": MAX_READ_CHARS,
                        "description": "Maximum body text characters. Default 12000."
                    }
                },
                "additionalProperties": false
            }),
            Self::Click => json!({
                "type": "object",
                "properties": {
                    "page_id": {
                        "type": "string",
                        "description": "Optional page id. Omit for the active visible tab."
                    },
                    "ref": {
                        "type": "string",
                        "description": "An element ref from the latest browser_read."
                    }
                },
                "required": ["ref"],
                "additionalProperties": false
            }),
            Self::Type => json!({
                "type": "object",
                "properties": {
                    "page_id": {
                        "type": "string",
                        "description": "Optional page id. Omit for the active visible tab."
                    },
                    "ref": {
                        "type": "string",
                        "description": "An input ref from the latest browser_read."
                    },
                    "text": {
                        "type": "string",
                        "description": "Text to set on the focused input."
                    }
                },
                "required": ["ref", "text"],
                "additionalProperties": false
            }),
        }
    }

    fn data_schema(self) -> Value {
        match self {
            Self::Open => json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "id": { "type": "string" },
                    "url": { "type": "string" },
                    "title": { "type": "string" },
                    "active": { "type": "boolean" },
                    "visible": { "type": "boolean" },
                    "agentOwned": { "type": "boolean" }
                },
                "required": ["id", "url", "title", "active", "visible", "agentOwned"]
            }),
            Self::Close => json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "id": { "type": "string" },
                    "closed": { "type": "boolean" },
                    "fallback": { "type": "boolean" }
                },
                "required": ["id", "closed"]
            }),
            Self::List => json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "pages": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "id": { "type": "string" },
                                "url": { "type": "string" },
                                "title": { "type": "string" },
                                "active": { "type": "boolean" },
                                "visible": { "type": "boolean" },
                                "agentOwned": { "type": "boolean" }
                            },
                            "required": ["id", "url", "title", "active", "visible", "agentOwned"]
                        }
                    },
                    "active_id": { "type": "string" }
                },
                "required": ["pages", "active_id"]
            }),
            Self::Read => json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "page_id": { "type": "string" },
                    "url": { "type": "string" },
                    "title": { "type": "string" },
                    "text": { "type": "string" },
                    "truncated": { "type": "boolean" },
                    "elements": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "ref": { "type": "string" },
                                "tag": { "type": "string" },
                                "text": { "type": "string" },
                                "type": { "type": "string" },
                                "href": { "type": "string" }
                            },
                            "required": ["ref", "tag", "text"]
                        }
                    }
                },
                "required": ["page_id", "url", "title", "text", "truncated", "elements"]
            }),
            Self::Click => json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "page_id": { "type": "string" },
                    "ref": { "type": "string" },
                    "tag": { "type": "string" },
                    "text": { "type": "string" },
                    "url": { "type": "string" },
                    "title": { "type": "string" }
                },
                "required": ["page_id", "ref", "tag", "text", "url", "title"]
            }),
            Self::Type => json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "page_id": { "type": "string" },
                    "ref": { "type": "string" },
                    "tag": { "type": "string" },
                    "url": { "type": "string" },
                    "title": { "type": "string" }
                },
                "required": ["page_id", "ref", "tag", "url", "title"]
            }),
        }
    }

    fn timeout(self) -> Duration {
        match self {
            Self::Open => Duration::from_secs(15),
            Self::Close => Duration::from_secs(10),
            Self::Read => Duration::from_secs(20),
            Self::Click | Self::Type => Duration::from_secs(20),
            Self::List => Duration::from_secs(10),
        }
    }
}

pub struct BrowserTool {
    op: BrowserOp,
    client: BrowserClient,
}

impl BrowserTool {
    pub fn new(op: BrowserOp) -> Self {
        Self {
            op,
            client: BrowserClient::from_env(),
        }
    }

    pub fn tool_for_name(name: &str) -> Option<Arc<dyn Tool>> {
        BrowserOp::from_name(name).map(|op| Arc::new(Self::new(op)) as Arc<dyn Tool>)
    }
}

pub fn all_code_tools() -> Vec<Arc<dyn Tool>> {
    BROWSER_CODE_TOOL_NAMES
        .iter()
        .filter_map(|name| BrowserTool::tool_for_name(name))
        .collect()
}

impl BrowserTool {
    async fn call_bridge(&self, input: &Value) -> Result<Value, BrowserCallError> {
        match self.op {
            BrowserOp::Open => {
                let url = require_nonempty_string_trimmed(input, "url").map_err(invalid_input)?;
                self.client
                    .post("/open", json!({ "url": url.trim() }))
                    .await
            }
            BrowserOp::Close => {
                let page_id = page_id(input, "page_id")?
                    .ok_or_else(|| invalid_input("page_id is required"))?;
                self.client
                    .post("/close", json!({ "page_id": page_id }))
                    .await
            }
            BrowserOp::List => self.client.get("/list").await,
            BrowserOp::Read => {
                let page_id = page_id(input, "page_id")?;
                let max_chars = read_chars(input)?;
                let mut body = json!({ "max_chars": max_chars });
                if let Some(page_id) = page_id {
                    body["page_id"] = Value::String(page_id);
                }
                self.client.post("/read", body).await
            }
            BrowserOp::Click => {
                let ref_id = element_ref(input)?;
                let page_id = page_id(input, "page_id")?;
                let mut body = json!({ "ref": ref_id });
                if let Some(page_id) = page_id {
                    body["page_id"] = Value::String(page_id);
                }
                self.client.post("/click", body).await
            }
            BrowserOp::Type => {
                let ref_id = element_ref(input)?;
                let text = require_string(input, "text").map_err(invalid_input)?;
                let page_id = page_id(input, "page_id")?;
                let mut body = json!({ "ref": ref_id, "text": text });
                if let Some(page_id) = page_id {
                    body["page_id"] = Value::String(page_id);
                }
                self.client.post("/type", body).await
            }
        }
    }

    async fn run(
        &self,
        input: Value,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> ToolCallResult {
        if let Err(message) = self.validate_input(&input) {
            return failure("invalid_input", message, false);
        }
        let result = tokio::select! {
            _ = cancel.cancelled() => {
                return failure("cancelled", "Browser operation was cancelled", false);
            }
            result = self.call_bridge(&input) => result,
        };
        match result {
            Ok(data) => ok_data(data),
            Err(error) => failure(error.code, error.message, error.retryable),
        }
    }
}

impl Tool for BrowserTool {
    fn name(&self) -> &str {
        self.op.name()
    }

    fn surface(&self) -> ToolSurface {
        ToolSurface::Code
    }

    fn schema(&self) -> Value {
        self.op.schema()
    }

    fn data_schema(&self) -> Option<Value> {
        Some(self.op.data_schema())
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(self.op.description())
    }

    fn timeout(&self) -> Option<Duration> {
        Some(self.op.timeout())
    }

    fn is_cancellable(&self) -> bool {
        true
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let cancel = execution.cancel.clone();
        Box::pin(async move { self.run(input, &cancel).await })
    }

    fn call_async(
        &self,
        input: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let cancel = tokio_util::sync::CancellationToken::new();
        Box::pin(async move { self.run(input, &cancel).await })
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        match self.op {
            BrowserOp::Open => validate_url(input)?,
            BrowserOp::Close => {
                page_id(input, "page_id")?.ok_or_else(|| "page_id is required".to_string())?;
            }
            BrowserOp::List => {}
            BrowserOp::Read => {
                page_id(input, "page_id")?;
                read_chars(input)?;
            }
            BrowserOp::Click => {
                page_id(input, "page_id")?;
                element_ref(input)?;
            }
            BrowserOp::Type => {
                page_id(input, "page_id")?;
                element_ref(input)?;
                let text = require_string(input, "text")?;
                if text.chars().count() > MAX_TYPE_LENGTH {
                    return Err(format!("text exceeds {MAX_TYPE_LENGTH} characters"));
                }
            }
        }
        Ok(())
    }
}

fn invalid_input(message: impl Into<String>) -> BrowserCallError {
    BrowserCallError {
        code: "invalid_input",
        message: message.into(),
        retryable: false,
    }
}

impl From<String> for BrowserCallError {
    fn from(message: String) -> Self {
        invalid_input(message)
    }
}

fn failure(code: &str, message: impl Into<String>, retryable: bool) -> ToolCallResult {
    ToolOutcome::<Value>::failure(code, message, retryable).into_call_result("")
}

fn validate_url(input: &Value) -> std::result::Result<(), String> {
    let url = require_nonempty_string_trimmed(input, "url")?;
    if url.len() > MAX_URL_LENGTH {
        return Err(format!("url exceeds {MAX_URL_LENGTH} characters"));
    }
    let parsed = url::Url::parse(url).map_err(|_| "url must be a valid http(s) URL".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("url must use http or https".to_string());
    }
    if parsed.host_str().is_none() {
        return Err("url must name a host".to_string());
    }
    Ok(())
}

fn page_id(input: &Value, name: &str) -> std::result::Result<Option<String>, String> {
    let Some(value) = input.get(name) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let id = require_string(input, name)?;
    if id.trim().is_empty() {
        return Ok(None);
    }
    if id.len() > MAX_PAGE_ID_LENGTH || !id.starts_with("browser-") {
        return Err(format!("{name} is not a browser page id"));
    }
    Ok(Some(id.to_string()))
}

fn element_ref(input: &Value) -> std::result::Result<String, String> {
    let ref_id = require_string(input, "ref")?;
    let valid = ref_id.strip_prefix('e').is_some_and(|digits| {
        !digits.is_empty()
            && digits.len() <= 4
            && digits.chars().all(|ch| ch.is_ascii_digit())
            && !digits.starts_with('0')
    });
    if !valid {
        return Err("ref must come from the latest browser_read".to_string());
    }
    Ok(ref_id.to_string())
}

fn read_chars(input: &Value) -> std::result::Result<u64, String> {
    let Some(value) = input.get("max_chars") else {
        return Ok(DEFAULT_READ_CHARS);
    };
    let Some(number) = value.as_u64() else {
        return Err("max_chars must be an integer".to_string());
    };
    if !(MIN_READ_CHARS..=MAX_READ_CHARS).contains(&number) {
        return Err(format!(
            "max_chars must be between {MIN_READ_CHARS} and {MAX_READ_CHARS}"
        ));
    }
    Ok(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(op: BrowserOp) -> BrowserTool {
        BrowserTool {
            op,
            client: BrowserClient {
                endpoint: None,
                token: None,
                http: http_client(),
            },
        }
    }

    #[test]
    fn opens_only_http_and_https() {
        let open = tool(BrowserOp::Open);
        assert!(
            open.validate_input(&json!({"url": "https://example.com"}))
                .is_ok()
        );
        assert!(
            open.validate_input(&json!({"url": "http://example.com"}))
                .is_ok()
        );
        assert!(
            open.validate_input(&json!({"url": "file:///etc/passwd"}))
                .is_err()
        );
        assert!(
            open.validate_input(&json!({"url": "javascript:alert(1)"}))
                .is_err()
        );
        assert!(open.validate_input(&json!({"url": "not a url"})).is_err());
    }

    #[test]
    fn refs_are_strict_and_page_ids_are_optional() {
        let click = tool(BrowserOp::Click);
        assert!(click.validate_input(&json!({"ref": "e1"})).is_ok());
        assert!(
            click
                .validate_input(&json!({"ref": "e12", "page_id": "browser-1"}))
                .is_ok()
        );
        assert!(click.validate_input(&json!({"ref": "1"})).is_err());
        assert!(click.validate_input(&json!({"ref": "e0"})).is_err());
        assert!(
            click
                .validate_input(&json!({"ref": "e1", "page_id": "other"}))
                .is_err()
        );
    }

    #[test]
    fn read_chars_respects_the_bridge_bounds() {
        let read = tool(BrowserOp::Read);
        assert!(read.validate_input(&json!({})).is_ok());
        assert!(read.validate_input(&json!({"max_chars": 1000})).is_ok());
        assert!(read.validate_input(&json!({"max_chars": 999})).is_err());
        assert!(read.validate_input(&json!({"max_chars": 30_001})).is_err());
    }

    #[test]
    fn type_text_is_bounded() {
        let typing = tool(BrowserOp::Type);
        assert!(
            typing
                .validate_input(&json!({"ref": "e1", "text": ""}))
                .is_ok()
        );
        assert!(
            typing
                .validate_input(&json!({"ref": "e1", "text": "hello"}))
                .is_ok()
        );
        assert!(
            typing
                .validate_input(&json!({"ref": "e1", "text": "x".repeat(4001)}))
                .is_err()
        );
    }

    #[test]
    fn loopback_endpoint_rejects_other_hosts_and_paths() {
        assert_eq!(
            loopback_endpoint("http://127.0.0.1:43121"),
            Some("http://127.0.0.1:43121".to_string())
        );
        assert_eq!(loopback_endpoint("http://localhost:43121"), None);
        assert_eq!(loopback_endpoint("http://127.0.0.1:43121/evil"), None);
        assert_eq!(loopback_endpoint("https://127.0.0.1:43121"), None);
    }
}
