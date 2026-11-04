//! Stdio bridge to a running app's own MCP endpoint.
//!
//! A bundled app picks a fresh port on every start, so an MCP client config
//! that names `127.0.0.1:<port>` goes stale as soon as the app restarts. This
//! command is the stable handle: the client names the app, and the port is
//! resolved from the live run marker on each request, so one config keeps
//! working across restarts.
//!
//! It is transport plumbing only. Whether an MCP server is configured at all
//! stays the user's decision: nothing here runs unless a client definition
//! names this command.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::runtime::{any_live_run_port, live_run_port};
use super::schema::validate_id;

const MAX_ERROR_CHARS: usize = 400;

struct Target {
    app_id: String,
    path: String,
    headers: Vec<(String, String)>,
    global_data: PathBuf,
    workspace_hash: String,
}

impl Target {
    fn port(&self) -> Result<u16, String> {
        live_run_port(&self.global_data, &self.app_id, &self.workspace_hash)
            .or_else(|| any_live_run_port(&self.global_data, &self.app_id))
            .ok_or_else(|| {
                format!(
                    "app '{}' is not running; start it in Apps first",
                    self.app_id
                )
            })
    }

    fn url(&self, port: u16) -> String {
        format!("http://127.0.0.1:{port}{}", self.path)
    }
}

/// Bridge stdio JSON-RPC to `app_id`'s MCP endpoint until stdin closes.
pub fn run(
    app_id: &str,
    path: &str,
    headers: &[String],
    timeout_secs: u64,
    workspace: Option<&Path>,
) -> Result<()> {
    validate_id(app_id).map_err(anyhow::Error::msg)?;
    if !path.starts_with('/') {
        bail!("--path must start with '/', e.g. /mcp/<endpoint>");
    }
    let headers = headers
        .iter()
        .map(|header| parse_header(header))
        .collect::<Result<Vec<_>>>()?;
    let workspace = match workspace {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir().context("resolve current directory")?,
    };
    let target = Target {
        app_id: app_id.to_string(),
        path: path.to_string(),
        headers,
        global_data: global_data_dir(),
        workspace_hash: super::registry::workspace_hash(&workspace),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("start bridge runtime")?;
    runtime.block_on(pump(target, timeout_secs.max(1)))
}

async fn pump(target: Target, timeout_secs: u64) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .context("build http client")?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    let mut session: Option<String> = None;

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            eprintln!("app-mcp: ignoring input that is not JSON");
            continue;
        };
        let id = message.get("id").cloned();
        let outcome = async {
            let port = target.port()?;
            forward(&client, &target, &target.url(port), &mut session, line).await
        };
        let payloads = match outcome.await {
            Ok(payloads) => payloads,
            Err(message) => match id {
                Some(id) => vec![json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32000, "message": message },
                })
                .to_string()],
                None => {
                    eprintln!("app-mcp: {message}");
                    Vec::new()
                }
            },
        };
        for payload in payloads {
            stdout.write_all(payload.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}

async fn forward(
    client: &reqwest::Client,
    target: &Target,
    url: &str,
    session: &mut Option<String>,
    body: &str,
) -> Result<Vec<String>, String> {
    let mut request = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json, text/event-stream")
        .body(body.to_string());
    for (name, value) in &target.headers {
        request = request.header(name.as_str(), value.as_str());
    }
    if let Some(id) = session.as_deref() {
        request = request.header("mcp-session-id", id);
    }

    let response = request.send().await.map_err(|error| {
        format!("cannot reach app '{}' at {url}: {error}", target.app_id)
    })?;
    if let Some(id) = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
    {
        *session = Some(id.to_string());
    }
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = response
        .text()
        .await
        .map_err(|error| format!("read app response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "app '{}' returned {status}: {}",
            target.app_id,
            snippet(&text)
        ));
    }
    Ok(messages(&content_type, &text))
}

/// One JSON-RPC message per line, whether the server answered with an event
/// stream or a plain JSON body.
fn messages(content_type: &str, body: &str) -> Vec<String> {
    if content_type.contains("text/event-stream") {
        return body
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(|data| data.trim().to_string())
            .filter(|data| !data.is_empty())
            .collect();
    }
    let body = body.trim();
    if body.is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<Value>(body) {
        // Re-serialize so a pretty-printed body still fits on one line.
        Ok(value) => vec![value.to_string()],
        Err(_) => vec![body.to_string()],
    }
}

fn parse_header(raw: &str) -> Result<(String, String)> {
    let (name, value) = raw
        .split_once(':')
        .with_context(|| format!("--header expects 'name: value', got '{raw}'"))?;
    let name = name.trim();
    if name.is_empty() || name.contains(char::is_whitespace) {
        bail!("--header has an invalid name: '{raw}'");
    }
    Ok((name.to_string(), value.trim().to_string()))
}

fn snippet(text: &str) -> String {
    let flat = text.trim().replace('\n', " ");
    if flat.chars().count() <= MAX_ERROR_CHARS {
        return flat;
    }
    let cut: String = flat.chars().take(MAX_ERROR_CHARS).collect();
    format!("{cut}...")
}

/// Global data root, the same directory the app service keys its markers by.
fn global_data_dir() -> PathBuf {
    crate::config::global_db::default_db_path()
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_keeps_the_first_colon_in_the_value() {
        let (name, value) = parse_header(" authorization: Bearer a:b ").expect("header");
        assert_eq!(name, "authorization");
        assert_eq!(value, "Bearer a:b");
    }

    #[test]
    fn header_rejects_malformed_names() {
        assert!(parse_header("no-colon").is_err());
        assert!(parse_header(": value").is_err());
        assert!(parse_header("bad name: value").is_err());
    }

    #[test]
    fn event_stream_yields_one_message_per_data_line() {
        let body = "event: message\ndata: {\"id\":1}\n\nevent: message\ndata: {\"id\":2}\n\n";
        assert_eq!(messages("text/event-stream", body), vec!["{\"id\":1}", "{\"id\":2}"]);
    }

    #[test]
    fn plain_json_body_is_flattened_to_one_line() {
        let body = "{\n  \"jsonrpc\": \"2.0\",\n  \"id\": 1\n}";
        assert_eq!(
            messages("application/json", body),
            vec!["{\"jsonrpc\":\"2.0\",\"id\":1}"]
        );
    }

    #[test]
    fn empty_notification_response_produces_nothing() {
        assert!(messages("text/event-stream", "").is_empty());
        assert!(messages("application/json", "  ").is_empty());
    }

    fn target(global_data: PathBuf, app_id: &str) -> Target {
        Target {
            app_id: app_id.to_string(),
            path: "/mcp/probe".to_string(),
            headers: Vec::new(),
            global_data,
            workspace_hash: "000000000000".to_string(),
        }
    }

    #[test]
    fn a_stopped_app_reports_itself_instead_of_hanging() {
        let missing = std::env::temp_dir().join("litecode-bridge-missing");
        let error = target(missing, "nocodb").port().expect_err("no marker");
        assert!(error.contains("is not running"), "{error}");
    }

    /// Live check against a running app. Run with:
    /// `LITECODE_BRIDGE_TEST_URL=http://127.0.0.1:<port>/mcp/<endpoint>`
    /// `LITECODE_BRIDGE_TEST_TOKEN=<secret> cargo test --lib apps::mcp_bridge -- --ignored`
    #[tokio::test]
    #[ignore = "needs a live app endpoint"]
    async fn forwards_to_a_live_endpoint() {
        let url = std::env::var("LITECODE_BRIDGE_TEST_URL").expect("LITECODE_BRIDGE_TEST_URL");
        let token = std::env::var("LITECODE_BRIDGE_TEST_TOKEN").expect("LITECODE_BRIDGE_TEST_TOKEN");
        let mut target = target(PathBuf::from("."), "nocodb");
        target.headers = vec![("xc-mcp-token".to_string(), token)];
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("client");
        let mut session = None;
        let initialize = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"bridge-test","version":"0"}}}"#;
        let first = forward(&client, &target, &url, &mut session, initialize)
            .await
            .expect("initialize");
        assert_eq!(first.len(), 1, "one response per request: {first:?}");
        assert!(first[0].contains("serverInfo"), "{}", first[0]);

        let listed = forward(
            &client,
            &target,
            &url,
            &mut session,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
        )
        .await
        .expect("tools/list");
        assert_eq!(listed.len(), 1);
        assert!(listed[0].contains("tools"), "{}", listed[0]);
    }
}
