# WP-135 / Session：session 语料进度上报（parent WP-132 / WP-131）— 修订版（已批准）

依据：WP-131-backend.md（冻结接口）、.tmp/wp134_signature_final.md（WP-134 定稿，src/engines/progress.rs 已落地）。
原 WP-134 阻塞已解除：使用真实类型 crate::engines::progress::{CorpusState, CorpusPhase, ProgressEngine, FailReason, CorpusError, CorpusProgress}，只经构造器 ready/scanning/embedding/needs_rebuild/failed 建值，不直接写字段，不另造类型。

## 入口
session_search::session_corpus_progress(root, SessionEngineFacts{desired, state})（仿 CodeEngineFacts；desired=false 时 Stopped/Failed 不报 failed） -> CorpusProgress（semantic_index.rs）。
status_view.rs:238 的临时适配器（只报 ready/needs_rebuild）被替换为一行委托；status_view 其余不动。

## 状态 → 构造器
| 情形 | 构造器 |
|---|---|
| engine=Stopped 且 desired | failed(CorpusError::new(Session, EngineStopped, ..), indexed) |
| engine=Failed 且 desired | failed(.., EngineUnavailable, ..) |
| job.json=Failed（reconcile 出错） | failed(.., IndexFailed, msg) |
| job Running, total 未知（切块中） | scanning(done, 0) |
| job Running, total 已知（嵌入中） | embedding(done, total, total-done) |
| queue.json 有排队会话（未切块） | scanning(indexed, 0)（Q1） |
| meta 缺失/不兼容且有 sessions.db | needs_rebuild(indexed, indexed) |
| 其他 | ready(indexed, 0) |
ModelMissing：session 拿不到 model_ready，由 code 腿上报，合并时胜出。

## 存储（Q3：与 IDE 同式）
- session-index/job.json：{status: running|failed, chunks{done,total?}, error?}，仿 code job.json + JobChunks。
- session-index/queue.json：SessionQueue{queued, embed_dirty}，仿 CodeQueue（同形、0 时删除）；单位=会话数，来自 change_log_since(watermark) 的去重 session_id，精确。
- pending_hint.json 与 abs_diff 估算已删除（Q4）。session_work_from_disk 读 queue.json。

## 两段式增量
pass1 切块：write scanning(done=保留块, total=None) → derive；pass2：total=保留块+staged（精确）→ 每 batch done+=n → save_meta 后清 job/queue → ready(done==total)。

## chunks_pending / files_pending
chunks_pending 只在切块后（embedding）= total-done，其余 0，不从行/文件估。files_pending = queue.queued（精确会话数），经 with_files_pending 填入；TODO 已删除。

## 交付结果（第二轮，按后端中层裁决）
1. files_pending：填 queue.queued（精确会话数），TODO 已删；新测 wp135_files_pending_is_exact_queued_sessions（3 行/2 会话 ⇒ files_pending=2，chunks_pending=0；消费后归 0）。
2. desired：新增 SessionEngineFacts{desired, state}（仿 CodeEngineFacts）；status_view 只改调用（适配器函数改为传 &EngineStatus 的委托，取真实 status.desired/state）。新测 wp135_undesired_stopped_engine_is_not_failed（desired=false + Stopped/Failed ⇒ 非 failed、无 error）。
3. 删除会话的块：读码确认 reconcile 先按 to_remove 删 doc，再以 self.chunks.len()+staged 计 total；新测 wp135_total_excludes_chunks_of_deleted_sessions（真实 SessionMutation::Delete 删除一个 5 行会话 + 另一会话新增 1 行：嵌入中每次 chunks_total == 最终 index.len()，结束 ready(done==total)，且 < 删除前）。无需修复。
4. read_session_pending_hint 保留（无动作）。
5. rustfmt：rustfmt 1.10.0（1.99）。只 fmt 我方 semantic_index.rs、session_search/mod.rs；mod.rs 中我方测试段按 fmt 输出手工调整（不对 IDE 文件整体 fmt）。`cargo fmt --check` 退出 0。
6. 基线（git worktree @ HEAD 37d5fbf，位于 .tmp/wp135/baseline，跑完已移除；未动他人未提交改动）：
   - terminal::shell::tests::shell_label_uses_the_program_stem：基线同样 FAILED（src/terminal/shell.rs:126）⇒ 与本包无关。
   - tools::bash::tests::live_large_output_windows_and_keeps_full_log：基线 ok；当前树本轮全量 ok、单独连跑 3 次均 ok。首轮失败为负载下偶发（flaky），非本包引起。
   - Windows 路径 4 个 = ISSUE-046；tool::registry = ISSUE-055。

## 测试（toolchain 1.99，cargo test -j 2 --no-fail-fast）
- 全量：57 个二进制，2329 通过 / 7 失败。其中 1 个为本包 wp135_incremental（files_pending 加入后旧断言未带 files_pending=1），已修正并重跑本包测试 10/10 通过；其余 6 个 = ISSUE-046×4、ISSUE-055×1、terminal::shell×1（基线即失败）。修正后等效 2330 通过 / 6 失败（未再整轮重跑，改动仅一行测试断言）。
- 本包测试共 10 个（9×wp135_* + engines::tests::session_tick_drains_queue_without_pending_hint），全部通过。

## 交付结果（第三轮：IDE 交叉审阅有条件通过后的修复）
- 磁盘：已删 .tmp/wp135 下可再生内容（独立 target 4.8G+、tree、base、baseline worktree，git worktree prune），只留日志；.tmp/wp135 由 11G 降到 596K，磁盘可用从 24G 升到 35G。此后只用仓库主 target。
- M1（方案 a，契约不变）：从一开始 done 就只报保留块。queue.json 增加 `kept_chunks`：入队时精确计算（indexed_chunks 减去 settled 中已不在库的行的 doc_keys 数，与 reconcile 第一步删除的块完全一致；legacy 无 settled 时用 indexed_chunks）。排队中的 scanning 用 kept_chunks 作 done，reconcile 的 scanning（semantic_index.rs 原 :513）同样是删除后的块数，所以 done 不会下降。删除场景测试新增“从第一条上报到结束单调”的断言。
- M2：
  - reconcile 拆为外层 + reconcile_inner：内部任何 Err 都写 failed（不论调用方是谁）。外层还有进程内 in-flight 守卫，出错时同样会释放。测试 wp135_reconcile_failure_never_leaves_running。
  - 加载时（load_session_index，以及 queue_session_dirty 开头）如果是 running，但本进程没有正在跑的 reconcile，就清除它；随后 queue_session_dirty 按水位线重新入队。测试 wp135_stale_running_is_cleared_on_load_and_requeued，另有反例 wp135_live_running_is_not_cleared。
- 原子写：job.json / queue.json 先写同目录临时文件（.name.pid.tmp），再 rename。测试 wp135_records_are_written_atomically。
- L1：change_log 读失败时打 tracing::warn，并保留原记录。L2：SessionQueue::embed_dirty 注明单位是会话（等于 queued，是会话数的上界，不是工作量估算），与 CodeQueue 对应。L4：session_corpus_progress 注明 desired=false 时 Stopped/Failed 不算 failed，按盘上状态上报，与 code_corpus_progress 同序同规则。
- 测试（cargo 1.99，-j 2，--lib 过滤 session_search / wp135 / status_view / engines tick）：182 通过，0 失败，1 忽略。其中 wp135_* 共 13 个，全部通过。
- 期间 IDE 在 engines/mod.rs:1105 有一段尚未落地的改动（reclaim_stale_index_job 还没导出），编译暂时被阻断；等 IDE 导出后重跑通过。
- fmt：我方 semantic_index.rs、session_search/mod.rs 用 `rustfmt --check` 通过。全仓 `cargo fmt --check` 目前退出码为 1，报出的 diff 都在 IDE 进行中的文件里（code_search/index_status.rs、code_search/mod.rs:48、engines/mod.rs:1102 的 reclaim 段），不是我方代码。

## 交付结果（第四轮：IDE 复审退回的 blocker 及附带项）
- Blocker（M2 跨进程误清）：从 queue_session_dirty 中移除 recover_stale_session_job。主进程 tick（engines/mod.rs consume_session_index_work → session_work_now → queue_session_dirty）和状态轮询都不再改 job.json。stale running 只在 worker 进程加载索引（load_session_index，由 code_search 运行时在 worker 中调用）时清理；IN_FLIGHT 只在 worker 进程内才有意义。没有引入 pid 或文件锁。
  - 新测 wp135_main_process_tick_does_not_touch_worker_running_job：模拟另一进程的 worker 正在跑（盘上 running，本进程 IN_FLIGHT 为空），并在中途写入新行，然后连跑 3 次 session_work_now / session_work_from_disk / session_corpus_progress。断言 job.json 字节不变，phase 仍是 embedding。
  - 原有测试 wp135_stale_running_is_cleared_on_load_and_requeued 改为按 worker 加载路径验证。
- 1) 旧 queue.json：kept_chunks 改为 Option<u64>；字段缺失时回退到 meta.indexed_chunks，不会回落到 0。新测 wp135_legacy_queue_without_kept_chunks_falls_back_to_indexed。
- 2) 临时文件名改为 `.name.<pid>.<进程内原子序号>.tmp`，同一进程内的并发写不会共用一个临时文件。
- 3) ready→scanning 之间 done 可能下降（会话被删除时）：不改。后端中层会把契约改为“单调只在同一刷新周期内成立”。
- 测试（HOME=$PWD/.tmp/home，unset RUSTUP_HOME/CARGO_HOME，cargo 1.99，-j 2，主 target）：本包过滤 184 通过 / 0 失败 / 1 忽略，其中 wp135_* 15 个全部通过。
- fmt：我方 semantic_index.rs、session_search/mod.rs 用 `rustfmt --check` 通过。全仓 `cargo fmt --check` 剩余 diff 全部属 IDE：code_search/index_status.rs:1105，code_search/mod.rs:879/1022/1046/1061。

## Diff：src/engines/session_search/semantic_index.rs
```diff
diff --git a/src/engines/session_search/semantic_index.rs b/src/engines/session_search/semantic_index.rs
index bcc2b1a..1312596 100644
--- a/src/engines/session_search/semantic_index.rs
+++ b/src/engines/session_search/semantic_index.rs
@@ -470,11 +470,32 @@ impl SessionSemanticIndex {
         reader: &SessionDataReader,
         workspace_root: &Path,
         embedder: &mut dyn Embedder,
+    ) -> Result<bool> {
+        // WP-135 M2: a `running` job.json must never outlive its run. The guard
+        // marks this root in flight for stale-record recovery; any error inside
+        // turns the record into `failed` instead of leaving `running` behind.
+        let _in_flight = InFlightGuard::enter(workspace_root);
+        match self.reconcile_inner(reader, workspace_root, embedder) {
+            Ok(changed) => Ok(changed),
+            Err(error) => {
+                mark_session_job_failed(workspace_root, &error.to_string());
+                Err(error)
+            }
+        }
+    }
+
+    fn reconcile_inner(
+        &mut self,
+        reader: &SessionDataReader,
+        workspace_root: &Path,
+        embedder: &mut dyn Embedder,
     ) -> Result<bool> {
         let live = reader.searchable_keys_blocking(None)?;
         let latest = reader.latest_change_id_blocking().unwrap_or(0);
         let keys_hash = hash_keys(&live);
         if self.last_change_id == latest && self.keys_hash == keys_hash {
+            clear_session_job(workspace_root);
+            write_session_queue_counts(workspace_root, 0, 0);
             return Ok(false);
         }
 
@@ -506,6 +527,13 @@ impl SessionSemanticIndex {
             }
         }
 
+        // WP-135: scanning — denominator = the whole index, but the new rows
+        // are not cut yet, so the total is unknown (None), never estimated.
+        // `done` = kept chunks only (deleted rows' chunks are already gone
+        // above), the same number `queue.json` reported as `kept_chunks`, so
+        // `done` never steps down across queued → scanning (M1).
+        write_session_running(workspace_root, self.chunks.len() as u64, None);
+
         if !to_add.is_empty() {
             let rows = reader.searchable_rows_for_blocking(&to_add)?;
             let mut staged: Vec<SessionChunk> = Vec::new();
@@ -571,8 +599,15 @@ impl SessionSemanticIndex {
             // One embed call per batch, not per document: the delta is small but
             // the first pass over an existing index is not, and a session run per
             // chunk is the difference between minutes and seconds.
+            // WP-135: cutting is done, so the total is exact: what the index
+            // keeps plus every staged chunk. `done` advances per batch.
+            let mut done = self.chunks.len() as u64;
+            let total = done + staged.len() as u64;
+            write_session_running(workspace_root, done, Some(total));
             for batch in staged.chunks(EMBED_BATCH) {
                 self.embed_and_add(batch.to_vec(), embedder)?;
+                done += batch.len() as u64;
+                write_session_running(workspace_root, done, Some(total));
             }
         }
 
@@ -610,7 +645,8 @@ impl SessionSemanticIndex {
             self.write_settled(workspace_root)?;
         }
         self.save_meta(workspace_root)?;
-        write_session_pending_hint(workspace_root, 0);
+        clear_session_job(workspace_root);
+        write_session_queue_counts(workspace_root, 0, 0);
         Ok(changed)
     }
 
@@ -711,36 +747,322 @@ fn index_files_exist(workspace_root: &Path) -> bool {
     vectors_path(workspace_root).is_file() && chunks_path(workspace_root).is_file()
 }
 
+// ---- WP-135: session job record -------------------------------------------
+//
+// Same storage as the code corpus (WP-134 `job.json` + chunk counters), kept in
+// this corpus's own index dir. Exact counters only: `total == None` ⇔ the new
+// rows are not cut yet (scanning). Nothing here is estimated.
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
+#[serde(rename_all = "snake_case")]
+enum SessionJobStatus {
+    /// A reconcile is cutting / embedding.
+    Running,
+    /// The last reconcile failed; cleared by the next successful one.
+    Failed,
+}
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
+struct SessionJobChunks {
+    done: u64,
+    #[serde(default)]
+    total: Option<u64>,
+}
+
 #[derive(Debug, Clone, Serialize, Deserialize)]
-struct SessionPendingHintFile {
-    pending_updates: usize,
+struct SessionJobFile {
+    status: SessionJobStatus,
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    chunks: Option<SessionJobChunks>,
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    error: Option<String>,
 }
 
-fn session_pending_hint_path(workspace_root: &Path) -> PathBuf {
-    session_index_dir(workspace_root).join("pending_hint.json")
+fn session_job_path(workspace_root: &Path) -> PathBuf {
+    session_index_dir(workspace_root).join("job.json")
 }
 
-pub fn write_session_pending_hint(workspace_root: &Path, pending_updates: usize) {
-    let path = session_pending_hint_path(workspace_root);
-    if pending_updates == 0 {
-        let _ = std::fs::remove_file(&path);
+fn read_session_job(workspace_root: &Path) -> Option<SessionJobFile> {
+    let content = std::fs::read_to_string(session_job_path(workspace_root)).ok()?;
+    serde_json::from_str(&content).ok()
+}
+
+/// Write-to-temp-then-rename in the same directory, so a poll never reads a
+/// half-written record.
+fn write_json_atomic(path: &Path, value: &impl Serialize) {
+    let Some(parent) = path.parent() else {
         return;
+    };
+    let _ = std::fs::create_dir_all(parent);
+    let Ok(body) = serde_json::to_string_pretty(value) else {
+        return;
+    };
+    let name = path
+        .file_name()
+        .and_then(|n| n.to_str())
+        .unwrap_or("record");
+    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
+    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
+    let tmp = parent.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));
+    if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, path).is_err() {
+        let _ = std::fs::remove_file(&tmp);
     }
-    if let Some(parent) = path.parent() {
-        let _ = std::fs::create_dir_all(parent);
+}
+
+fn write_session_job(workspace_root: &Path, job: &SessionJobFile) {
+    write_json_atomic(&session_job_path(workspace_root), job);
+}
+
+/// Roots with a reconcile running in this process (the worker that owns the
+/// session index). A `running` record for a root not in here has no live
+/// worker behind it.
+static IN_FLIGHT: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
+
+struct InFlightGuard(PathBuf);
+
+impl InFlightGuard {
+    fn enter(workspace_root: &Path) -> Self {
+        let root = workspace_root.to_path_buf();
+        if let Ok(mut roots) = IN_FLIGHT.lock() {
+            roots.push(root.clone());
+        }
+        Self(root)
+    }
+}
+
+impl Drop for InFlightGuard {
+    fn drop(&mut self) {
+        if let Ok(mut roots) = IN_FLIGHT.lock()
+            && let Some(i) = roots.iter().position(|r| *r == self.0)
+        {
+            roots.swap_remove(i);
+        }
     }
-    if let Ok(body) = serde_json::to_string_pretty(&SessionPendingHintFile { pending_updates }) {
-        let _ = std::fs::write(&path, body);
+}
+
+fn reconcile_in_flight(workspace_root: &Path) -> bool {
+    IN_FLIGHT
+        .lock()
+        .map(|roots| roots.iter().any(|r| r == workspace_root))
+        .unwrap_or(false)
+}
+
+/// M2: a `running` record left by a run that died (crash, kill) is cleared
+/// when the worker process loads the index — the only process that runs
+/// reconciles, so `IN_FLIGHT` is authoritative there. Never called from the
+/// main-process tick path. Returns true when it cleared one; the caller's next
+/// `queue_session_dirty` re-queues the work from the watermark, which the dead
+/// run never advanced.
+fn recover_stale_session_job(workspace_root: &Path) -> bool {
+    let stale = matches!(
+        read_session_job(workspace_root).map(|j| j.status),
+        Some(SessionJobStatus::Running)
+    ) && !reconcile_in_flight(workspace_root);
+    if stale {
+        tracing::warn!("session_search: clearing stale running job record");
+        clear_session_job(workspace_root);
+    }
+    stale
+}
+
+fn clear_session_job(workspace_root: &Path) {
+    let _ = std::fs::remove_file(session_job_path(workspace_root));
+}
+
+/// Exact pending-queue record, modelled on the code corpus's `CodeQueue`
+/// (`code-index/queue.json`): same shape, same write/remove-at-zero rule, in
+/// this corpus's own index dir. Units are sessions (the session corpus's
+/// "files"): distinct sessions with change-log entries past the index
+/// watermark. Never a chunk count: chunks exist only after cutting.
+#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
+pub struct SessionQueue {
+    /// Sessions with changes past the watermark (including deletions).
+    pub queued: usize,
+    /// Sessions that may need embedding. Unit: sessions, never chunks or rows.
+    /// The change log does not tell a pure deletion/truncation apart from an
+    /// append without reading the rows, so today this counts every queued
+    /// session (== `queued`), an upper bound of sessions, not an estimate of
+    /// work size. Mirrors `CodeQueue::embed_dirty` (paths needing embedding).
+    pub embed_dirty: usize,
+    /// Chunks the index keeps once rows gone from the store are dropped: the
+    /// exact `done` a refresh starts from (M1). Session-only field. `None` on
+    /// a record written before the field existed: readers fall back to
+    /// `meta.indexed_chunks` rather than 0.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub kept_chunks: Option<u64>,
+}
+
+fn session_queue_path(workspace_root: &Path) -> PathBuf {
+    session_index_dir(workspace_root).join("queue.json")
+}
+
+fn write_session_queue_counts(workspace_root: &Path, queued: usize, embed_dirty: usize) {
+    write_session_queue(
+        workspace_root,
+        SessionQueue {
+            queued,
+            embed_dirty,
+            kept_chunks: None,
+        },
+    );
+}
+
+fn write_session_queue(workspace_root: &Path, queue: SessionQueue) {
+    let path = session_queue_path(workspace_root);
+    if queue.queued == 0 {
+        let _ = std::fs::remove_file(&path);
+        return;
     }
+    write_json_atomic(&path, &queue);
 }
 
+/// Exact signal for status and the periodic tick: queued work on disk.
+pub fn read_session_queue(workspace_root: &Path) -> SessionQueue {
+    std::fs::read_to_string(session_queue_path(workspace_root))
+        .ok()
+        .and_then(|s| serde_json::from_str(&s).ok())
+        .unwrap_or_default()
+}
+
+fn write_session_running(workspace_root: &Path, done: u64, total: Option<u64>) {
+    write_session_job(
+        workspace_root,
+        &SessionJobFile {
+            status: SessionJobStatus::Running,
+            chunks: Some(SessionJobChunks { done, total }),
+            error: None,
+        },
+    );
+}
+
+fn mark_session_job_failed(workspace_root: &Path, error: &str) {
+    let done = read_meta(workspace_root)
+        .ok()
+        .flatten()
+        .map_or(0, |m| m.indexed_chunks as u64);
+    write_session_job(
+        workspace_root,
+        &SessionJobFile {
+            status: SessionJobStatus::Failed,
+            chunks: Some(SessionJobChunks {
+                done,
+                total: Some(done),
+            }),
+            error: Some(error.to_string()),
+        },
+    );
+}
+
+/// The store has rows the index has not taken in yet (queued or in flight).
+/// A flag, not a count: the exact count only exists once the rows are cut.
+pub fn session_has_pending_work(workspace_root: &Path) -> bool {
+    read_session_queue(workspace_root).queued > 0
+        || matches!(
+            read_session_job(workspace_root).map(|j| j.status),
+            Some(SessionJobStatus::Running)
+        )
+}
+
+/// Deprecated compatibility view for the legacy `pending_updates` field and
+/// existing callers: the exact queued-session count from `queue.json` (the
+/// `pending_hint.json` change-id estimate is gone). Remove with the other
+/// deprecated fields before WP-131 seals.
 pub fn read_session_pending_hint(workspace_root: &Path) -> usize {
-    let Ok(content) = std::fs::read_to_string(session_pending_hint_path(workspace_root)) else {
-        return 0;
+    read_session_queue(workspace_root).queued
+}
+
+/// Engine facts the session corpus needs, modelled on `CodeEngineFacts`.
+/// Model readiness is reported by the code leg (one shared embedder).
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub struct SessionEngineFacts {
+    /// Persisted workspace intent (engine enabled).
+    pub desired: bool,
+    pub state: Option<crate::engines::EngineState>,
+}
+
+/// WP-135: this corpus's progress for `retrieval.session`, built only through
+/// the WP-134 constructors.
+pub fn session_corpus_progress(
+    workspace_root: &Path,
+    facts: SessionEngineFacts,
+) -> crate::engines::progress::CorpusProgress {
+    use crate::engines::EngineState;
+    use crate::engines::progress::{CorpusError, CorpusProgress, FailReason, ProgressEngine};
+
+    let indexed = read_meta(workspace_root)
+        .ok()
+        .flatten()
+        .map_or(0, |m| m.indexed_chunks as u64);
+    let fail = |reason: FailReason, message: String, done: u64| {
+        CorpusProgress::failed(
+            CorpusError::new(ProgressEngine::Session, reason, message),
+            done,
+        )
     };
-    serde_json::from_str::<SessionPendingHintFile>(&content)
-        .map(|h| h.pending_updates)
-        .unwrap_or(0)
+    // L4: engine failures count only when desired, same order and rule as
+    // `code_corpus_progress`. desired=false (user turned the engine off):
+    // Stopped/Failed are not `failed`; the corpus reports its on-disk state
+    // (ready / needs_rebuild / queued scanning) with no error.
+    match facts.state.filter(|_| facts.desired) {
+        Some(EngineState::Stopped) => {
+            return fail(
+                FailReason::EngineStopped,
+                "search engine stopped".into(),
+                indexed,
+            );
+        }
+        Some(EngineState::Failed) => {
+            return fail(
+                FailReason::EngineUnavailable,
+                "search engine unavailable".into(),
+                indexed,
+            );
+        }
+        _ => {}
+    }
+    // files_pending = exact queued sessions (`queue.json`); chunks_pending only
+    // once rows are cut and the count is exact (no estimate from rows).
+    let queue = read_session_queue(workspace_root);
+    let files = queue.queued as u64;
+    if let Some(job) = read_session_job(workspace_root) {
+        match (job.status, job.chunks) {
+            (SessionJobStatus::Failed, _) => {
+                return fail(
+                    FailReason::IndexFailed,
+                    job.error.unwrap_or_else(|| "session index failed".into()),
+                    indexed,
+                );
+            }
+            (
+                _,
+                Some(SessionJobChunks {
+                    done,
+                    total: Some(total),
+                }),
+            ) => {
+                return CorpusProgress::embedding(done, total, total.saturating_sub(done))
+                    .with_files_pending(files);
+            }
+            // Not cut yet: total unknown, so nothing is claimed pending either
+            // (`total == None` is the signal, WP-131 Q1).
+            (_, Some(SessionJobChunks { done, total: None })) => {
+                return CorpusProgress::scanning(done, 0).with_files_pending(files);
+            }
+            (_, None) => return CorpusProgress::scanning(indexed, 0).with_files_pending(files),
+        }
+    }
+    if files > 0 && !session_should_rebuild(workspace_root) {
+        // Queued, not cut yet ⇒ refreshing + scanning, total unknown (Q1).
+        // done = kept chunks (M1): the refresh never reports fewer.
+        let kept = queue.kept_chunks.unwrap_or(indexed);
+        return CorpusProgress::scanning(kept, 0).with_files_pending(files);
+    }
+    match session_work_from_disk(workspace_root) {
+        crate::engines::code_search::IndexWork::Rebuild { .. } => {
+            CorpusProgress::needs_rebuild(indexed, indexed).with_files_pending(files)
+        }
+        _ => CorpusProgress::ready(indexed, 0).with_files_pending(files),
+    }
 }
 
 /// Absent / unloadable / embedder mismatch — warmup auto-rebuilds this corpus only.
@@ -762,14 +1084,14 @@ pub fn session_index_status(workspace_root: &Path) -> crate::engines::code_searc
             IndexStatus::Absent
         };
     }
-    if read_session_pending_hint(workspace_root) > 0 {
+    if session_has_pending_work(workspace_root) {
         IndexStatus::Stale
     } else {
         IndexStatus::Ready
     }
 }
 
-/// Work as seen from disk alone: the pending hint plus the rebuild criteria.
+/// Work as seen from disk alone: the job record plus the rebuild criteria.
 ///
 /// The hint is written when an index is loaded or a reconcile finishes, so this
 /// view can already be behind a store that has moved since. Callers that can see
@@ -793,11 +1115,14 @@ pub fn session_work_from_disk(workspace_root: &Path) -> crate::engines::code_sea
             },
         };
     }
-    let dirty = read_session_pending_hint(workspace_root);
-    if dirty == 0 {
+    // Exact queued sessions from `queue.json` (WP-135), never an estimate.
+    let queue = read_session_queue(workspace_root);
+    if queue.queued == 0 {
         IndexWork::None
     } else {
-        IndexWork::Update { dirty }
+        IndexWork::Update {
+            dirty: queue.queued,
+        }
     }
 }
 
@@ -819,6 +1144,7 @@ pub fn session_work_now(
 /// Load compatible vectors; empty shell when the library is absent/unloadable.
 /// Does not embed or write the index.
 pub fn load_session_index(workspace_root: &Path) -> Result<SessionSemanticIndex> {
+    recover_stale_session_job(workspace_root);
     let dir = session_index_dir(workspace_root);
     std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
     if session_should_rebuild(workspace_root) {
@@ -827,24 +1153,64 @@ pub fn load_session_index(workspace_root: &Path) -> Result<SessionSemanticIndex>
     SessionSemanticIndex::load(workspace_root)
 }
 
-/// Compare `sessions.db` watermark to the on-disk session index; write hint only.
+/// Compare `sessions.db` watermark to the on-disk session index and write the
+/// exact queue record (`queue.json`): distinct sessions changed since the
+/// watermark, read from the change log. Never a row- or id-delta estimate.
 pub fn queue_session_dirty(workspace_root: &Path, reader: &SessionDataReader) {
+    // A rebuild need is read from `meta.json` itself (`session_should_rebuild`).
     if session_should_rebuild(workspace_root) {
-        write_session_pending_hint(workspace_root, 1);
         return;
     }
+    // No stale-job recovery here: this runs on the main-process tick, which
+    // cannot see the worker's in-flight state (WP-135 re-review). Recovery
+    // happens only when the worker loads the index (`load_session_index`).
     let latest = reader.latest_change_id_blocking().unwrap_or(0);
-    let indexed = read_meta(workspace_root)
-        .ok()
-        .flatten()
-        .map(|m| m.last_change_id)
-        .unwrap_or(0);
+    let meta = read_meta(workspace_root).ok().flatten();
+    let indexed = meta.as_ref().map_or(0, |m| m.last_change_id);
+    let indexed_chunks = meta.as_ref().map_or(0, |m| m.indexed_chunks as u64);
     if latest == indexed {
-        write_session_pending_hint(workspace_root, 0);
+        write_session_queue_counts(workspace_root, 0, 0);
         return;
     }
-    let dirty = latest.abs_diff(indexed).max(1) as usize;
-    write_session_pending_hint(workspace_root, dirty);
+    let changes = match reader.change_log_since_blocking(indexed) {
+        Ok(changes) => changes,
+        Err(error) => {
+            // L1: leave the previous record; the next pass retries.
+            tracing::warn!(error = %error, "session_search: change log read failed; queue not updated");
+            return;
+        }
+    };
+    let sessions: HashSet<&str> = changes.iter().map(|c| c.session_id.as_str()).collect();
+    write_session_queue(
+        workspace_root,
+        SessionQueue {
+            queued: sessions.len(),
+            embed_dirty: sessions.len(),
+            kept_chunks: Some(kept_chunk_count(workspace_root, reader, indexed_chunks)),
+        },
+    );
+}
+
+/// Chunks the index keeps after dropping every settled row the store no longer
+/// holds: exactly what `reconcile` removes first. Exact (settled doc keys), not
+/// estimated; a legacy index without settled rows keeps `indexed_chunks`.
+fn kept_chunk_count(workspace_root: &Path, reader: &SessionDataReader, indexed_chunks: u64) -> u64 {
+    let (Ok(live), Ok(settled)) = (
+        reader.searchable_keys_blocking(None),
+        read_settled(workspace_root),
+    ) else {
+        return indexed_chunks;
+    };
+    if settled.is_empty() {
+        return indexed_chunks;
+    }
+    let live: HashSet<(String, i64)> = live.into_iter().collect();
+    let gone: HashSet<&String> = settled
+        .iter()
+        .filter(|(key, _)| !live.contains(*key))
+        .flat_map(|(_, row)| row.doc_keys.iter())
+        .collect();
+    indexed_chunks.saturating_sub(gone.len() as u64)
 }
 
 /// Embed + save session drift. Wipe first when the library must rebuild.
@@ -867,9 +1233,8 @@ pub fn consume_session_index(
         std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
         *index = SessionSemanticIndex::new_empty()?;
     }
-    let dirty = index.reconcile(reader, workspace_root, embedder)?;
-    write_session_pending_hint(workspace_root, 0);
-    Ok(dirty)
+    // `reconcile` itself turns any failure into a `failed` record (M2).
+    index.reconcile(reader, workspace_root, embedder)
 }
 
 /// Test/helper: load (or empty) then consume against `sessions.db`.
@@ -985,23 +1350,23 @@ mod tests {
         assert_eq!(reloaded.len(), 1, "load must not digest new session rows");
         queue_session_dirty(root, &reader);
         assert!(
-            read_session_pending_hint(root) > 0,
+            session_has_pending_work(root),
             "watermark lag must be queued, not embedded"
         );
         index.reconcile(&reader, root, &mut emb).unwrap();
         assert_eq!(index.len(), 2);
-        assert_eq!(read_session_pending_hint(root), 0);
+        assert!(!session_has_pending_work(root));
     }
 
     #[test]
     fn session_work_none_when_hint_cleared() {
         let dir = TempDir::new().unwrap();
         let root = dir.path();
-        write_session_pending_hint(root, 3);
+        write_session_queue_counts(root, 3, 3);
         assert_eq!(
             session_work_from_disk(root),
             crate::engines::code_search::IndexWork::None,
-            "hint without sessions.db or vectors is not engine work"
+            "a job record without sessions.db or vectors is not engine work"
         );
         let db = root.join(".litecode").join("sessions.db");
         std::fs::create_dir_all(db.parent().unwrap()).unwrap();
@@ -1038,7 +1403,7 @@ mod tests {
         let mut emb = HashEmbedder;
         let reader = crate::session::SessionDataReader::open(&db);
         let mut index = ensure_session_index(root, &reader, &mut emb).unwrap();
-        assert_eq!(read_session_pending_hint(root), 0);
+        assert!(!session_has_pending_work(root));
 
         let dir = root.join(".litecode").join("session-index");
         let before = (
@@ -1086,10 +1451,9 @@ mod tests {
         );
 
         data.insert_items(&id, &[user_text("later row")]).unwrap();
-        assert_eq!(
-            read_session_pending_hint(root),
-            0,
-            "the writer never touches the hint, so nothing else would see this row"
+        assert!(
+            !session_has_pending_work(root),
+            "the writer never touches the job record, so nothing else would see this row"
         );
         assert!(
             matches!(session_work_now(root, &reader), IndexWork::Update { .. }),
@@ -1126,4 +1490,544 @@ mod tests {
             "a dense rebuild must not touch the sparse lane's index"
         );
     }
+
+    // ---- WP-135 progress -------------------------------------------------
+
+    use crate::engines::EngineState;
+    use crate::engines::progress::{CorpusPhase, CorpusProgress, CorpusState, FailReason};
+
+    fn warm() -> SessionEngineFacts {
+        SessionEngineFacts {
+            desired: true,
+            state: Some(EngineState::Warm),
+        }
+    }
+
+    #[test]
+    fn wp135_undesired_stopped_engine_is_not_failed() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        for state in [EngineState::Stopped, EngineState::Failed] {
+            let p = session_corpus_progress(
+                root,
+                SessionEngineFacts {
+                    desired: false,
+                    state: Some(state),
+                },
+            );
+            assert_invariants(&p);
+            assert_ne!(
+                p.state,
+                CorpusState::Failed,
+                "user turned it off: {state:?}"
+            );
+            assert!(p.error.is_none());
+        }
+    }
+
+    #[test]
+    fn wp135_files_pending_is_exact_queued_sessions() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (data, id, db, _lease) = seeded(root, 2);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
+        assert_eq!(session_corpus_progress(root, warm()).files_pending, 0);
+        let other = data.create_session("/proj", "default", None).unwrap();
+        data.insert_items(&id, &[user_text("a")]).unwrap();
+        data.insert_items(&id, &[user_text("b")]).unwrap();
+        data.insert_items(&other, &[user_text("c")]).unwrap();
+        queue_session_dirty(root, &reader);
+        let p = session_corpus_progress(root, warm());
+        assert_invariants(&p);
+        assert_eq!(p.files_pending, 2, "two sessions changed, not three rows");
+        assert_eq!(p.chunks_pending, 0, "nothing cut yet");
+        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
+        assert_eq!(session_corpus_progress(root, warm()).files_pending, 0);
+    }
+
+    #[test]
+    fn wp135_total_excludes_chunks_of_deleted_sessions() {
+        use crate::session::data::command::{MutationId, SessionMutation};
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (data, keep, db, _lease) = seeded(root, 3);
+        let gone = data.create_session("/proj", "default", None).unwrap();
+        for i in 0..5 {
+            data.insert_items(&gone, &[user_text(&format!("doomed {i}"))])
+                .unwrap();
+        }
+        let reader = crate::session::SessionDataReader::open(&db);
+        let mut index = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
+        let before = index.len() as u64;
+        let revision = reader
+            .change_log_since_blocking(0)
+            .unwrap()
+            .iter()
+            .filter(|c| c.session_id == gone)
+            .map(|c| c.revision)
+            .max()
+            .unwrap();
+        data.mutate_blocking(SessionMutation::Delete {
+            session_id: gone.clone(),
+            expected_revision: revision,
+            operation_id: MutationId("wp135-delete".into()),
+        })
+        .unwrap();
+        data.insert_items(&keep, &[user_text("one more")]).unwrap();
+        queue_session_dirty(root, &reader);
+        let queued = session_corpus_progress(root, warm());
+        assert_eq!(queued.phase, Some(CorpusPhase::Scanning));
+        assert!(
+            queued.chunks_done < before,
+            "queued done = kept chunks only"
+        );
+
+        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
+        let mut emb = ObservingEmbedder {
+            root: root.to_path_buf(),
+            seen: seen.clone(),
+            fail: false,
+        };
+        consume_session_index(root, &reader, &mut emb, &mut index).unwrap();
+        let seen = seen.lock().unwrap().clone();
+        let end = session_corpus_progress(root, warm());
+        assert!(index.len() as u64 > 0 && (index.len() as u64) < before);
+        assert_eq!(end, CorpusProgress::ready(index.len() as u64, 0));
+        for p in &seen {
+            assert_eq!(
+                p.chunks_total,
+                Some(index.len() as u64),
+                "total counts only live sessions' chunks"
+            );
+        }
+        assert!(!seen.is_empty());
+
+        // M1: deletion run is monotonic from the very first report.
+        let mut run = vec![queued];
+        run.extend(seen.iter().cloned());
+        run.push(end);
+        let mut last = 0;
+        for p in &run {
+            assert_invariants(p);
+            assert!(p.chunks_done >= last, "done decreased: {run:?}");
+            last = p.chunks_done;
+        }
+    }
+
+    #[test]
+    fn wp135_reconcile_failure_never_leaves_running() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 3);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let mut index = load_session_index(root).unwrap();
+        let mut emb = ObservingEmbedder {
+            root: root.to_path_buf(),
+            seen: Default::default(),
+            fail: true,
+        };
+        // Direct reconcile (not via consume): the failure is still recorded.
+        assert!(index.reconcile(&reader, root, &mut emb).is_err());
+        let job = read_session_job(root).unwrap();
+        assert_eq!(job.status, SessionJobStatus::Failed);
+        assert!(!reconcile_in_flight(root), "guard released on error");
+        assert_eq!(
+            session_corpus_progress(root, warm()).state,
+            CorpusState::Failed
+        );
+    }
+
+    #[test]
+    fn wp135_stale_running_is_cleared_on_load_and_requeued() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (data, id, db, _lease) = seeded(root, 2);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
+        data.insert_items(&id, &[user_text("written before the crash")])
+            .unwrap();
+        // A run died mid-embedding: `running` on disk, no worker in flight.
+        write_session_running(root, 1, Some(9));
+        assert_eq!(
+            session_corpus_progress(root, warm()).phase,
+            Some(CorpusPhase::Embedding)
+        );
+        // Worker start-up / index load is the only place that recovers.
+        let _ = load_session_index(root).unwrap();
+        assert!(
+            read_session_job(root).is_none(),
+            "stale running cleared on worker load"
+        );
+        queue_session_dirty(root, &reader);
+        assert_eq!(read_session_queue(root).queued, 1, "work re-queued");
+        let p = session_corpus_progress(root, warm());
+        assert_invariants(&p);
+        assert_eq!(p.phase, Some(CorpusPhase::Scanning));
+        assert_eq!(p.files_pending, 1);
+    }
+
+    #[test]
+    fn wp135_main_process_tick_does_not_touch_worker_running_job() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (data, id, db, _lease) = seeded(root, 2);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
+        // Worker (another process) is mid-run: `running` on disk, nothing in
+        // this process's IN_FLIGHT — exactly what the main process sees.
+        write_session_running(root, 2, Some(7));
+        let before = std::fs::read(session_job_path(root)).unwrap();
+        data.insert_items(&id, &[user_text("arrives mid-run")])
+            .unwrap();
+        // The main-process tick path: engines/mod.rs consume_session_index_work
+        // → session_work_now → queue_session_dirty; plus status polls.
+        for _ in 0..3 {
+            let _ = session_work_now(root, &reader);
+            let _ = session_work_from_disk(root);
+            let _ = session_corpus_progress(root, warm());
+        }
+        assert_eq!(
+            std::fs::read(session_job_path(root)).unwrap(),
+            before,
+            "tick must not change the worker's job.json"
+        );
+        assert_eq!(
+            session_corpus_progress(root, warm()).phase,
+            Some(CorpusPhase::Embedding)
+        );
+    }
+
+    #[test]
+    fn wp135_legacy_queue_without_kept_chunks_falls_back_to_indexed() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 3);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let index = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
+        // A queue.json from before `kept_chunks` existed.
+        std::fs::write(
+            session_queue_path(root),
+            r#"{"queued": 2, "embed_dirty": 2}"#,
+        )
+        .unwrap();
+        let p = session_corpus_progress(root, warm());
+        assert_invariants(&p);
+        assert_eq!(p.phase, Some(CorpusPhase::Scanning));
+        assert_eq!(p.chunks_done, index.len() as u64, "not 0");
+        assert_eq!(p.files_pending, 2);
+    }
+
+    #[test]
+    fn wp135_live_running_is_not_cleared() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        write_session_running(root, 1, Some(9));
+        let _guard = InFlightGuard::enter(root);
+        assert!(!recover_stale_session_job(root));
+        assert!(read_session_job(root).is_some());
+    }
+
+    #[test]
+    fn wp135_records_are_written_atomically() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        write_session_running(root, 2, None);
+        write_session_queue_counts(root, 3, 3);
+        let names: Vec<String> = std::fs::read_dir(session_index_dir(root))
+            .unwrap()
+            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
+            .collect();
+        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
+        assert!(read_session_job(root).is_some());
+        assert_eq!(read_session_queue(root).queued, 3);
+    }
+
+    fn assert_invariants(p: &CorpusProgress) {
+        assert_eq!(
+            p.phase.is_some(),
+            p.state == CorpusState::Refreshing,
+            "{p:?}"
+        );
+        assert_eq!(
+            p.chunks_total.is_none(),
+            p.phase == Some(CorpusPhase::Scanning),
+            "{p:?}"
+        );
+        if p.phase == Some(CorpusPhase::Embedding) {
+            let total = p.chunks_total.unwrap();
+            assert!(total > 0 && p.chunks_done <= total, "{p:?}");
+        }
+        assert_eq!(p.error.is_some(), p.state == CorpusState::Failed, "{p:?}");
+    }
+
+    fn seeded(
+        root: &Path,
+        rows: usize,
+    ) -> (
+        std::sync::Arc<SessionData>,
+        String,
+        std::path::PathBuf,
+        WorkspaceWriteLease,
+    ) {
+        let db = root.join(".litecode").join("sessions.db");
+        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
+        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
+        let data = SessionData::open(&lease, &db).unwrap();
+        let id = data.create_session("/proj", "default", None).unwrap();
+        for i in 0..rows {
+            data.insert_items(
+                &id,
+                &[user_text(&format!("row number {i} about topic {i}"))],
+            )
+            .unwrap();
+        }
+        (data, id, db, lease)
+    }
+
+    /// Records every progress the corpus reports while embedding, by peeking
+    /// at the job record from inside the embedder (one call per batch).
+    struct ObservingEmbedder {
+        root: std::path::PathBuf,
+        seen: std::sync::Arc<std::sync::Mutex<Vec<CorpusProgress>>>,
+        fail: bool,
+    }
+
+    impl Embedder for ObservingEmbedder {
+        fn embedder_id(&self) -> &'static str {
+            HashEmbedder.embedder_id()
+        }
+        fn inference_device(&self) -> &'static str {
+            "hash"
+        }
+        fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
+            self.seen
+                .lock()
+                .unwrap()
+                .push(session_corpus_progress(&self.root, warm()));
+            if self.fail {
+                return Err(LitecodeError::Config("embed boom".into()));
+            }
+            HashEmbedder.embed_batch(texts)
+        }
+    }
+
+    #[test]
+    fn wp135_rebuild_reports_embedding_then_ready_exact() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 80);
+        let reader = crate::session::SessionDataReader::open(&db);
+        assert_eq!(
+            session_corpus_progress(root, warm()).state,
+            CorpusState::NeedsRebuild
+        );
+        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
+        let mut emb = ObservingEmbedder {
+            root: root.to_path_buf(),
+            seen: seen.clone(),
+            fail: false,
+        };
+        let index = ensure_session_index(root, &reader, &mut emb).unwrap();
+        let seen = seen.lock().unwrap().clone();
+        assert!(seen.len() >= 2, "80 rows must embed in several batches");
+        let total = seen[0].chunks_total.unwrap();
+        let mut last = 0;
+        for p in &seen {
+            assert_invariants(p);
+            assert_eq!(p.phase, Some(CorpusPhase::Embedding));
+            assert_eq!(p.chunks_total, Some(total), "exact total, fixed once cut");
+            assert!(p.chunks_done >= last, "done is monotonic");
+            assert_eq!(p.chunks_pending, total - p.chunks_done);
+            last = p.chunks_done;
+        }
+        assert_eq!(seen[0].chunks_done, 0, "rebuild starts from an empty index");
+        let done = session_corpus_progress(root, warm());
+        assert_invariants(&done);
+        assert_eq!(done, CorpusProgress::ready(total, 0));
+        assert_eq!(index.len() as u64, total, "ready: done == total == indexed");
+    }
+
+    #[test]
+    fn wp135_incremental_scans_then_embeds_from_kept_chunks() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (data, id, db, _lease) = seeded(root, 3);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let mut index = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
+        let kept = index.len() as u64;
+        assert_eq!(
+            session_corpus_progress(root, warm()),
+            CorpusProgress::ready(kept, 0)
+        );
+
+        for i in 0..40 {
+            data.insert_items(&id, &[user_text(&format!("later {i} news"))])
+                .unwrap();
+        }
+        queue_session_dirty(root, &reader);
+        // Q1: new rows not cut yet ⇒ refreshing + scanning, total None.
+        let queued = session_corpus_progress(root, warm());
+        assert_invariants(&queued);
+        assert_eq!(
+            queued,
+            CorpusProgress::scanning(kept, 0).with_files_pending(1)
+        );
+
+        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
+        let mut emb = ObservingEmbedder {
+            root: root.to_path_buf(),
+            seen: seen.clone(),
+            fail: false,
+        };
+        consume_session_index(root, &reader, &mut emb, &mut index).unwrap();
+        let seen = seen.lock().unwrap().clone();
+        assert!(!seen.is_empty());
+        assert_eq!(seen[0].chunks_done, kept, "denominator = whole index");
+        for p in &seen {
+            assert_invariants(p);
+            assert!(p.chunks_done >= kept);
+        }
+        let end = session_corpus_progress(root, warm());
+        assert_eq!(end, CorpusProgress::ready(index.len() as u64, 0));
+        assert_eq!(end.chunks_total, seen[0].chunks_total);
+
+        // Whole run, pass 1 (queued/scanning) → pass 2 (embedding) → ready:
+        // done never decreases, done ≤ total whenever total is known, and
+        // done == total at the end.
+        let mut run = vec![queued.clone()];
+        run.extend(seen.iter().cloned());
+        run.push(end.clone());
+        let mut last = 0;
+        for p in &run {
+            assert_invariants(p);
+            assert!(p.chunks_done >= last, "done decreased: {run:?}");
+            if let Some(total) = p.chunks_total {
+                assert!(p.chunks_done <= total, "{p:?}");
+            }
+            last = p.chunks_done;
+        }
+        assert_eq!(run[0].phase, Some(CorpusPhase::Scanning));
+        assert!(
+            run[1..run.len() - 1]
+                .iter()
+                .all(|p| p.phase == Some(CorpusPhase::Embedding))
+        );
+        assert_eq!(Some(end.chunks_done), end.chunks_total);
+    }
+
+    #[test]
+    fn wp135_reconcile_writes_scanning_before_cutting() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 2);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let index = load_session_index(root).unwrap();
+        // The scanning record is what a poll sees while rows are being cut.
+        write_session_running(root, index.len() as u64, None);
+        let p = session_corpus_progress(root, warm());
+        assert_invariants(&p);
+        assert_eq!(p.phase, Some(CorpusPhase::Scanning));
+        assert_eq!(p.chunks_total, None);
+        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
+        assert_eq!(
+            session_corpus_progress(root, warm()).state,
+            CorpusState::Ready
+        );
+    }
+
+    #[test]
+    fn wp135_index_error_is_failed_index_failed_and_clears_on_success() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 3);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let mut emb = ObservingEmbedder {
+            root: root.to_path_buf(),
+            seen: Default::default(),
+            fail: true,
+        };
+        assert!(ensure_session_index(root, &reader, &mut emb).is_err());
+        let p = session_corpus_progress(root, warm());
+        assert_invariants(&p);
+        assert_eq!(p.state, CorpusState::Failed);
+        let err = p.error.unwrap();
+        assert_eq!(
+            err.engine,
+            crate::engines::progress::ProgressEngine::Session
+        );
+        assert_eq!(err.reason, FailReason::IndexFailed);
+        assert!(err.message.contains("embed boom"));
+        assert!(!session_has_pending_work(root), "failed is not pending");
+
+        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
+        assert_eq!(
+            session_corpus_progress(root, warm()).state,
+            CorpusState::Ready
+        );
+    }
+
+    #[test]
+    fn wp135_engine_state_maps_to_fail_reason() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        for (state, reason) in [
+            (EngineState::Stopped, FailReason::EngineStopped),
+            (EngineState::Failed, FailReason::EngineUnavailable),
+        ] {
+            let p = session_corpus_progress(
+                root,
+                SessionEngineFacts {
+                    desired: true,
+                    state: Some(state),
+                },
+            );
+            assert_invariants(&p);
+            assert_eq!(p.state, CorpusState::Failed);
+            assert_eq!(p.error.unwrap().reason, reason);
+        }
+        for state in [None, Some(EngineState::Idle), Some(EngineState::Warming)] {
+            let p = session_corpus_progress(
+                root,
+                SessionEngineFacts {
+                    desired: true,
+                    state,
+                },
+            );
+            assert_invariants(&p);
+            assert_eq!(
+                p,
+                CorpusProgress::ready(0, 0),
+                "no history is ready, not work"
+            );
+        }
+    }
+
+    #[test]
+    fn wp135_no_pending_hint_file_is_written() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (data, id, db, _lease) = seeded(root, 2);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
+        data.insert_items(&id, &[user_text("more")]).unwrap();
+        queue_session_dirty(root, &reader);
+        assert!(
+            !session_index_dir(root).join("pending_hint.json").exists(),
+            "the estimate source is gone"
+        );
+        let queue = read_session_queue(root);
+        assert_eq!(
+            (queue.queued, queue.embed_dirty),
+            (1, 1),
+            "exact sessions, not a chunk or change-id count"
+        );
+        assert_eq!(
+            queue.kept_chunks.unwrap(),
+            read_meta(root).unwrap().unwrap().indexed_chunks as u64
+        );
+        let p = session_corpus_progress(root, warm());
+        assert_eq!(p.chunks_total, None, "no total before cutting");
+        assert_eq!(p.chunks_pending, 0, "no chunk estimate from rows");
+    }
 }
```

## Diff：src/engines/session_search/mod.rs
```diff
diff --git a/src/engines/session_search/mod.rs b/src/engines/session_search/mod.rs
index e95f235..b75032c 100644
--- a/src/engines/session_search/mod.rs
+++ b/src/engines/session_search/mod.rs
@@ -27,9 +27,10 @@ mod trimmer;
 pub use lexical::{ensure_sparse_index, spawn_sparse_maintenance};
 pub use ranking::{ContentRole, HitEvidence, LayerId, LayerTrace, RankBand, RankKey, StopReason};
 pub use semantic_index::{
-    SessionSemanticIndex, consume_session_index, ensure_session_index, load_session_index,
-    queue_session_dirty, read_session_pending_hint, session_index_status, session_should_rebuild,
-    session_work_from_disk, session_work_now, write_session_pending_hint,
+    SessionEngineFacts, SessionQueue, SessionSemanticIndex, consume_session_index,
+    ensure_session_index, load_session_index, queue_session_dirty, read_session_pending_hint,
+    read_session_queue, session_corpus_progress, session_has_pending_work, session_index_status,
+    session_should_rebuild, session_work_from_disk, session_work_now,
 };
 pub use sparse::sparse_index_path;
 
@@ -2046,7 +2047,11 @@ mod tests {
             },
         )
         .unwrap();
-        assert_eq!(hits.len(), 1, "sparse whitelist: call matches, result does not");
+        assert_eq!(
+            hits.len(),
+            1,
+            "sparse whitelist: call matches, result does not"
+        );
         assert_eq!(hits[0].item_type, "function_call");
 
         let view = build_agent_view(&reader, &hits, dir.path()).unwrap();
```

## Diff：src/engines/mod.rs（仅我方 session 分支新增测试，文件其余改动属 IDE）
```diff
+    // WP-135: the 30s session tick (router SESSION_INDEX_TICK →
+    // `consume_session_index_work`) gates on the session `queue.json`, not the
+    // deleted `pending_hint.json`, and runs on its own busy flag so the code
+    // tick (`code_index_tick`, `refresh_busy`) cannot starve it.
+    #[test]
+    fn session_tick_drains_queue_without_pending_hint() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        seed_ready_code_index(root);
+        let engines = tick_engines(root);
+        // Ready session index with one row, then a later row queued.
+        let db = root.join(".litecode").join("sessions.db");
+        let lease = crate::session::WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
+        let data = crate::session::SessionData::open(&lease, &db).unwrap();
+        let id = data.create_session("/proj", "default", None).unwrap();
+        data.insert_items(&id, &[crate::types::user_text("first")])
+            .unwrap();
+        let reader = crate::session::SessionDataReader::open(&db);
+        let mut emb = code_search::HashEmbedder;
+        let mut index = session_search::ensure_session_index(root, &reader, &mut emb).unwrap();
+        assert_eq!(
+            engines.consume_session_index_work().unwrap().mode,
+            RefreshAcceptedMode::Incremental,
+            "clean queue: the tick is a no-op"
+        );
+        data.insert_items(&id, &[crate::types::user_text("second")])
+            .unwrap();
+        session_search::queue_session_dirty(root, &reader);
+        assert!(
+            !root
+                .join(".litecode/session-index/pending_hint.json")
+                .exists()
+        );
+        assert_eq!(session_search::read_session_queue(root).queued, 1);
+        // Code tick busy must not block the session tick.
+        engines.refresh_busy.store(true, Ordering::SeqCst);
+        assert_eq!(
+            engines.consume_session_index_work().unwrap().mode,
+            RefreshAcceptedMode::Starting,
+            "queued session work passes the gate (stops only at the test worker)"
+        );
+        engines.refresh_busy.store(false, Ordering::SeqCst);
+        // What the worker runs for that tick: the queue drains to empty.
+        session_search::consume_session_index(root, &reader, &mut emb, &mut index).unwrap();
+        assert_eq!(session_search::read_session_queue(root).queued, 0);
+        assert_eq!(
+            engines.consume_session_index_work().unwrap().mode,
+            RefreshAcceptedMode::Incremental
+        );
+    }
+
```

## Diff：src/engines/status_view.rs（仅 session 调用与适配器替换；其余改动属 IDE）
```diff
-    RetrievalProgress::merge(code, session_corpus_progress(workspace_root))
-/// Temporary adapter until Session reports chunk progress itself. …（原 ready/needs_rebuild 适配器整段删除）
+    RetrievalProgress::merge(code, session_corpus_progress(workspace_root, status))
+}
+
+/// Session corpus progress, reported by `session_search` itself (WP-135; the
+/// temporary ready/needs_rebuild adapter is removed). Same engine facts as the
+/// code leg: a stopped engine is `failed` only when desired.
+fn session_corpus_progress(workspace_root: &Path, status: &EngineStatus) -> CorpusProgress {
+    crate::engines::session_search::session_corpus_progress(
+        workspace_root,
+        crate::engines::session_search::SessionEngineFacts {
+            desired: status.desired,
+            state: status.state,
+        },
+    )
+}
```

## 补记（fmt 复核，01:17 HKT）
- 环境：HOME=$PWD/.tmp/home，unset RUSTUP_HOME/CARGO_HOME，cargo 1.99。
- `cargo fmt --check` 和 `rustfmt --check --edition 2021 src/engines/mod.rs` 在 engines/mod.rs 我方测试 session_tick_drains_queue_without_pending_hint（L1729 起）范围内都没有 diff。第二轮已按 rustfmt 输出手工改过（insert_items 换行、pending_hint.json 断言拆行），本轮无需再改。
- 剩余 diff 全部属于 IDE：code_search/index_status.rs:959/966/985/999，code_search/mod.rs:48，engines/mod.rs:1102（reclaim_stale_index_job 段）。
- 重跑 engines::tests::session_tick_drains_queue_without_pending_hint：1 通过 / 0 失败。
