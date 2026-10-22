//! WP-167: startup-order acceptance by source scan (identifier boundaries,
//! same approach as WP-051 C1/C2 and `code_index_tick_death_list.rs`).
//!
//! - The orphan sweep, optional-engine warmup and legacy text-index removal
//!   no longer run inline on the serve startup path.
//! - LSP warmup starts no subprocess; the availability probe runs on first need.
//! - Every language-server spawn waits for the sweep gate.
//! - The new code has no language-specific branch.
//!
//! WP-165 lands WP-167's cross-domain session.diff in `serve::run`
//! (background sweep + `defer_reconcile`). The two `serve_run_*` tests scan
//! `src/serve/run.rs` as changed by WP-167; WP-166's snapshot warm lives in
//! `cli.rs`/`router.rs`/`state.rs` and does not touch these checks.

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

/// Drop `//` comments so prose cannot satisfy or trip a check.
fn code_only(body: &str) -> String {
    body.lines()
        .map(|line| match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const SERVE_RUN: &str = include_str!("../src/serve/run.rs");
const HUB: &str = include_str!("../src/lsp/hub.rs");
const SWEEP_GATE: &str = include_str!("../src/lsp/sweep_gate.rs");

#[test]
fn serve_run_does_not_sweep_inline() {
    let body = code_only(fn_body(SERVE_RUN, "pub fn run("));
    assert!(
        !body.contains("sweep_orphan_lsp_processes()"),
        "serve::run must not run the orphan sweep inline"
    );
    assert!(
        has_ident(&body, "start_background"),
        "serve::run must start the sweep through the gate"
    );
    // WP-165: gate must be armed before ServeState / LspHub exist, otherwise
    // a concurrent first-need spawn could race an unarmed gate.
    let start = body
        .find("start_background")
        .expect("start_background call");
    let engines = body
        .find("WorkspaceEngines::new")
        .expect("WorkspaceEngines::new creates LspHub");
    let state = body.find("ServeState::new").expect("ServeState::new");
    assert!(
        start < engines && start < state,
        "start_background must precede ServeState and LspHub construction"
    );
}

#[test]
fn serve_run_arms_sweep_before_ready_path() {
    // Source-level: READY is printed in router::listen after ServeState is
    // built; the only call site that arms the gate is serve::run, and it
    // must not await the sweep (no wait / join / block_on around it).
    let body = code_only(fn_body(SERVE_RUN, "pub fn run("));
    let start = body.find("start_background").expect("armed");
    let listen = body.find("router::listen").expect("listen");
    assert!(start < listen, "sweep is armed before listen/READY");
    // No synchronous wait on the gate in serve::run (spawn_ls waits instead).
    assert!(
        !body.contains("sweep_gate.wait()") && !has_ident(&body, "join"),
        "serve::run must not wait for the sweep before READY"
    );
}

#[test]
fn every_spawn_waits_for_the_sweep_gate() {
    let body = code_only(fn_body(HUB, "async fn spawn_ls("));
    let wait = body
        .find("sweep_gate.wait()")
        .expect("spawn_ls awaits the gate");
    let spawn = body.find("LspServer::spawn").expect("spawn_ls spawns");
    assert!(wait < spawn, "the gate is awaited before the spawn");
    // spawn_ls is the only spawn point.
    assert_eq!(HUB.matches("LspServer::spawn(").count(), 1);
}

#[test]
fn sweep_gate_has_no_timeout() {
    let code = code_only(SWEEP_GATE);
    for banned in ["timeout", "Duration", "Instant"] {
        let outside_tests = code.split("#[cfg(test)]").next().unwrap();
        assert!(
            !has_ident(outside_tests, banned),
            "the sweep gate must not open on a timer (`{banned}`)"
        );
    }
}

