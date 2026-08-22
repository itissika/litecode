use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;

use futures_util::FutureExt;
use tokio_util::sync::CancellationToken;

use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, InputContent, InputFileContent,
    InputImageContent, InputTextContent,
};
use crate::context_pipeline::Context;
use crate::permission::{PermissionEngine, PermissionSink};
use crate::session::media::resolve_media_artifact_url;
use crate::session::store::Session;
use crate::tool::authorize::{AuthResult, authorize};
use crate::tool::output;
use crate::tool::schema_validate::{
    check_tool_input, invalid_input_for, parse_tool_arguments, unknown_top_level_properties,
};
use crate::tool::trait_::{Tool, ToolExecutionContext};
use crate::tool::write_lock::{ResourceKey, WorkspaceWriteLock};
use crate::types::{
    FunctionToolCall, Item, MediaArtifact, MediaKind, Result, ToolCallResult, ToolOutputPart,
    ToolSignalLevel,
};

#[derive(Debug)]
pub(crate) struct Batch {
    pub is_concurrency_safe: bool,
    pub blocks: Vec<FunctionToolCall>,
}

pub(crate) fn partition_tool_calls(
    tool_uses: &[FunctionToolCall],
    tools: &[Arc<dyn Tool>],
    workspace_root: &Path,
    path_mode_for: impl Fn(&str) -> crate::workspace::ToolPathMode,
) -> Vec<Batch> {
    let mut batches: Vec<Batch> = Vec::new();

    for tu in tool_uses {
        let tu_input = serde_json::from_str(&tu.arguments).unwrap_or(serde_json::Value::Null);
        let tool = tools.iter().find(|t| t.name() == tu.name);
        let is_safe = tool
            .map(|t| t.is_concurrency_safe(&tu_input))
            .unwrap_or(false);
        let keys = tool
            .map(|t| t.resource_keys(&tu_input, path_mode_for(&tu.name), workspace_root))
            .unwrap_or_default();

        if is_safe
            && batches.last().is_some_and(|b| {
                b.is_concurrency_safe
                    && !batch_conflicts_with(b, &keys, tools, workspace_root, &path_mode_for)
            })
        {
            batches.last_mut().unwrap().blocks.push(tu.clone());
        } else {
            batches.push(Batch {
                is_concurrency_safe: is_safe,
                blocks: vec![tu.clone()],
            });
        }
    }

    batches
}

fn batch_conflicts_with(
    batch: &Batch,
    keys: &[ResourceKey],
    tools: &[Arc<dyn Tool>],
    workspace_root: &Path,
    path_mode_for: &impl Fn(&str) -> crate::workspace::ToolPathMode,
) -> bool {
    for existing in &batch.blocks {
        let input = serde_json::from_str(&existing.arguments).unwrap_or(serde_json::Value::Null);
        let existing_keys = tools
            .iter()
            .find(|t| t.name() == existing.name)
            .map(|t| t.resource_keys(&input, path_mode_for(&existing.name), workspace_root))
            .unwrap_or_default();
        if resource_keys_conflict(keys, &existing_keys) {
            return true;
        }
    }
    false
}

/// Cross-session process lock is write/edit only. bash and read still expose
/// `resource_keys` for same-turn partitioning; they must not fail other sessions.
fn cross_session_lock_keys(tool_name: &str, keys: Vec<ResourceKey>) -> Vec<ResourceKey> {
    match tool_name {
        "write" | "edit" => keys,
        _ => Vec::new(),
    }
}

fn resource_keys_conflict(a: &[ResourceKey], b: &[ResourceKey]) -> bool {
    for x in a {
        for y in b {
            if x == y {
                return true;
            }
            if matches!(x, ResourceKey::Workspace) || matches!(y, ResourceKey::Workspace) {
                return true;
            }
        }
    }
    false
}

fn cancelled_tool_result(name: &str) -> ToolCallResult {
    ToolCallResult::error(format!("tool '{name}' cancelled")).finalize_signals()
}

fn wire_result(result: ToolCallResult) -> ToolCallResult {
    result.finalize_signals()
}

fn attach_unknown_param_warning(result: ToolCallResult, extra: &[String]) -> ToolCallResult {
    if extra.is_empty() || result.level == ToolSignalLevel::Error {
        return result;
    }
    let msg = format!(
        "ignored unknown parameter(s): {}. Not in this tool's schema; the call ran without them",
        extra.join(", ")
    );
    let existing = result
        .warning_status
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(existing) = existing {
        result.with_warning(format!("{existing}; {msg}"))
    } else {
        result.with_warning(msg)
    }
}

fn call_id(fc: &FunctionToolCall) -> String {
    fc.call_id.clone()
}

fn filename_for_artifact(artifact: &MediaArtifact) -> String {
    let ext = match artifact.mime_type.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" => "wav",
        "audio/ogg" => "ogg",
        other if other.contains('/') => other.split('/').nth(1).unwrap_or("bin"),
        _ => "bin",
    };
    match artifact.kind {
        MediaKind::Image => format!("image.{ext}"),
        MediaKind::Video => format!("video.{ext}"),
        MediaKind::Audio => format!("audio.{ext}"),
    }
}

/// Materialize one media artifact into authority [`InputContent`].
///
/// Images → [`InputContent::InputImage`]; video/audio → [`InputContent::InputFile`].
/// Unmaterialized `LocalFile`, missing mime / data_root, or missing blob → [`Err`].
fn input_content_from_artifact(
    artifact: &MediaArtifact,
    data_root: Option<&Path>,
) -> Result<InputContent> {
    let resolved = resolve_media_artifact_url(data_root, artifact)?;
    match artifact.kind {
        MediaKind::Image => Ok(InputContent::InputImage(InputImageContent {
            detail: Default::default(),
            file_id: None,
            image_url: Some(resolved),
        })),
        MediaKind::Video | MediaKind::Audio => {
            let filename = Some(filename_for_artifact(artifact));
            if resolved.starts_with("data:") {
                Ok(InputContent::InputFile(InputFileContent {
                    file_data: Some(resolved),
                    file_id: None,
                    file_url: None,
                    filename,
                    detail: None,
                }))
            } else {
                Ok(InputContent::InputFile(InputFileContent {
                    file_data: None,
                    file_id: None,
                    file_url: Some(resolved),
                    filename,
                    detail: None,
                }))
            }
        }
    }
}

/// Map [`ToolCallResult`] → [`FunctionCallOutput`], materializing media into
/// authority [`InputContent`] (no text-stub dialects for resolvable media).
fn function_call_output_from_result(
    result: &ToolCallResult,
    data_root: Option<&Path>,
) -> Result<FunctionCallOutput> {
    if result.parts.is_empty() {
        return Ok(FunctionCallOutput::Text(result.content.clone()));
    }

    let mut contents: Vec<InputContent> = Vec::new();
    if !result.content.is_empty() {
        contents.push(InputContent::InputText(InputTextContent {
            text: result.content.clone(),
        }));
    }

    for part in &result.parts {
        match part {
            ToolOutputPart::Text { text } => {
                contents.push(InputContent::InputText(InputTextContent {
                    text: text.clone(),
                }));
            }
            ToolOutputPart::Media { artifact } => {
                contents.push(input_content_from_artifact(artifact, data_root)?);
            }
        }
    }

    if contents.is_empty() {
        Ok(FunctionCallOutput::Text(result.content.clone()))
    } else {
        Ok(FunctionCallOutput::Content(contents))
    }
}

fn function_call_output_item(
    call_id: String,
    result: &ToolCallResult,
    data_root: Option<&Path>,
) -> Item {
    match function_call_output_from_result(result, data_root) {
        Ok(output) => Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id,
            output,
            id: None,
            status: None,
        }),
        // Surface encode failure as a tool error (no placeholder media dialects).
        Err(e) => Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id,
            output: FunctionCallOutput::Text(
                wire_result(ToolCallResult::error(e.to_string())).content,
            ),
            id: None,
            status: None,
        }),
    }
}

/// Drop guard that releases all of a session's write locks when the function
/// returns (success or error). Ensures a tool never leaks a held lock.
struct WriteLockGuard {
    lock: Arc<WorkspaceWriteLock>,
    session_id: String,
}

impl Drop for WriteLockGuard {
    fn drop(&mut self) {
        self.lock.release_all(&self.session_id);
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_tool(
    tool_use: &FunctionToolCall,
    tools: &[Arc<dyn Tool>],
    permission: &PermissionEngine,
    ctx: &Context,
    session_id: &str,
    agent_name: &str,
    sink: &dyn PermissionSink,
    cancel: CancellationToken,
    data_root: &std::path::Path,
    spill_threshold: usize,
    _turn_anchor_k: Option<i64>,
    write_lock: Arc<WorkspaceWriteLock>,
) -> ToolCallResult {
    let tu_name = tool_use.name.clone();
    let tu_id = call_id(tool_use);

    if cancel.is_cancelled() {
        return cancelled_tool_result(&tu_name);
    }

    let tool = match tools.iter().find(|t| t.name() == tu_name) {
        Some(t) => t.clone(),
        None => {
            return wire_result(ToolCallResult::error(format!("tool '{tu_name}' not found")));
        }
    };

    let tool_use_input = match parse_tool_arguments(&tool_use.arguments) {
        Ok(value) => value,
        Err(detail) => {
            return wire_result(ToolCallResult::error(invalid_input_for(&tu_name, detail)));
        }
    };

    if let Err(msg) = check_tool_input(tool.as_ref(), &tool_use_input) {
        return wire_result(ToolCallResult::error(invalid_input_for(&tu_name, msg)));
    }

    let auth = authorize(
        tool_use,
        tool.as_ref(),
        permission,
        ctx,
        agent_name,
        sink,
        &cancel,
    )
    .await;

    if matches!(auth, AuthResult::Aborted) {
        cancel.cancel();
        return cancelled_tool_result(&tu_name);
    }

    if cancel.is_cancelled() {
        return cancelled_tool_result(&tu_name);
    }

    let effective_input = match auth {
        AuthResult::Denied(msg) => {
            return wire_result(ToolCallResult::error(msg));
        }
        AuthResult::Aborted => return cancelled_tool_result(&tu_name),
        AuthResult::Proceed { effective_input } => effective_input,
    };

    let path_mode = permission.path_mode(&tu_name).to_tool_path_mode();
    let resource_keys = cross_session_lock_keys(
        &tu_name,
        tool.resource_keys(&effective_input, path_mode, &ctx.cwd),
    );
    let _lock_guard = if !resource_keys.is_empty() {
        match write_lock.try_acquire(&resource_keys, session_id) {
            Ok(()) => Some(WriteLockGuard {
                lock: Arc::clone(&write_lock),
                session_id: session_id.to_string(),
            }),
            Err(holder) => {
                return wire_result(ToolCallResult::error(format!(
                    "resource busy: held by session {holder}. Retry after the other session finishes writing"
                )));
            }
        }
    } else {
        None
    };

    if cancel.is_cancelled() {
        return cancelled_tool_result(&tu_name);
    }

    let tool_clone = tool.clone();
    let input = effective_input.clone();
    let name = tu_name.clone();
    let execution = ToolExecutionContext {
        path_mode,
        workspace_root: ctx.cwd.clone(),
        call_id: tu_id.clone(),
        cancel: cancel.clone(),
        output_limit: tool.max_result_size(),
        session_id: session_id.to_string(),
    };
    // Build the tool execution future
    let tool_fut = AssertUnwindSafe(tool_clone.execute(input, execution)).catch_unwind();

    // Wrap with timeout if the tool specifies one
    let raw_result = if let Some(secs) = tool.timeout() {
        match tokio::time::timeout(std::time::Duration::from_secs(secs), tool_fut).await {
            Ok(result) => result,
            Err(_elapsed) => Ok(ToolCallResult::error(format!(
                "tool '{name}' timed out after {secs} seconds. Retry with a narrower scope, or use bash run_in_background for long commands"
            ))),
        }
    } else {
        tool_fut.await
    };

    if cancel.is_cancelled() {
        return cancelled_tool_result(&tu_name);
    }

    let mut output = match raw_result {
        Ok(r) => r,
        Err(panic_payload) => {
            let msg = panic_payload_to_string(panic_payload);
            tracing::error!(tool = %name, "tool panicked: {}", msg);
            ToolCallResult::error(format!("tool '{name}' panicked: {msg}"))
        }
    };
    let unknown = unknown_top_level_properties(&tool.schema(), &effective_input);
    output = attach_unknown_param_warning(output, &unknown);
    // Compose Error/Warning/Hint before truncation so the wire text is stable.
    output = wire_result(output);
    if tool.max_result_size() < usize::MAX {
        output.content = Session::truncated_tool_result(&output.content, tool.max_result_size());
    }

    let mut output = match output::finalize_tool_call_result(output, data_root, spill_threshold) {
        Ok(processed) => processed,
        Err(e) => {
            tracing::error!(tool = %tu_name, error = %e, "tool output processing failed");
            wire_result(ToolCallResult::error(format!(
                "output processing failed: {e}"
            )))
        }
    };
    if tu_name != "wait_shell"
        && let Some(hub) = tools.iter().find_map(|t| t.agent_terminal())
    {
        let notices = hub.jobs.take_mailbox(session_id);
        if !notices.is_empty() {
            let jobs = hub.jobs.running(session_id);
            if !output.content.ends_with('\n') {
                output.content.push('\n');
            }
            output
                .content
                .push_str(&crate::tools::bash_status::format_exit_reminder(
                    &notices, &jobs, &ctx.cwd,
                ));
        }
    }

    tracing::info!(
        tool = %tu_name,
        id = %tu_id,
        output_len = output.content.len(),
        "tool executed"
    );

    output
}

fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Build FunctionCallOutput Items only (FunctionCall already in transcript from model).
///
/// `data_root` is required to resolve `BlobRef` media into data URLs. Encode failures
/// for a single call become tool-error text outputs (fail closed — no stub dialects).
pub fn outputs_from_tool_results(
    tool_uses: &[FunctionToolCall],
    results_by_id: HashMap<String, ToolCallResult>,
    data_root: &Path,
) -> Vec<Item> {
    let mut items = Vec::new();
    for fc in tool_uses {
        let cid = call_id(fc);
        let result = results_by_id.get(&cid);
        let item = match result {
            Some(r) => function_call_output_item(cid, r, Some(data_root)),
            // Reached on cancelled turns (user stopped the loop) and as a
            // fail-closed fallback: the model must never see a dangling tool call.
            None => Item::FunctionCallOutput(FunctionCallOutputItemParam {
                call_id: cid.clone(),
                output: FunctionCallOutput::Text(
                    wire_result(ToolCallResult::error(format!(
                        "tool '{}' was interrupted: the user cancelled the turn before a result arrived",
                        fc.name
                    )))
                    .content,
                ),
                id: None,
                status: None,
            }),
        };
        items.push(item);
    }
    items
}

/// Alias kept for call sites that still say "items from results" — output-only.
pub fn items_from_tool_results(
    tool_uses: &[FunctionToolCall],
    results_by_id: HashMap<String, ToolCallResult>,
    data_root: &Path,
) -> Vec<Item> {
    outputs_from_tool_results(tool_uses, results_by_id, data_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::media::write_media_blob;
    use crate::types::{MediaArtifact, MediaSource, ToolOutputPart};

    #[test]
    fn cross_session_lock_is_write_edit_only() {
        let file = vec![ResourceKey::File("/a".into())];
        assert_eq!(cross_session_lock_keys("write", file.clone()), file);
        assert_eq!(cross_session_lock_keys("edit", file.clone()), file);
        assert!(cross_session_lock_keys("bash", file.clone()).is_empty());
        assert!(cross_session_lock_keys("read", file).is_empty());
    }

    #[test]
    fn blob_ref_image_becomes_input_image_data_url() {
        let dir = tempfile::tempdir().unwrap();
        let blob_id = write_media_blob(dir.path(), b"\x89PNG").unwrap();
        let result = ToolCallResult {
            content: "shot".into(),
            parts: vec![ToolOutputPart::Media {
                artifact: MediaArtifact::image(MediaSource::BlobRef { blob_id }, "image/png"),
            }],
            ..ToolCallResult::ok("")
        };
        let out = function_call_output_from_result(&result, Some(dir.path())).unwrap();
        match out {
            FunctionCallOutput::Content(parts) => {
                let img = parts.iter().find_map(|p| match p {
                    InputContent::InputImage(i) => Some(i),
                    _ => None,
                });
                let img = img.expect("InputImage");
                let url = img.image_url.as_deref().expect("image_url");
                assert!(url.starts_with("data:image/png;base64,"));
                assert!(!parts.iter().any(|p| matches!(
                    p,
                    InputContent::InputText(t) if t.text.contains("blob:")
                )));
            }
            FunctionCallOutput::Text(t) => panic!("expected Content, got Text: {t}"),
        }
    }

    #[test]
    fn local_file_after_finalize_encodes_as_input_image() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("shot.png");
        // Minimal PNG header so finalize can optionally read dims; bytes still materialize.
        std::fs::write(&file_path, b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR").unwrap();
        let raw = ToolCallResult {
            content: String::new(),
            parts: vec![ToolOutputPart::image_file(
                file_path.to_string_lossy(),
                "image/png",
            )],
            ..ToolCallResult::ok("")
        };
        let finalized = output::finalize_tool_call_result(raw, dir.path(), 1024).unwrap();
        assert!(matches!(
            finalized.parts[0],
            ToolOutputPart::Media {
                artifact: MediaArtifact {
                    source: MediaSource::BlobRef { .. },
                    ..
                }
            }
        ));
        let out = function_call_output_from_result(&finalized, Some(dir.path())).unwrap();
        match out {
            FunctionCallOutput::Content(parts) => {
                assert!(
                    parts
                        .iter()
                        .any(|p| matches!(p, InputContent::InputImage(_)))
                );
            }
            FunctionCallOutput::Text(t) => panic!("expected Content, got Text: {t}"),
        }
    }

    #[test]
    fn missing_blob_returns_err() {
        let result = ToolCallResult {
            content: String::new(),
            parts: vec![ToolOutputPart::Media {
                artifact: MediaArtifact::image(
                    MediaSource::BlobRef {
                        blob_id: "missing-blob".into(),
                    },
                    "image/png",
                ),
            }],
            ..ToolCallResult::ok("")
        };
        let dir = tempfile::tempdir().unwrap();
        let err = function_call_output_from_result(&result, Some(dir.path())).unwrap_err();
        assert!(
            matches!(err, crate::types::LitecodeError::MediaBlobMissing(_))
                || err.to_string().contains("missing")
                || err.to_string().contains("unavailable")
        );
    }

    #[test]
    fn unmaterialized_local_file_returns_err() {
        let result = ToolCallResult {
            content: String::new(),
            parts: vec![ToolOutputPart::image_file("/tmp/nope.png", "image/png")],
            ..ToolCallResult::ok("")
        };
        let err = function_call_output_from_result(&result, None).unwrap_err();
        assert!(err.to_string().contains("unmaterialized"));
    }

    #[test]
    fn video_becomes_input_file() {
        let result = ToolCallResult {
            content: String::new(),
            parts: vec![ToolOutputPart::video(
                MediaSource::Url {
                    url: "https://example.com/v.mp4".into(),
                },
                "video/mp4",
            )],
            ..ToolCallResult::ok("")
        };
        let out = function_call_output_from_result(&result, None).unwrap();
        match out {
            FunctionCallOutput::Content(parts) => {
                let file = parts.iter().find_map(|p| match p {
                    InputContent::InputFile(f) => Some(f),
                    _ => None,
                });
                let file = file.expect("InputFile");
                assert_eq!(file.file_url.as_deref(), Some("https://example.com/v.mp4"));
            }
            FunctionCallOutput::Text(t) => panic!("expected Content, got Text: {t}"),
        }
    }

    /// 2.6: a blocking tool whose work runs in `spawn_blocking` (the fixed
    /// custom/webfetch pattern) must be interruptible by the pipeline timeout —
    /// it must not stall the async executor so the timeout never fires.
    #[tokio::test]
    async fn blocking_tool_is_interrupted_by_pipeline_timeout() {
        struct SlowTool;
        impl Tool for SlowTool {
            fn name(&self) -> &str {
                "slow"
            }
            fn schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object", "properties": {}})
            }
            fn description(&self, _ctx: &Context) -> String {
                "blocks for a while".into()
            }
            fn timeout(&self) -> Option<u64> {
                Some(1)
            }
            fn execute(
                &self,
                _input: serde_json::Value,
                _execution: ToolExecutionContext,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>>
            {
                Box::pin(async move {
                    // Mirrors the 2.6 fix: blocking work runs off the executor so
                    // the pipeline timeout can interrupt the call.
                    let _ = tokio::task::spawn_blocking(|| {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    })
                    .await;
                    ToolCallResult::ok("done")
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut global = crate::config::schema::GlobalSettings::default();
        global.agents.insert(
            "default".into(),
            crate::config::schema::AgentProfile {
                role: crate::config::schema::AgentRole::Primary,
                model_ref: "default".into(),
                tools: HashMap::from([(
                    "slow".to_string(),
                    crate::config::schema::AgentToolBinding {
                        enabled: true,
                        policy: crate::permission::ToolPolicy::allow_all(),
                        path_mode: crate::permission::BindingPathMode::default(),
                        last_applied_preset: None,
                        allowed_tools: None,
                    },
                )]),
                ..Default::default()
            },
        );
        let resolved = crate::config::resolved::resolve(
            global,
            crate::config::resolved::WorkspaceState::new("/tmp/test"),
        );
        let permission = crate::permission::PermissionEngine::resolver(resolved, "default", 0);
        let ctx = Context {
            cwd: dir.path().to_path_buf(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(dir.path()),
            agents_md: None,
            claude_md: None,
        };
        let sink = crate::permission::sinks::RecordingPermissionSink::from_reply(true, true);
        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(SlowTool)];
        let tu = FunctionToolCall {
            arguments: "{}".into(),
            call_id: "call_1".into(),
            name: "slow".into(),
            namespace: None,
            id: Some("fc_1".into()),
            status: None,
        };
        let start = std::time::Instant::now();
        let result = run_tool(
            &tu,
            &tools,
            &permission,
            &ctx,
            "sess",
            "default",
            &sink,
            CancellationToken::new(),
            dir.path(),
            4096,
            None,
            Arc::new(WorkspaceWriteLock::new()),
        )
        .await;
        assert!(
            result.content.contains("timed out after 1 seconds"),
            "expected timeout error, got: {:?}",
            result.content
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(3),
            "timeout did not interrupt promptly: {:?}",
            start.elapsed()
        );
    }

    fn fc(call_id: &str, name: &str, args: serde_json::Value) -> FunctionToolCall {
        FunctionToolCall {
            arguments: args.to_string(),
            call_id: call_id.into(),
            name: name.into(),
            namespace: None,
            id: Some(call_id.into()),
            status: None,
        }
    }

    #[test]
    fn partition_splits_same_path_read_and_write() {
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(crate::tools::read::ReadTool::default()),
            Arc::new(crate::tools::write::WriteTool::default()),
        ];
        let root = Path::new("/proj");
        let uses = vec![
            fc("r1", "read", serde_json::json!({"file_path": "a.txt"})),
            fc(
                "w1",
                "write",
                serde_json::json!({"file_path": "a.txt", "content": "x"}),
            ),
        ];
        let batches =
            partition_tool_calls(&uses, &tools, root, |_| crate::workspace::ToolPathMode::All);
        assert!(
            batches.len() >= 2,
            "same-path read/write must not share a concurrent batch, got {batches:?}"
        );
        assert!(
            !batches
                .iter()
                .any(|b| b.is_concurrency_safe && b.blocks.len() > 1),
            "overlapping path tools must not run in one concurrent batch"
        );
    }

    #[test]
    fn partition_splits_same_path_read_and_readonly_bash() {
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(crate::tools::read::ReadTool::default()),
            Arc::new(crate::tools::bash::BashTool::new(Arc::new(
                crate::terminal::TerminalHub::new(),
            ))),
        ];
        let root = Path::new("/proj");
        let uses = vec![
            fc("r1", "read", serde_json::json!({"file_path": "a.txt"})),
            fc("b1", "bash", serde_json::json!({"command": "cat a.txt"})),
        ];
        let batches =
            partition_tool_calls(&uses, &tools, root, |_| crate::workspace::ToolPathMode::All);
        assert!(
            batches.len() >= 2,
            "same-path read and bash must not share a concurrent batch, got {} batches",
            batches.len()
        );
    }

    #[test]
    fn partition_keeps_different_path_reads_concurrent() {
        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(crate::tools::read::ReadTool::default())];
        let root = Path::new("/proj");
        let uses = vec![
            fc("r1", "read", serde_json::json!({"file_path": "a.txt"})),
            fc("r2", "read", serde_json::json!({"file_path": "b.txt"})),
        ];
        let batches =
            partition_tool_calls(&uses, &tools, root, |_| crate::workspace::ToolPathMode::All);
        assert_eq!(batches.len(), 1);
        assert!(batches[0].is_concurrency_safe);
        assert_eq!(batches[0].blocks.len(), 2);
    }

    #[test]
    fn bash_is_cancellable_other_tools_are_not_by_default() {
        let bash = crate::tools::bash::BashTool::new(Arc::new(crate::terminal::TerminalHub::new()));
        assert!(bash.is_cancellable());
        struct Dummy;
        impl Tool for Dummy {
            fn name(&self) -> &str {
                "dummy"
            }
            fn schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            fn description(&self, _ctx: &crate::context_pipeline::Context) -> String {
                String::new()
            }
        }
        assert!(!Dummy.is_cancellable());
    }

    async fn run_schema_tool(tool: Arc<dyn Tool>, arguments: &str) -> ToolCallResult {
        let name = tool.name().to_string();
        let dir = tempfile::tempdir().unwrap();
        let mut global = crate::config::schema::GlobalSettings::default();
        global.agents.insert(
            "default".into(),
            crate::config::schema::AgentProfile {
                role: crate::config::schema::AgentRole::Primary,
                model_ref: "default".into(),
                tools: HashMap::from([(
                    name.clone(),
                    crate::config::schema::AgentToolBinding {
                        enabled: true,
                        policy: crate::permission::ToolPolicy::allow_all(),
                        path_mode: crate::permission::BindingPathMode::default(),
                        last_applied_preset: None,
                        allowed_tools: None,
                    },
                )]),
                ..Default::default()
            },
        );
        let resolved = crate::config::resolved::resolve(
            global,
            crate::config::resolved::WorkspaceState::new("/tmp/test"),
        );
        let permission = crate::permission::PermissionEngine::resolver(resolved, "default", 0);
        let ctx = Context {
            cwd: dir.path().to_path_buf(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(dir.path()),
            agents_md: None,
            claude_md: None,
        };
        let sink = crate::permission::sinks::RecordingPermissionSink::from_reply(true, true);
        let tu = FunctionToolCall {
            arguments: arguments.into(),
            call_id: "call_1".into(),
            name,
            namespace: None,
            id: Some("fc_1".into()),
            status: None,
        };
        let result = run_tool(
            &tu,
            &[tool],
            &permission,
            &ctx,
            "sess",
            "default",
            &sink,
            CancellationToken::new(),
            dir.path(),
            4096,
            None,
            Arc::new(WorkspaceWriteLock::new()),
        )
        .await;
        result
    }

    #[tokio::test]
    async fn glob_wrong_pattern_type_is_unified_error() {
        let result =
            run_schema_tool(Arc::new(crate::tools::glob::GlobTool), r#"{"pattern": 1}"#).await;
        assert_eq!(
            result.content,
            "Error: invalid input for 'glob': parameter 'pattern' expected string, got integer 1"
        );
    }

    #[tokio::test]
    async fn invalid_json_arguments_are_not_swallowed() {
        let result = run_schema_tool(Arc::new(crate::tools::glob::GlobTool), "{not json").await;
        assert!(
            result
                .content
                .starts_with("Error: invalid input for 'glob': arguments were not valid JSON"),
            "{}",
            result.content
        );
    }

    struct CustomShapeTool;

    impl Tool for CustomShapeTool {
        fn name(&self) -> &str {
            "my_custom"
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {"key": {"type": "string"}},
                "required": ["key"]
            })
        }
        fn description(&self, _ctx: &Context) -> String {
            "custom-shaped".into()
        }
        fn call_inner(&self, _input: serde_json::Value) -> ToolCallResult {
            ToolCallResult::ok("ran")
        }
    }

    #[tokio::test]
    async fn custom_shaped_schema_rejects_wrong_type() {
        let result = run_schema_tool(Arc::new(CustomShapeTool), r#"{"key": false}"#).await;
        assert_eq!(
            result.content,
            "Error: invalid input for 'my_custom': parameter 'key' expected string, got boolean false"
        );
    }

    #[tokio::test]
    async fn unknown_parameters_warn_and_still_run() {
        let result =
            run_schema_tool(Arc::new(CustomShapeTool), r#"{"key": "ok", "timeout": 3}"#).await;
        assert!(result.content.contains("ran"), "{}", result.content);
        assert!(
            result
                .content
                .contains("ignored unknown parameter(s): timeout"),
            "{}",
            result.content
        );
        assert_eq!(result.level, crate::types::ToolSignalLevel::Warning);
    }

    #[tokio::test]
    async fn custom_shaped_schema_rejects_missing_required() {
        let result = run_schema_tool(Arc::new(CustomShapeTool), "{}").await;
        assert_eq!(
            result.content,
            "Error: invalid input for 'my_custom': missing required parameter 'key'"
        );
    }

    struct EchoTool;

    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo_tool"
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        fn description(&self, _ctx: &Context) -> String {
            "echo".into()
        }
        fn call_inner(&self, _input: serde_json::Value) -> ToolCallResult {
            ToolCallResult::ok("echo-ok")
        }
    }

    async fn run_named(
        tools: Vec<Arc<dyn Tool>>,
        name: &str,
        arguments: &str,
        session_id: &str,
        cwd: &std::path::Path,
    ) -> ToolCallResult {
        let mut global = crate::config::schema::GlobalSettings::default();
        let mut bindings = HashMap::new();
        for t in &tools {
            bindings.insert(
                t.name().to_string(),
                crate::config::schema::AgentToolBinding {
                    enabled: true,
                    policy: crate::permission::ToolPolicy::allow_all(),
                    path_mode: crate::permission::BindingPathMode::default(),
                    last_applied_preset: None,
                    allowed_tools: None,
                },
            );
        }
        global.agents.insert(
            "default".into(),
            crate::config::schema::AgentProfile {
                role: crate::config::schema::AgentRole::Primary,
                model_ref: "default".into(),
                tools: bindings,
                ..Default::default()
            },
        );
        let resolved = crate::config::resolved::resolve(
            global,
            crate::config::resolved::WorkspaceState::new(cwd),
        );
        let permission = crate::permission::PermissionEngine::resolver(resolved, "default", 0);
        let ctx = Context {
            cwd: cwd.to_path_buf(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(cwd),
            agents_md: None,
            claude_md: None,
        };
        let sink = crate::permission::sinks::RecordingPermissionSink::from_reply(true, true);
        let tu = FunctionToolCall {
            arguments: arguments.into(),
            call_id: "call_1".into(),
            name: name.into(),
            namespace: None,
            id: Some("fc_1".into()),
            status: None,
        };
        let result = run_tool(
            &tu,
            &tools,
            &permission,
            &ctx,
            session_id,
            "default",
            &sink,
            CancellationToken::new(),
            cwd,
            4096,
            None,
            Arc::new(WorkspaceWriteLock::new()),
        )
        .await;
        result
    }

    fn wait_job_exit(hub: &crate::terminal::TerminalHub, id: &str) {
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(8) {
            if hub.jobs.get(id).is_some_and(|(alive, _, _, _)| !alive) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("job {id} did not exit");
    }

    #[tokio::test]
    async fn mailbox_reminder_appended_to_next_tool_result() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(crate::terminal::TerminalHub::new());
        let spawned = hub
            .spawn_command("echo hi", None, dir.path(), "sess", "")
            .unwrap();
        wait_job_exit(&hub, &spawned.id);
        let notice = hub.jobs.notice_snapshot(&spawned.id).expect("notice");
        let reminder = crate::tools::bash_status::format_exit_reminder(
            &[notice],
            &hub.jobs.running("sess"),
            dir.path(),
        );
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(EchoTool),
            Arc::new(crate::tools::bash::BashTool::new(Arc::clone(&hub))),
        ];
        let result = run_named(tools, "echo_tool", "{}", "sess", dir.path()).await;
        assert_eq!(result.content, format!("echo-ok\n{reminder}"));
        assert!(hub.jobs.take_mailbox("sess").is_empty());
    }

    #[tokio::test]
    async fn wait_shell_exit_does_not_wrap_reminder() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(crate::terminal::TerminalHub::new());
        let spawned = hub
            .spawn_command("echo hi", None, dir.path(), "sess", "")
            .unwrap();
        wait_job_exit(&hub, &spawned.id);
        let notice = hub.jobs.notice_snapshot(&spawned.id).expect("notice");
        let expected = crate::tools::bash_status::format_exited_status(
            &notice,
            dir.path(),
            &hub.jobs.running("sess"),
        );
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(crate::tools::wait_shell::WaitShellTool::new(Arc::clone(
                &hub,
            ))),
            Arc::new(crate::tools::bash::BashTool::new(Arc::clone(&hub))),
        ];
        let args = serde_json::json!({ "id": spawned.id }).to_string();
        let result = run_named(tools, "wait_shell", &args, "sess", dir.path()).await;
        assert_eq!(result.content, expected);
        assert!(!result.content.contains("<system-reminder>"));
        assert!(hub.jobs.take_mailbox("sess").is_empty());
    }
}
