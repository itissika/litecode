use std::sync::{Arc, RwLock};

use regex::Regex;
use serde_json::Value;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::types::ToolCallResult;

const MAX_OUTPUT_BYTES: usize = 50_000;

pub struct WebFetchTool {
    client: Arc<RwLock<Option<reqwest::blocking::Client>>>,
}

impl WebFetchTool {
    pub fn new(client: Arc<RwLock<Option<reqwest::blocking::Client>>>) -> Self {
        Self { client }
    }

    fn fetch(&self, url: &str) -> Result<String, String> {
        // Verify the engine is warmed before fetching (gives the "not warmed"
        // hint); the actual request uses a redirect-disabled client for SSRF.
        let _warmed = self
            .client
            .read()
            .map_err(|e| format!("webfetch engine lock: {e}"))?
            .clone()
            .ok_or_else(|| "webfetch engine not warmed".to_string())?;
        let url = url.to_string();

        std::thread::spawn(move || {
            // Per hop: resolve+validate, then pin those addrs on the client so
            // reqwest does not DNS again (closes ISSUE-021 rebinding window).
            // Redirects stay Policy::none so every Location is re-checked (G2).
            let mut current = url;
            for _ in 0..=10 {
                let parsed = url::Url::parse(&current).map_err(|e| format!("invalid url: {e}"))?;
                let pinned = resolve_and_validate_public_http_url(&parsed)?;
                let client = client_for_validated_hop(&parsed, &pinned)?;
                let response = client
                    .get(parsed.clone())
                    .send()
                    .map_err(|e| format!("request failed: {e}"))?;
                if let Some(location) = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                {
                    current = parsed
                        .join(location)
                        .map_err(|e| format!("bad redirect: {e}"))?
                        .to_string();
                    continue;
                }
                let status = response.status();
                if !status.is_success() {
                    return Err(format!("http error: {status}"));
                }
                return response.text().map_err(|e| format!("read body: {e}"));
            }
            Err("too many redirects".to_string())
        })
        .join()
        .map_err(|_| "webfetch thread panicked".to_string())?
    }
}

/// Reject non-http(s) schemes and any URL resolving to a private/loopback/
/// link-local/unspecified/benchmarking address (SSRF guard, G2).
///
/// Returns the public IPs from this resolve pass. Callers must pin them on the
/// HTTP client (`resolve_to_addrs`) so connect does not DNS again — that closes
/// the validate→request rebinding window (ISSUE-021).
fn resolve_and_validate_public_http_url(
    parsed: &url::Url,
) -> Result<Vec<std::net::IpAddr>, String> {
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("scheme '{other}' not allowed (only http/https)")),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "url has no host".to_string())?;
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
                .ok_or_else(|| "bad host".to_string())?,
        ]
    } else {
        // Resolve the hostname once here; the fetch path pins these addrs.
        use std::net::ToSocketAddrs;
        let resolved = format!("{host}:443")
            .to_socket_addrs()
            .map_err(|_| format!("could not resolve host '{host}'"))?;
        let ips: Vec<_> = resolved.map(|sa| sa.ip()).collect();
        if ips.is_empty() {
            return Err(format!("could not resolve host '{host}'"));
        }
        ips
    };
    for ip in &addrs {
        if is_non_public_ip(*ip) {
            return Err(format!("URL resolves to a non-public address ({ip})"));
        }
    }
    Ok(addrs)
}

fn validate_public_http_url(parsed: &url::Url) -> Result<(), String> {
    resolve_and_validate_public_http_url(parsed).map(|_| ())
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
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || is_ipv4_shared(ip)
        || is_ipv4_benchmarking(ip)
}

/// RFC 6598 shared address space (100.64.0.0/10) — also non-public.
fn is_ipv4_shared(ip: &std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] & 0x40) == 0x40
}

/// RFC 2544 benchmarking range 198.18.0.0/15 — non-public (often used by
/// local DNS sinkholes / network simulators).
fn is_ipv4_benchmarking(ip: &std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 198 && (octets[1] & 0xfe) == 18
}

impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "webfetch"
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

    fn execute(
        &self,
        input: Value,
        _execution: crate::tool::trait_::ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        // 2.6: the network fetch is blocking; run it in `spawn_blocking` so a slow
        // webfetch cannot stall the async executor and its timeout stays effective.
        let tool = WebFetchTool::new(Arc::clone(&self.client));
        Box::pin(async move {
            let join = tokio::task::spawn_blocking(move || tool.call_inner(input));
            match join.await {
                Ok(result) => result,
                Err(e) => ToolCallResult::error(format!("webfetch task join failed: {e}")),
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let url_str = match crate::tool::require_nonempty_string_trimmed(&input, "url") {
            Ok(s) => s,
            Err(e) => return ToolCallResult::error(e),
        };

        if url_str.parse::<url::Url>().is_err() {
            return ToolCallResult::error(format!("invalid url: {url_str}"));
        }

        let format = input["format"].as_str().unwrap_or("markdown");

        let body = match self.fetch(url_str) {
            Ok(b) => b,
            Err(e) if e.contains("not warmed") => {
                return ToolCallResult::error(format!(
                    "{e}. Enable webfetch in Settings → Engines and wait until Warm"
                ));
            }
            Err(e) => return ToolCallResult::error(e),
        };

        let formatted = match format {
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

        ToolCallResult::ok(truncated)
    }

    fn max_result_size(&self) -> usize {
        MAX_OUTPUT_BYTES
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/webfetch.md"))
    }

    fn timeout(&self) -> Option<u64> {
        Some(35)
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

    #[test]
    fn html_to_markdown_preserves_heading() {
        let html = "<h1>Title</h1><p>Body</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("# Title"));
        assert!(md.contains("Body"));
    }

    #[test]
    fn strip_html_tags_removes_markup() {
        let text = strip_html_tags("<p>hello <b>world</b></p>");
        assert_eq!(text, "hello world");
    }

    fn parse_validate(url: &str) -> Result<(), String> {
        validate_public_http_url(&url::Url::parse(url).unwrap())
    }

    #[test]
    fn ssrf_rejects_non_http_schemes() {
        assert!(parse_validate("file:///etc/passwd").is_err());
        assert!(parse_validate("ftp://example.com/x").is_err());
        assert!(parse_validate("gopher://example.com").is_err());
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
        // RFC 6598 shared space is also non-public.
        assert!(parse_validate("http://100.64.0.1/").is_err());
        // RFC 2544 benchmarking range 198.18.0.0/15 is non-public.
        assert!(parse_validate("http://198.18.0.1/").is_err());
        assert!(parse_validate("http://198.19.255.255/").is_err());
        // IPv6 unique-local is non-public.
        assert!(parse_validate("http://[fd00::1]/").is_err());
        // IPv6 link-local fe80::/10 is non-public.
        assert!(parse_validate("http://[fe80::1]/").is_err());
        // IPv4-mapped IPv6 must judge the embedded IPv4 (ISSUE-012).
        assert!(parse_validate("http://[::ffff:127.0.0.1]/").is_err());
        assert!(parse_validate("http://[::ffff:10.0.0.1]/").is_err());
        assert!(parse_validate("http://[::ffff:192.168.0.1]/").is_err());
        assert!(parse_validate("http://[::ffff:198.18.0.1]/").is_err());
    }

    #[test]
    fn ssrf_allows_public_http_urls() {
        // A public IP is accepted without needing DNS resolution.
        assert!(parse_validate("http://8.8.8.8/").is_ok());
        assert!(parse_validate("http://[2001:4860:4860::8888]/").is_ok());
        // IPv4-mapped public address is still public.
        assert!(parse_validate("http://[::ffff:8.8.8.8]/").is_ok());
    }


    #[test]
    fn resolve_returns_literal_public_ips() {
        let ips =
            resolve_and_validate_public_http_url(&url::Url::parse("http://8.8.8.8/").unwrap())
                .unwrap();
        assert_eq!(ips, vec!["8.8.8.8".parse::<std::net::IpAddr>().unwrap()]);
    }

    #[test]
    fn client_pins_hostname_to_validated_public_ips() {
        // Smoke: builder accepts resolve_to_addrs with port 0 (URL port wins).
        let parsed = url::Url::parse("https://example.invalid/path").unwrap();
        let ips = vec!["93.184.216.34".parse::<std::net::IpAddr>().unwrap()];
        let client = client_for_validated_hop(&parsed, &ips).expect("pinned client");
        // Client is built; connect would use the pin (no second DNS for host).
        let _ = client;
    }

    #[test]
    fn client_for_literal_ip_needs_no_pin() {
        let parsed = url::Url::parse("http://1.1.1.1/").unwrap();
        let ips = resolve_and_validate_public_http_url(&parsed).unwrap();
        assert!(client_for_validated_hop(&parsed, &ips).is_ok());
    }
}
