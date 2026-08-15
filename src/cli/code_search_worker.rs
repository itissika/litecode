//! Hidden subcommand: `litecode code-search-worker` — JSON-RPC stdin/stdout loop.

pub fn run() -> anyhow::Result<()> {
    crate::engines::code_search_ipc::server::run_worker_loop()?;
    std::process::exit(0);
}
