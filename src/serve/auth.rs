use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::serve::state::ServeState;

/// Normalize an Origin header value to its host component (localhost, IP, or
/// hostname). Bracketed IPv6 (`http://[::1]:3000`) is unwrapped without any
/// further colon split — IPv6 addresses themselves contain colons, so the port
/// must never be split off them.
pub fn origin_host(origin: &str) -> String {
    let without_scheme = origin.split("://").nth(1).unwrap_or(origin);
    if let Some(rest) = without_scheme.strip_prefix('[') {
        // Bracketed IPv6: the host is everything inside the brackets.
        return rest.split(']').next().unwrap_or(rest).to_string();
    }
    // Hostname / IPv4: split off the port at the first ':'.
    without_scheme
        .split(':')
        .next()
        .unwrap_or(without_scheme)
        .to_string()
}

#[cfg(test)]
mod origin_host_tests {
    use super::origin_host;

    #[test]
    fn origin_host_extracts_host_for_all_forms() {
        assert_eq!(origin_host("http://localhost:3000"), "localhost");
        assert_eq!(origin_host("http://127.0.0.1:3000"), "127.0.0.1");
        assert_eq!(origin_host("http://[::1]:3000"), "::1");
        assert_eq!(origin_host("https://[2001:db8::1]:8443"), "2001:db8::1");
        assert_eq!(origin_host("[::1]:3000"), "::1");
        assert_eq!(origin_host("https://evil.example"), "evil.example");
        assert_eq!(
            origin_host("https://sub.example.com:8080"),
            "sub.example.com"
        );
    }
}

pub async fn middleware(
    State(state): State<ServeState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(expected) = &state.auth_token else {
        return next.run(request).await;
    };

    let path = request.uri().path();

    if path == "/health" {
        return next.run(request).await;
    }

    // Browser navigation and static assets load without Authorization; only API/WS require auth.
    if !path.starts_with("/api/") && path != "/ws" {
        return next.run(request).await;
    }

    // G3 (REV-1/REV-2): `/api/*` accepts Authorization: Bearer only — a query
    // token would land in access logs. `/ws` keeps the query token as its sole
    // handshake channel (native WebSocket cannot set custom headers).
    let matched = if path == "/ws" {
        extract_token(&request)
            .or_else(|| extract_query_token(&request))
            .is_some_and(|token| token == *expected)
    } else {
        extract_token(&request).is_some_and(|token| token == *expected)
    };

    if matched {
        next.run(request).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

fn extract_token(request: &Request<Body>) -> Option<String> {
    // Bearer header only (G3): never accept a query token for /api/*.
    if let Some(auth_header) = request.headers().get(axum::http::header::AUTHORIZATION)
        && let Ok(value) = auth_header.to_str()
        && let Some(token) = value.strip_prefix("Bearer ")
    {
        let token = token.trim();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    None
}

/// WS-handshake-only token source (`/ws?token=...`); never used for /api/*.
fn extract_query_token(request: &Request<Body>) -> Option<String> {
    request.uri().query().and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == "token")
            .map(|(_, value)| value.into_owned())
            .filter(|token| !token.is_empty())
    })
}
