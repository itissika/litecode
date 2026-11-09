fn main() -> anyhow::Result<()> {
    let result = litecode::cli::run();
    // Model calls run on a process-wide runtime that owns their connections.
    // Close it (bounded) so an idle pooled connection can never hold the exit.
    litecode::llm::http_runtime::shutdown();
    result
}
