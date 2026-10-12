//! Wire protocol roundtrip for status bar telemetry envelopes.

use litecode::client_protocol::protocol::{JsonRpcRequestEnvelope, LogLine, ServerStats, methods};

#[test]
fn server_stats_roundtrip() {
    let stats = ServerStats {
        rss_kb: Some(577_536),
        core_rss_kb: Some(12_288),
        embed_rss_kb: Some(520_192),
        lsp_rss_kb: Some(45_056),
        ts_ms: 1_700_000_000_000,
    };
    let json = serde_json::to_string(&stats).expect("serialize");
    assert!(json.contains("embed_rss_kb"));
    let back: ServerStats = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.rss_kb, Some(577_536));
    assert_eq!(back.core_rss_kb, Some(12_288));
    assert_eq!(back.embed_rss_kb, Some(520_192));
    assert_eq!(back.lsp_rss_kb, Some(45_056));
    assert_eq!(back.ts_ms, 1_700_000_000_000);
}

#[test]
fn log_line_roundtrip() {
    let line = LogLine {
        ts_ms: 1,
        level: "INFO".into(),
        target: "litecode::optional".into(),
        message: "warmup finished".into(),
    };
    let json = serde_json::to_string(&line).expect("serialize");
    let back: LogLine = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.level, "INFO");
    assert_eq!(back.message, "warmup finished");
}

#[test]
fn log_subscribe_methods_are_jsonrpc() {
    // Table-driven: subscribe_logs / unsubscribe_logs method-name constants (T1).
    for (raw, expected) in [
        (
            r#"{"jsonrpc":"2.0","id":"1","method":"subscribe_logs","params":{}}"#,
            methods::SUBSCRIBE_LOGS,
        ),
        (
            r#"{"jsonrpc":"2.0","id":"2","method":"unsubscribe_logs","params":{}}"#,
            methods::UNSUBSCRIBE_LOGS,
        ),
    ] {
        let env: JsonRpcRequestEnvelope = serde_json::from_str(raw).expect("parse");
        assert_eq!(env.jsonrpc, "2.0");
        assert_eq!(env.method, expected, "method wire for {raw}");
    }
}
