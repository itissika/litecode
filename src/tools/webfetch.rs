use std::sync::{Arc, RwLock};

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::outcome::ToolOutcome;
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

const MAX_OUTPUT_BYTES: usize = 50_000;

pub struct WebFetchTool {
    client: Arc<RwLock<Option<reqwest::blocking::Client>>>,
    /// WP-167: builds the client on first use (startup no longer does).
    warm: Option<crate::optional::WarmHook>,
}

impl WebFetchTool {
    pub fn new(client: Arc<RwLock<Option<reqwest::blocking::Client>>>) -> Self {
        Self { client, warm: None }
    }

    pub fn with_warm_hook(mut self, warm: crate::optional::WarmHook) -> Self {
        self.warm = Some(warm);
        self
    }

    fn fetch_outcome(&self, url: &str) -> FetchOutcome {
        // Verify the engine is warmed before fetching (gives the "not warmed"
        // hint); the actual request uses a redirect-disabled client for SSRF.
        // WP-167: one ensure_warm per call (builds on first use, reuses after,
        // retries after a failed build). A failed build is returned as is.
        if let Some(warm) = &self.warm
            && let Err(e) = warm()
        {
            return FetchOutcome::Hard {
                code: "unavailable",
                message: format!(
                    "webfetch engine failed to start: {e}. Enable webfetch in Settings → Engines and wait until Warm"
                ),
            };
        }
        let warmed = match self.client.read() {
            Ok(guard) => guard.clone(),
            Err(e) => {
                return FetchOutcome::Hard {
                    code: "unavailable",
                    message: format!("webfetch engine lock: {e}"),
                };
            }
        };
        if warmed.is_none() {
            return FetchOutcome::Hard {
                code: "unavailable",
                message: "webfetch engine not warmed. Enable webfetch in Settings → Engines and wait until Warm"
                    .to_string(),
            };
        }
        let url = url.to_string();

        match std::thread::spawn(move || fetch_hops(url)).join() {
            Ok(outcome) => outcome,
            Err(_) => FetchOutcome::Hard {
                code: "internal",
                message: "webfetch thread panicked".to_string(),
            },
        }
    }
}

#[derive(Debug)]
enum FetchOutcome {
    Ok {
        url: String,
        status: u16,
        body: String,
    },
    Soft {
        url: String,
        status: Option<u16>,
        message: String,
    },
    Hard {
        code: &'static str,
        message: String,
    },
}

fn fetch_hops(start_url: String) -> FetchOutcome {
    // Per hop: resolve+validate, then pin those addrs on the client so
    // reqwest does not DNS again (closes ISSUE-021 rebinding window).
    // Redirects stay Policy::none so every Location is re-checked (G2).
    let mut current = start_url;
    for _ in 0..=10 {
        let parsed = match url::Url::parse(&current) {
            Ok(u) => u,
            Err(e) => {
                return FetchOutcome::Hard {
                    code: "invalid_input",
                    message: format!("invalid url: {e}"),
                };
            }
        };
        let pinned = match resolve_and_validate_public_http_url(&parsed) {
            Ok(ips) => ips,
            Err(GateError::Hard { code, message }) => {
                return FetchOutcome::Hard { code, message };
            }
            Err(GateError::Soft { message }) => {
                return FetchOutcome::Soft {
                    url: current,
                    status: None,
                    message,
                };
            }
        };
        let client = match client_for_validated_hop(&parsed, &pinned) {
            Ok(c) => c,
            Err(message) => {
                return FetchOutcome::Soft {
                    url: current,
                    status: None,
                    message,
                };
            }
        };
        let response = match client.get(parsed.clone()).send() {
            Ok(r) => r,
            Err(e) => {
                return FetchOutcome::Soft {
                    url: current,
                    status: None,
                    message: format!("request failed: {e}"),
                };
            }
        };
        if let Some(location) = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
        {
            current = match parsed.join(location) {
                Ok(next) => next.to_string(),
                Err(e) => {
                    return FetchOutcome::Soft {
                        url: current,
                        status: Some(response.status().as_u16()),
                        message: format!("bad redirect: {e}"),
                    };
                }
            };
            continue;
        }
        let status = response.status();
        let status_code = status.as_u16();
        if !status.is_success() {
            return FetchOutcome::Soft {
                url: current,
                status: Some(status_code),
                message: format!("http error: {status}"),
            };
        }
        return match response.text() {
            Ok(body) => FetchOutcome::Ok {
                url: current,
                status: status_code,
                body,
            },
            Err(e) => FetchOutcome::Soft {
                url: current,
                status: Some(status_code),
                message: format!("read body: {e}"),
            },
        };
    }
    FetchOutcome::Soft {
        url: current,
        status: None,
        message: "too many redirects".to_string(),
    }
}

#[derive(Debug)]
enum GateError {
    Hard {
        code: &'static str,
        message: String,
    },
    Soft {
        message: String,
    },
}

/// Reject non-http(s) schemes and any URL resolving to a private/loopback/
/// link-local/unspecified/CGNAT address (SSRF guard, G2).
///
/// WP-203: `198.18.0.0/15` (RFC 2544 benchmarking / common TUN fake-ip) is
/// **allowed** through this gate so hostname fetch works under fake-ip TUN.
/// Connection still pins the resolved IPs (ISSUE-021).
///
/// Returns the public (gate-allowed) IPs from this resolve pass. Callers must
/// pin them on the HTTP client (`resolve_to_addrs`) so connect does not DNS
/// again — that closes the validate→request rebinding window (ISSUE-021).
fn resolve_and_validate_public_http_url(
    parsed: &url::Url,
) -> Result<Vec<std::net::IpAddr>, GateError> {
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(GateError::Hard {
                code: "invalid_input",
                message: format!("scheme '{other}' not allowed (only http/https)"),
            });
        }
    }
    let host = parsed.host_str().ok_or_else(|| GateError::Hard {
        code: "invalid_input",
        message: "url has no host".to_string(),
    })?;
    let is_ip = parsed
        .host()
        .is_some_and(|h| matches!(h, url::Host::Ipv4(_) | url::Host::Ipv6(_)));
    let addrs: Vec<std::net::IpAddr> = if is_ip {
        vec![
            parsed
                .host()
                .and_then(|h| match h {
                    url::Host::Ipv4(ip) => Some(std::net::IpAddr::V4(ip)),
                    url::Host::Ipv6(ip) => Some(std::net::IpAddr::V6(ip)),
                    _ => None,
                })
                .ok_or_else(|| GateError::Hard {
                    code: "invalid_input",
                    message: "bad host".to_string(),
                })?,
        ]
    } else {
        // Resolve the hostname once here; the fetch path pins these addrs.
        use std::net::ToSocketAddrs;
        let resolved = format!("{host}:443")
            .to_socket_addrs()
            .map_err(|_| GateError::Soft {
                message: format!("could not resolve host '{host}'"),
            })?;
        let ips: Vec<_> = resolved.map(|sa| sa.ip()).collect();
        if ips.is_empty() {
            return Err(GateError::Soft {
                message: format!("could not resolve host '{host}'"),
            });
        }
        ips
    };
    for ip in &addrs {
        if is_non_public_ip(*ip) {
            return Err(GateError::Hard {
                code: "permission_denied",
                message: format!("URL resolves to a non-public address ({ip})"),
            });
        }
    }
    Ok(addrs)
}

fn validate_public_http_url(parsed: &url::Url) -> Result<(), String> {
    resolve_and_validate_public_http_url(parsed).map(|_| ()).map_err(|e| match e {
        GateError::Hard { message, .. } | GateError::Soft { message } => message,
    })
}

/// Build a redirect-disabled client that connects only to already-validated IPs.
/// Hostname URLs keep the original URL for Host/SNI/certs; literal IP URLs need
/// no pin. Port `0` lets reqwest use the URL/scheme port.
fn client_for_validated_hop(
    parsed: &url::Url,
    public_ips: &[std::net::IpAddr],
) -> Result<reqwest::blocking::Client, String> {
    use std::net::SocketAddr;

    let mut builder = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none());

    let is_ip = parsed
        .host()
        .is_some_and(|h| matches!(h, url::Host::Ipv4(_) | url::Host::Ipv6(_)));
    if !is_ip {
        let host = parsed
            .host_str()
            .ok_or_else(|| "url has no host".to_string())?;
        let pinned: Vec<SocketAddr> = public_ips
            .iter()
            .copied()
            .map(|ip| SocketAddr::new(ip, 0))
            .collect();
        if pinned.is_empty() {
            return Err(format!("no public addresses to pin for host '{host}'"));
        }
        builder = builder.resolve_to_addrs(host, &pinned);
    }

    builder.build().map_err(|e| format!("client build: {e}"))
}

fn is_non_public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => is_non_public_ipv4(&v4),
        std::net::IpAddr::V6(v6) => {
            if v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
            {
                return true;
            }
            // IPv4-mapped IPv6 (::ffff:a.b.c.d): judge the embedded IPv4.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_non_public_ipv4(&v4);
            }
            false
        }
    }
}

fn is_non_public_ipv4(ip: &std::net::Ipv4Addr) -> bool {
    // WP-203: do NOT treat RFC 2544 benchmarking 198.18.0.0/15 as non-public —
    // common TUN fake-ip sinks resolve public hostnames there. Still reject
    // RFC1918 / loopback / link-local / unspecified / CGNAT 100.64/10.
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || is_ipv4_shared(ip)
}

/// RFC 6598 shared address space (100.64.0.0/10) — also non-public.
fn is_ipv4_shared(ip: &std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] & 0xc0) == 0x40
}

/// RFC 2544 benchmarking range 198.18.0.0/15 — used by many TUN fake-ip stacks.
/// Kept for tests; **not** part of the SSRF non-public gate (WP-203).
#[cfg(test)]
fn is_ipv4_benchmarking(ip: &std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 198 && (octets[1] & 0xfe) == 18
}

#[derive(Debug, Clone, Serialize)]
struct WebFetchData {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    message: String,
}

fn webfetch_data_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "description": "webfetch outcome. Soft failures (HTTP/DNS/timeout/body) resolve with ok=false; hard failures reject ToolError.",
        "properties": {
            "ok": { "type": "boolean" },
            "url": { "type": "string" },
            "status": { "type": "integer" },
            "format": { "type": "string", "enum": ["text", "markdown", "html"] },
            "body": { "type": "string" },
            "message": { "type": "string" }
        },
        "required": ["ok", "message"]
    })
}

fn present_webfetch(data: WebFetchData) -> ToolCallResult {
    // Code surface: facts live in data; content is not the contract.
    ToolOutcome::ok(data).into_call_result("")
}

impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "webfetch"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from"
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "markdown", "html"],
                    "description": "Desired output format (default: markdown)"
                }
            },
            "required": ["url"]
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(webfetch_data_schema())
    }

    fn execute(
        &self,
        input: Value,
        _execution: crate::tool::trait_::ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        // 2.6: the network fetch is blocking; run it in `spawn_blocking` so a slow
        // webfetch cannot stall the async executor and its timeout stays effective.
        let tool = WebFetchTool {
            client: Arc::clone(&self.client),
            warm: self.warm.clone(),
        };
        Box::pin(async move {
            let join = tokio::task::spawn_blocking(move || tool.call_inner(input));
            match join.await {
                Ok(result) => result,
                Err(e) => coded_error("internal", format!("webfetch task join failed: {e}")),
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let url_str = match crate::tool::require_nonempty_string_trimmed(&input, "url") {
            Ok(s) => s.to_string(),
            Err(e) => return coded_error("invalid_input", e),
        };

        if url_str.parse::<url::Url>().is_err() {
            return coded_error("invalid_input", format!("invalid url: {url_str}"));
        }

        let format = input["format"].as_str().unwrap_or("markdown").to_string();
        if !matches!(format.as_str(), "text" | "markdown" | "html") {
            return coded_error(
                "invalid_input",
                format!("format must be text, markdown, or html (got '{format}')"),
            );
        }

        match self.fetch_outcome(&url_str) {
            FetchOutcome::Ok {
                url,
                status,
                body,
            } => {
                let formatted = match format.as_str() {
                    "html" => body,
                    "text" => strip_html_tags(&body),
                    _ => html_to_markdown(&body),
                };
                let truncated = if formatted.len() > MAX_OUTPUT_BYTES {
                    format!(
                        "{}... [truncated {} bytes]",
                        &formatted[..MAX_OUTPUT_BYTES],
                        formatted.len() - MAX_OUTPUT_BYTES
                    )
                } else {
                    formatted
                };
                present_webfetch(WebFetchData {
                    ok: true,
                    url: Some(url),
                    status: Some(status),
                    format: Some(format),
                    body: Some(truncated),
                    message: "Fetched.".to_string(),
                })
            }
            FetchOutcome::Soft {
                url,
                status,
                message,
            } => present_webfetch(WebFetchData {
                ok: false,
                url: Some(url),
                status,
                format: Some(format),
                body: None,
                message,
            }),
            FetchOutcome::Hard { code, message } => coded_error(code, message),
        }
    }

    fn max_result_size(&self) -> usize {
        MAX_OUTPUT_BYTES
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/webfetch.md"))
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(35))
    }
}

fn strip_html_tags(html: &str) -> String {
    let re = Regex::new("<[^>]*>").expect("valid regex for HTML tag stripping");
    let text = re.replace_all(html, "").to_string();
    collapse_whitespace(&text)
}

fn html_to_markdown(html: &str) -> String {
    let mut text = html.to_string();

    for level in 1..=6 {
        let tag = format!("h{level}");
        let re = Regex::new(&format!(r"(?i)<{tag}[^>]*>(.*?)</{tag}>")).expect("valid regex");
        let hashes = "#".repeat(level);
        text = re
            .replace_all(&text, |caps: &regex::Captures| {
                format!("{hashes} {}", caps[1].trim())
            })
            .to_string();
    }

    let li_re = Regex::new(r"(?i)<li[^>]*>").expect("valid regex");
    text = li_re.replace_all(&text, "- ").to_string();

    let br_re = Regex::new(r"(?i)<br\s*/?>").expect("valid regex");
    text = br_re.replace_all(&text, "\n").to_string();

    let p_re = Regex::new(r"(?i)</p>").expect("valid regex");
    text = p_re.replace_all(&text, "\n\n").to_string();

    let tag_re = Regex::new("<[^>]*>").expect("valid regex");
    text = tag_re.replace_all(&text, "").to_string();

    text = text.replace("&amp;", "&");
    text = text.replace("&lt;", "<");
    text = text.replace("&gt;", ">");
    text = text.replace("&quot;", "\"");
    text = text.replace("&#39;", "'");
    text = text.replace("&nbsp;", " ");

    collapse_whitespace(&text)
}

fn collapse_whitespace(text: &str) -> String {
    let lines: Vec<String> = text.lines().map(|l| l.trim_end().to_string()).collect();
    let mut result = String::new();
    let mut blank_count = 0usize;
    for line in &lines {
        if line.is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            result.push_str(line);
            result.push('\n');
        }
    }
    result.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolSignalLevel;

    fn parse_validate(url: &str) -> Result<(), String> {
        validate_public_http_url(&url::Url::parse(url).unwrap())
    }

    #[test]
    fn ssrf_rejects_loopback_and_private_ips() {
        // Loopback / unspecified / link-local are non-public and rejected.
        assert!(parse_validate("http://127.0.0.1:8080/admin").is_err());
        assert!(parse_validate("http://[::1]/").is_err());
        assert!(parse_validate("http://0.0.0.0/").is_err());
        assert!(parse_validate("http://169.254.169.254/latest/meta-data").is_err());
        // RFC 1918 private ranges are non-public.
        assert!(parse_validate("http://10.0.0.5/").is_err());
        assert!(parse_validate("http://192.168.1.1/").is_err());
        assert!(parse_validate("http://172.16.0.1/").is_err());
        // RFC 6598 shared space is also non-public (WP-203: still rejected).
        assert!(parse_validate("http://100.64.0.1/").is_err());
        assert!(parse_validate("http://100.127.255.255/").is_err());
        // IPv6 unique-local is non-public.
        assert!(parse_validate("http://[fd00::1]/").is_err());
        // IPv6 link-local fe80::/10 is non-public.
        assert!(parse_validate("http://[fe80::1]/").is_err());
        // IPv4-mapped IPv6 must judge the embedded IPv4 (ISSUE-012).
        assert!(parse_validate("http://[::ffff:127.0.0.1]/").is_err());
        assert!(parse_validate("http://[::ffff:10.0.0.1]/").is_err());
        assert!(parse_validate("http://[::ffff:192.168.0.1]/").is_err());
        assert!(parse_validate("http://[::ffff:100.64.0.1]/").is_err());
    }

    #[test]
    fn ssrf_allows_benchmarking_fake_ip_range() {
        // WP-203: 198.18.0.0/15 is gate-allowed (TUN fake-ip); still pin on connect.
        assert!(
            parse_validate("http://198.18.0.1/").is_ok(),
            "198.18.0.1 must pass SSRF gate for TUN fake-ip"
        );
        assert!(parse_validate("http://198.19.255.255/").is_ok());
        assert!(parse_validate("http://[::ffff:198.18.0.1]/").is_ok());
        // Classifier helper still identifies the range; gate does not use it.
        assert!(is_ipv4_benchmarking(&"198.18.0.1".parse().unwrap()));
        assert!(!is_non_public_ip("198.18.0.1".parse().unwrap()));
        assert!(!is_non_public_ip("198.19.1.2".parse().unwrap()));
    }

    #[test]
    fn non_http_scheme_is_hard_invalid_input() {
        let tool = WebFetchTool::new(Arc::new(RwLock::new(Some(reqwest::blocking::Client::new()))));
        let result = tool.call_inner(serde_json::json!({ "url": "file:///etc/passwd" }));
        assert_eq!(result.level, ToolSignalLevel::Error);
        assert_eq!(result.error_code.as_deref(), Some("invalid_input"));
        assert!(result.content.contains("scheme") || result.content.contains("not allowed"));
    }

    #[test]
    fn private_ip_is_hard_permission_denied() {
        let tool = WebFetchTool::new(Arc::new(RwLock::new(Some(reqwest::blocking::Client::new()))));
        let result = tool.call_inner(serde_json::json!({ "url": "http://127.0.0.1/" }));
        assert_eq!(result.level, ToolSignalLevel::Error);
        assert_eq!(result.error_code.as_deref(), Some("permission_denied"));
        let data_ok = result.data.as_ref().and_then(|d| d.get("ok")).and_then(|v| v.as_bool());
        assert_ne!(data_ok, Some(false), "SSRF must be ToolError, not ok:false");
    }

    #[test]
    fn gate_hard_ssrf_uses_permission_denied_code() {
        let err = resolve_and_validate_public_http_url(
            &url::Url::parse("http://10.0.0.1/").unwrap(),
        )
        .unwrap_err();
        match err {
            GateError::Hard { code, .. } => assert_eq!(code, "permission_denied"),
            GateError::Soft { message } => panic!("expected hard SSRF, got soft: {message}"),
        }
    }

}
