//! Parent-process client: spawn worker, JSON-RPC over newline-delimited stdin/stdout.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use super::protocol::{
    InitializeParams, JsonRpcRequest, JsonRpcResponse, NotifyFsChangesParams, RefreshResult,
    SearchParams, SearchResult, SessionSearchParams, SessionSearchResult,
};
use crate::engines::code_search::SearchHit;
use crate::engines::session_search::SessionTextHit;
use crate::types::{LitecodeError, Result};

pub struct CodeSearchWorkerClient {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    next_id: AtomicU64,
}

fn worker_binary() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_litecode") {
        return Ok(PathBuf::from(path));
    }
    std::env::current_exe()
        .map_err(|e| LitecodeError::ToolExecution(format!("code_search worker: current_exe: {e}")))
}

impl CodeSearchWorkerClient {
    pub fn spawn() -> Result<Self> {
        let exe = worker_binary()?;
        let mut child = Command::new(exe)
            .arg("code-search-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                LitecodeError::ToolExecution(format!("code_search worker spawn failed: {e}"))
            })?;

        let stdin = child.stdin.take().expect("worker stdin piped");
        let stdout = child.stdout.take().expect("worker stdout piped");
        let reader = BufReader::new(stdout);

        Ok(Self {
            child,
            stdin,
            reader,
            next_id: AtomicU64::new(1),
        })
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        };
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.flush()?;

        let mut response_line = String::new();
        let n = self.reader.read_line(&mut response_line)?;
        if n == 0 {
            return Err(LitecodeError::ToolExecution(
                "code_search worker closed stdout".into(),
            ));
        }

        let resp: JsonRpcResponse = serde_json::from_str(response_line.trim())?;
        if resp.id != id {
            return Err(LitecodeError::ToolExecution(format!(
                "code_search worker response id mismatch: expected {id}, got {}",
                resp.id
            )));
        }
        if let Some(err) = resp.error {
            return Err(LitecodeError::ToolExecution(format!(
                "code_search worker error ({}): {}",
                err.code, err.message
            )));
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    pub fn initialize(&mut self, workspace_root: &std::path::Path) -> Result<()> {
        let params = serde_json::to_value(InitializeParams {
            workspace_root: workspace_root.to_string_lossy().into_owned(),
        })?;
        self.request("initialize", params)?;
        Ok(())
    }

    pub fn warmup(&mut self) -> Result<()> {
        self.request("warmup", serde_json::json!({}))?;
        Ok(())
    }

    pub fn ping(&mut self) -> Result<()> {
        self.request("ping", serde_json::json!({}))?;
        Ok(())
    }

    pub fn search(
        &mut self,
        query: &str,
        glob_filter: Option<&str>,
        top_k: usize,
    ) -> Result<Vec<SearchHit>> {
        let params = serde_json::to_value(SearchParams {
            query: query.into(),
            glob: glob_filter.map(str::to_string),
            top_k,
        })?;
        let result = self.request("search", params)?;
        let parsed: SearchResult = serde_json::from_value(result)?;
        Ok(parsed.hits)
    }

    pub fn session_search(
        &mut self,
        query: &str,
        top_k: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<SessionTextHit>> {
        let params = serde_json::to_value(SessionSearchParams {
            query: query.into(),
            top_k,
            session_id: session_id.map(str::to_string),
        })?;
        let result = self.request("session_search", params)?;
        let parsed: SessionSearchResult = serde_json::from_value(result)?;
        Ok(parsed.hits)
    }

    pub fn refresh(&mut self) -> Result<RefreshResult> {
        let result = self.request("refresh", serde_json::json!({}))?;
        let parsed: RefreshResult = serde_json::from_value(result)?;
        Ok(parsed)
    }

    /// Forward workspace FS changes for Index dirty queue (serve is sole OS watcher).
    pub fn notify_fs_changes(&mut self, paths: &[String], deleted: bool) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let params = serde_json::to_value(NotifyFsChangesParams {
            paths: paths.to_vec(),
            deleted,
        })?;
        self.request("notify_fs_changes", params)?;
        Ok(())
    }

    /// Force disk↔index dirty reconcile (used after broadcast lag).
    pub fn reconcile_disk(&mut self) -> Result<()> {
        self.request("reconcile_disk", serde_json::json!({}))?;
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<()> {
        let _ = self.request("shutdown", serde_json::json!({}));
        Ok(())
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    pub fn pid(&self) -> Option<u32> {
        Some(self.child.id())
    }
}

impl Drop for CodeSearchWorkerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
