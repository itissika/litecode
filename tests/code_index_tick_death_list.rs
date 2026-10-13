//! WP-079 acceptance 4: the background code-index path may never build or cool.
//! Source scan with identifier boundaries (same approach as WP-051 C1/C2).

fn fn_body<'a>(src: &'a str, sig: &str) -> &'a str {
    let start = src.find(sig).unwrap_or_else(|| panic!("missing {sig}"));
    let open = start + src[start..].find('{').unwrap();
    let mut depth = 0usize;
    for (i, ch) in src[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &src[open..open + i + 1];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced body for {sig}");
}

fn has_ident(body: &str, ident: &str) -> bool {
    let b = body.as_bytes();
    let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut from = 0;
    while let Some(pos) = body[from..].find(ident) {
        let s = from + pos;
        let e = s + ident.len();
        let before = s == 0 || !is_id(b[s - 1]);
        let after = e >= b.len() || !is_id(b[e]);
        if before && after {
            return true;
        }
        from = e;
    }
    false
}

const ENGINES: &str = include_str!("../src/engines/mod.rs");

#[test]
fn code_index_tick_never_builds_or_cools() {
    let body = fn_body(ENGINES, "pub fn code_index_tick(");
    for banned in [
        "begin_building",
        "Warming",
        "request_refresh",
        "consume_index_work",
        "reconcile",
        "engine_index_work",
    ] {
        assert!(
            !has_ident(body, banned),
            "code_index_tick must not use `{banned}`"
        );
    }
    // Only Update work may reach the spawn; Rebuild must return early.
    assert!(has_ident(body, "RebuildPending"));
}

#[test]
fn manual_refresh_does_not_merge_plans_for_execution() {
    let body = fn_body(ENGINES, "pub fn consume_index_work(");
    assert!(
        !has_ident(body, "engine_index_work") && !has_ident(body, "merge_index_work"),
        "consume_index_work must execute code/session plans separately"
    );
    assert!(!has_ident(body, "begin_building"));
}
