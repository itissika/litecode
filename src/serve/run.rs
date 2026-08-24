use std::net::SocketAddr;
use std::sync::Arc;

use crate::config::{ResolvedConfig, SettingsWriter, TurnGuard, WorkspaceState};
use crate::engines::WorkspaceEngines;
use crate::optional::EngineManager;
use crate::serve::router;
use crate::serve::state::ServeState;
use crate::serve::web_dist;

pub struct ServeOptions {
    pub bind: String,
    pub loopback_only: bool,
    pub require_auth: bool,
    pub parent_pid: Option<u32>,
    pub shutdown_on_stdin_eof: bool,
}

/// Resolve auth token from `LITECODE_TOKEN` env only (non-empty).
fn resolve_auth_token() -> Option<String> {
    std::env::var("LITECODE_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Bind / auth policy for `serve`:
/// - `--loopback-only` rejects non-loopback binds.
/// - Non-loopback binds require `--require-auth` (and a non-empty token).
/// - `--require-auth` alone does **not** imply loopback-only.
pub fn validate_serve_bind(
    addr: SocketAddr,
    loopback_only: bool,
    require_auth: bool,
    has_token: bool,
) -> anyhow::Result<()> {
    if loopback_only && !addr.ip().is_loopback() {
        anyhow::bail!("bind address {addr} is not loopback (loopback-only mode enabled)");
    }
    if !addr.ip().is_loopback() && !require_auth {
        anyhow::bail!(
            "non-loopback bind {addr} requires --require-auth (and a non-empty LITECODE_TOKEN)"
        );
    }
    if require_auth && !has_token {
        anyhow::bail!("--require-auth is set but LITECODE_TOKEN is missing or empty");
    }
    Ok(())
}

pub fn run(
    resolved: ResolvedConfig,
    agent_name: String,
    workspace: WorkspaceState,
    session_id: Option<String>,
    options: ServeOptions,
) -> anyhow::Result<()> {
    let addr: SocketAddr = options
        .bind
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid bind {}: {e}", options.bind))?;

    let auth_token = resolve_auth_token();
    validate_serve_bind(
        addr,
        options.loopback_only,
        options.require_auth,
        auth_token.is_some(),
    )?;

    let web_dist = web_dist::resolve_web_dist()?;
    tracing::info!("web dist: {}", web_dist.display());

    let turn_guard = Arc::new(TurnGuard::new());
    let mut settings_writer = SettingsWriter::new(turn_guard.clone());
    let engine_manager = Arc::new(EngineManager::new());
    let workspace_engines = Arc::new(WorkspaceEngines::new());
    settings_writer.set_engine_manager(Arc::clone(&engine_manager));
    let settings_writer = Arc::new(settings_writer);

    engine_manager.reconcile(&resolved);
    workspace_engines.reconcile(&resolved);

    let state = ServeState::new(
        resolved,
        agent_name,
        workspace,
        engine_manager,
        workspace_engines,
        session_id,
        auth_token,
        turn_guard,
        settings_writer,
    )?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(router::listen(
        state,
        addr,
        web_dist,
        crate::serve::shutdown::ShutdownWatch {
            parent_pid: options.parent_pid,
            stdin_eof: options.shutdown_on_stdin_eof,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn loopback(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    fn all_interfaces(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)
    }

    #[test]
    fn case_a_loopback_require_auth_with_token_ok() {
        assert!(validate_serve_bind(loopback(7483), false, true, true).is_ok());
    }

    #[test]
    fn case_b_non_loopback_require_auth_with_token_ok() {
        assert!(validate_serve_bind(all_interfaces(7483), false, true, true).is_ok());
    }

    #[test]
    fn case_c_non_loopback_without_auth_fails() {
        let err = validate_serve_bind(all_interfaces(7483), false, false, false)
            .expect_err("must reject non-loopback without auth");
        assert!(err.to_string().contains("require-auth"));
    }

    #[test]
    fn case_d_require_auth_without_token_fails() {
        let err = validate_serve_bind(loopback(7483), false, true, false)
            .expect_err("must reject require-auth without token");
        assert!(err.to_string().contains("LITECODE_TOKEN"));
    }

    #[test]
    fn loopback_only_rejects_non_loopback_even_with_auth() {
        let err = validate_serve_bind(all_interfaces(7483), true, true, true)
            .expect_err("loopback-only must still reject 0.0.0.0");
        assert!(err.to_string().contains("loopback"));
    }

    #[test]
    fn loopback_without_auth_still_ok() {
        assert!(validate_serve_bind(loopback(0), false, false, false).is_ok());
    }
}
