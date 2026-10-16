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
const LSP_ENGINE: &str = include_str!("../src/engines/lsp_engine.rs");
const ENGINES: &str = include_str!("../src/engines/mod.rs");
const PATH_INDEX: &str = include_str!("../src/engines/path_index/mod.rs");
const DEPS: &str = include_str!("../src/lsp/deps.rs");

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
    let state = body
        .find("ServeState::new")
        .expect("ServeState::new");
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
fn serve_run_does_not_warm_optional_engines() {
    let body = code_only(fn_body(SERVE_RUN, "pub fn run("));
    assert!(
        !body.contains("engine_manager.reconcile("),
        "serve::run must not build the web clients before READY"
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

#[test]
fn attach_does_not_touch_legacy_text_index_inline() {
    let attach = code_only(fn_body(PATH_INDEX, "pub fn attach("));
    assert!(!has_ident(&attach, "remove_legacy_text_index"));
    let run = code_only(fn_body(PATH_INDEX, "fn run(&self)"));
    assert!(has_ident(&run, "remove_legacy_text_index"));
}

#[test]
fn lsp_warmup_runs_no_subprocess() {
    let warmup = code_only(fn_body(LSP_ENGINE, "pub fn warmup("));
    for banned in [
        "activate",
        "command_runnable_command_in",
        "ensure_probed_blocking",
    ] {
        assert!(
            !has_ident(&warmup, banned),
            "warmup must not use `{banned}`"
        );
    }
    assert!(has_ident(&warmup, "activate_deferred"));
    let deferred = code_only(fn_body(HUB, "pub fn activate_deferred("));
    for banned in [
        "probe_commands",
        "command_runnable_command_in",
        "run_version_probe",
        "resolve_server_binary",
        "Command",
    ] {
        assert!(
            !has_ident(&deferred, banned),
            "activate_deferred must stay metadata-only (`{banned}`)"
        );
    }
}

#[test]
fn first_need_paths_probe_before_spawning() {
    for sig in [
        "async fn resolve_server_key(",
        "pub async fn restart_server(",
    ] {
        let body = code_only(fn_body(HUB, sig));
        let probe = body
            .find("ensure_probed()")
            .unwrap_or_else(|| panic!("{sig} probes"));
        let spawn = body
            .find("spawn_ls(")
            .unwrap_or_else(|| panic!("{sig} spawns"));
        assert!(probe < spawn, "{sig} must probe before spawning");
    }
}

/// Server ids from `SERVER_METAS` (`id: "..."` entries in deps.rs).
fn server_ids() -> Vec<String> {
    let mut ids = Vec::new();
    for line in DEPS.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("id: \"")
            && let Some(end) = rest.find('"')
        {
            ids.push(rest[..end].to_string());
        }
    }
    assert!(!ids.is_empty(), "SERVER_METAS ids not found");
    ids
}

#[test]
fn no_language_specific_branch_added() {
    let mut bodies = vec![
        code_only(SWEEP_GATE),
        code_only(fn_body(HUB, "pub fn activate_deferred(")),
        code_only(fn_body(HUB, "pub fn ensure_probed_blocking(")),
        code_only(fn_body(HUB, "pub async fn ensure_probed(")),
        code_only(fn_body(HUB, "fn probe_input(")),
        code_only(fn_body(HUB, "fn finish_probe(")),
        code_only(fn_body(HUB, "fn run_probe(")),
        code_only(fn_body(HUB, "fn probe_commands(")),
        code_only(fn_body(ENGINES, "pub async fn ensure_lsp_probed(")),
        code_only(fn_body(ENGINES, "pub fn ensure_lsp_probed_blocking(")),
    ];
    bodies.push(code_only(fn_body(LSP_ENGINE, "pub fn warmup(")));
    for id in server_ids() {
        let quoted = format!("\"{id}\"");
        for body in &bodies {
            assert!(
                !body.contains(&quoted),
                "new startup code must not special-case `{id}`"
            );
        }
    }
}
