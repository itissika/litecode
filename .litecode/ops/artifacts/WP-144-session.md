# WP-144 Session：会话语义索引分批提交 / 可恢复构建（第 4 轮）

依据：`.tmp/wp144_plan.md`（已批准）+ 裁定 Q1=(c) 追加式、Q3=构建期可搜索（refreshing）；第 2 轮按 IDE 审阅 `.tmp/wp143/wp144_review.md` 与后端中层退回意见修改。裁定覆盖计划中"整体替换 vectors/chunks"部分。

## 设计
- `vectors.log`（记录 = u64 id LE + f32×EMBED_DIM LE）、`chunks.jsonl`、`source_state.jsonl` 均为追加，每批经 `code_search::append_durable(path, committed_len, bytes)` 写入并 fsync。
- `meta.json` 是唯一原子检查点（`code_search::atomic_write_bytes`），记录 `layout=1`、三个日志已提交长度、`building`、水位线。
- 每批：vectors → chunks → settled → meta。恢复时截断到 meta 长度，最多重做 1 批；usearch 在内存中由 vectors.log 重建。
- 每批写入字节只与批大小相关（计数改用 `code_search::WRITTEN_BYTES`）。
- Q3：`begin_build` 先写空日志 + `building=true` 的 meta，此后首建/重建均可搜索，状态 refreshing + 真实百分比。

## 第 2 轮修改
- **换用 IDE WP-143 v3 API，删除 `mod seam` 和本地 `truncate_to`**：`atomic_write_bytes`、`append_durable`、`sweep_atomic_tmp`、`truncate_to_committed`、`atomic_write_set`、`WRITTEN_BYTES`（仅测试用）。无重复实现。仅保留 `salvage_log`：在 `truncate_to_committed` 失败（文件比检查点短或缺失）时截到最长完整前缀——这是抢救语义，IDE API 不提供。
- **M1 抢救而非整库重建**：文件短于检查点 → 截到最后一个完整记录或行；损坏行 → 跳过；chunks 文件缺失 → 视为空；settled 损坏行 → 跳过（`read_settled_lenient`）。有向量的 chunk 保留；没有向量或丢失 chunk 行的 settled 行走现有的"chunk 无向量"重做路径。抢救后以 `building=true` 重写检查点，只做 diff 重做。`load_session_index` 的删 meta 分支只剩真正无法读取的情况（如 meta 本身损坏），兜底用。
- **M2**：追加改用 `append_durable`（先切掉未提交的尾部）。`commit` 失败时从磁盘 `load` 重新加载内存索引：内存里已有本批 chunk、settled 和删除，单回滚长度会让内存与磁盘不一致，下一轮会漏写本批（测试证明会丢 32 个 chunk）。
- **流式读取向量**：`BufReader` 逐条读记录，只为需要的 id 建 HashMap，不再整文件读入内存。
- **迁移**：三个日志用一次 `atomic_write_set` 写入，之后才写新 meta 和删除旧文件；中途崩溃会以幂等方式重跑。
- 第 2 轮新增测试：
  - `wp144_half_record_tail_is_cut_without_reembedding`
  - `wp144_log_shorter_than_checkpoint_salvages_instead_of_rebuilding`
  - `wp144_corrupt_line_and_missing_chunks_log_salvage_instead_of_rebuilding`
  - `wp144_crash_mid_migration_reruns_without_reembedding`
  - `wp144_commit_after_failed_commit_does_not_count_the_stale_tail`
  - `wp144_legacy_layout_migrates_without_reembedding`

## 第 3 轮修改（IDE 第 2 轮审阅 `.tmp/wp143/wp144_review_r2.md`）
- **R2-1**：只有 meta 本身解析失败时才删除 meta（`check_meta_or_drop`，在 load 前判定，`error!` reason=meta_corrupt）。meta 读取 I/O 失败、日志 I/O 失败、ANN 失败一律保留 meta，`error!` 并标 reason=io/ann，返回 Err，下次 load 重试。
- 行读取改为 `read_until` + `from_utf8`（`for_each_line`）：非 UTF-8 行与其他损坏行一样跳过，归入抢救路径；chunks 和 settled 都用它。
- `session_should_rebuild`：layout 1 下日志文件缺失不再触发重建，交给 load 抢救；只有 legacy 布局仍要求整文件存在。原先 chunks 文件缺失时实际走了整库重建，第 2 轮测试没有发现这个问题，现在测试改为通过 vectors.log 只增不清来证明没有重建。
- **提交失败后重载也失败**：设置 `needs_reload`，下次使用（reconcile 入口）先从磁盘重载；同时 `error!` 带 reason。
- 新增测试：
  - `wp144_invalid_utf8_line_keeps_meta_and_redoes_only_its_rows`
  - `wp144_io_error_on_load_keeps_meta_and_retries`
  - `wp144_corrupt_meta_is_the_only_case_that_drops_it`
  - `wp144_failed_reload_after_failed_commit_forces_reload_on_next_use`
- `salvage_log` 按 IDE 意见留在我方。

## 第 4 轮修改（IDE 第 3 轮审阅 `.tmp/wp143/wp144_review_r3.md`）
- **R3-1**：`session_should_rebuild` 改为 `read_meta` 的 `Ok(None)` → 重建，`Ok(Some)` → 原有判断，`Err` → 不重建。meta 读取 I/O 错误不再经 status/work/queue 走到 `begin_build` 清空日志。解析失败已由 `check_meta_or_drop` 先删除 meta，之后读到 None，仍会正确重建。
- `reload_reason` 的字符串匹配改为类型：`LoadFailure { Io, Ann }` + `LoadError`。`load_tagged` 中 ANN 步骤显式标记 Ann，其余默认 Io（`From<LitecodeError>`）。公开的 `load` 签名不变。
- 迁移失败日志改为 "kept legacy layout; retrying"。
- 新增测试：
  - `wp144_meta_read_io_error_is_not_a_rebuild_and_keeps_the_logs`：注入 meta 读取 I/O 错误后，`session_should_rebuild` 为 false，work 不是 Rebuild，status 不是 NeedsRebuild/Absent，queue 和 worker load 都不清空日志，四个文件长度不变，恢复后零 embed。
  - `wp144_load_failure_reason_is_typed`
- 补充（后端中层要求）：`consume_session_index` 入口在 `read_meta` 出错时直接返回 Err（`error!` reason=io），不对空内存索引做 reconcile。测试 `wp144_consume_with_unreadable_meta_errs_without_embedding`：注入 meta 读取 I/O 错误，用空内存索引调用 consume → 返回 Err，embed 调用 0 次，四个文件长度不变。
- R4-1（IDE `.tmp/wp143/wp144_review_r4.md`）：`queue_session_dirty` 在 `read_meta` 出错时 `warn!` 并立即返回，保留原 queue.json，不再按水位 0 把全部会话标脏（违反"不估算"）。`wp144_meta_read_io_error_is_not_a_rebuild_and_keeps_the_logs` 增加断言：预先写入的 queue.json 在调用后逐字节不变。

## 审阅意见处理
- M1、M2、流式读取、迁移崩溃测试、4 个缺失测试：已修。
- 压缩：按裁定转 ISSUE-058，不在本包。
- 目录 fsync：更正上一轮的说法——第 2 轮时 `append_durable` 并未做目录 fsync；第 3 轮 IDE 已在 atomic.rs 中加入（`append_durable`/`append_fsync` 新建文件时 fsync 父目录），我方无需修改。
- `persist_usearch` 再导出现在只有测试用到，IDE 计划将其 cfg(test) 化，我方没有异议。
- settled 采用追加（del 行）而非整体替换：理由不变，整体替换为 O(索引)，违反每批字节验收。请确认。

## 测试（MemAvailable≥6GiB，-j2，共享 target）
- `cargo test --lib -- session_search wp135 wp144 status_view engines::tests::session_tick engines::tests::code_index_tick`：**209 passed, 0 failed, 1 ignored**（第 3 轮 206）；编译前确认无其他 cargo/rustc 进程。
- `rustfmt --edition 2024 --check`：我方 3 个文件通过。

## Diff
```diff
diff --git a/src/engines/session_search/dense_parity.rs b/src/engines/session_search/dense_parity.rs
index 73ff50f..8a7503f 100644
--- a/src/engines/session_search/dense_parity.rs
+++ b/src/engines/session_search/dense_parity.rs
@@ -10,7 +10,6 @@
 //! that row was already settled, never because the diff forgot it existed.
 //! Append, revert, and a late echo result all land here for that reason.
 
-use std::path::Path;
 use std::sync::Arc;
 
 use tempfile::TempDir;
@@ -69,27 +68,23 @@ impl Fixture {
     }
 
     fn artifacts(&self) -> (Vec<String>, Vec<String>) {
-        (
-            read_lines(&self.index_dir().join("chunks.jsonl"), true),
-            read_lines(&self.index_dir().join("source_state.jsonl"), false),
-        )
+        // WP-144: the logs are append-only, so compare what the index holds
+        // (live chunks + settled rows), not the raw files.
+        let (chunks, settled) = SessionSemanticIndex::live_artifact_lines(self.root.path());
+        (normalize(chunks, true), normalize(settled, false))
     }
 }
 
-/// One artifact file as sorted lines. `strip_id` removes the vector id, which
-/// differs between two equally correct builds (it counts allocations, not rows).
-fn read_lines(path: &Path, strip_id: bool) -> Vec<String> {
-    let Ok(body) = std::fs::read_to_string(path) else {
-        return Vec::new();
-    };
-    let mut lines: Vec<String> = body
-        .lines()
-        .filter(|line| !line.trim().is_empty())
+/// Sorted lines; `strip_id` removes the vector id, which differs between two
+/// equally correct builds (it counts allocations, not rows).
+fn normalize(lines: Vec<String>, strip_id: bool) -> Vec<String> {
+    let mut lines: Vec<String> = lines
+        .into_iter()
         .map(|line| {
             if !strip_id {
-                return line.to_string();
+                return line;
             }
-            let mut value: serde_json::Value = serde_json::from_str(line).unwrap();
+            let mut value: serde_json::Value = serde_json::from_str(&line).unwrap();
             value.as_object_mut().unwrap().remove("id");
             serde_json::to_string(&value).unwrap()
         })
@@ -224,8 +219,10 @@ fn a_legacy_index_migrates_in_one_pass_without_re_embedding() {
     f.insert(&[user_text("alpha row"), user_text("beta row")]);
     f.reconcile();
 
-    // Drop the state, leaving exactly what the previous version wrote.
-    std::fs::remove_file(f.index_dir().join("source_state.jsonl")).unwrap();
+    // Leave exactly what the pre-settled version wrote: whole-file vectors
+    // (pre-WP-144 layout) and no settled state. Loading migrates the layout
+    // without re-embedding, then the legacy pass reuses every vector.
+    SessionSemanticIndex::write_legacy_layout_for_test(f.root.path(), true);
     f.insert(&[user_text("gamma row")]);
 
     let mut emb = Counting::default();
diff --git a/src/engines/session_search/semantic_index.rs b/src/engines/session_search/semantic_index.rs
index 464981c..9efd609 100644
--- a/src/engines/session_search/semantic_index.rs
+++ b/src/engines/session_search/semantic_index.rs
@@ -5,7 +5,7 @@
 use std::collections::{HashMap, HashSet};
 use std::fs::File;
 use std::hash::{Hash, Hasher};
-use std::io::{BufRead, BufReader, Write};
+use std::io::{BufRead, BufReader};
 use std::path::{Path, PathBuf};
 
 use chrono::Utc;
@@ -13,8 +13,9 @@ use serde::{Deserialize, Serialize};
 use usearch::{Index, IndexOptions, MetricKind, ScalarKind};
 
 use crate::engines::code_search::{
-    EMBED_DIM, Embedder, MODEL_ID, PIPELINE_VERSION, persist_usearch, production_embedder_id,
-    restore_usearch,
+    AtomicPart, EMBED_DIM, Embedder, MODEL_ID, PIPELINE_VERSION, append_durable,
+    atomic_write_bytes, atomic_write_set, production_embedder_id, restore_usearch,
+    sweep_atomic_tmp, truncate_to_committed,
 };
 use crate::types::{LitecodeError, Result};
 
@@ -78,6 +79,23 @@ pub struct SessionIndexMeta {
     /// full reconcile, which stores it.
     #[serde(default)]
     pub keys_hash: u64,
+    /// WP-144 on-disk layout: 1 = append-only `vectors.log` / `chunks.jsonl` /
+    /// `source_state.jsonl` with this meta as the checkpoint. 0/absent = the
+    /// old whole-file layout, rebuilt once (resumably) on upgrade.
+    #[serde(default)]
+    pub layout: u32,
+    /// Committed byte lengths of the three append logs. A resume truncates each
+    /// log back to these, so anything after the last checkpoint is redone.
+    #[serde(default)]
+    pub vectors_len: u64,
+    #[serde(default)]
+    pub chunks_len: u64,
+    #[serde(default)]
+    pub settled_len: u64,
+    /// A first build / rebuild is in flight: committed batches are searchable,
+    /// the watermark has not advanced yet.
+    #[serde(default)]
+    pub building: bool,
 }
 
 impl SessionIndexMeta {
@@ -92,10 +110,20 @@ impl SessionIndexMeta {
             last_change_id: 0,
             doc_schema: DOC_SCHEMA,
             keys_hash: 0,
+            layout: LAYOUT,
+            vectors_len: 0,
+            chunks_len: 0,
+            settled_len: 0,
+            building: false,
         }
     }
 }
 
+/// See [`SessionIndexMeta::layout`].
+const LAYOUT: u32 = 1;
+/// One vector record in `vectors.log`: chunk id (u64 LE) + EMBED_DIM f32 LE.
+const VECTOR_RECORD: usize = 8 + 4 * EMBED_DIM;
+
 fn session_index_dir(workspace_root: &Path) -> PathBuf {
     workspace_root.join(".litecode").join("session-index")
 }
@@ -104,7 +132,14 @@ fn meta_path(workspace_root: &Path) -> PathBuf {
     session_index_dir(workspace_root).join("meta.json")
 }
 
+/// Append-only raw vector log (WP-144). usearch can only save whole, so the
+/// durable form is this log and the ANN graph is rebuilt in RAM on load.
 fn vectors_path(workspace_root: &Path) -> PathBuf {
+    session_index_dir(workspace_root).join("vectors.log")
+}
+
+/// Pre-WP-144 whole-file ANN dump; removed when a rebuild starts.
+fn legacy_vectors_path(workspace_root: &Path) -> PathBuf {
     session_index_dir(workspace_root).join("vectors.usearch")
 }
 
@@ -144,6 +179,7 @@ fn needs_rebuild(meta: &SessionIndexMeta) -> bool {
         || meta.embedder_id != production_embedder_id()
         || meta.embed_dim != EMBED_DIM
         || meta.doc_schema != DOC_SCHEMA
+        || meta.layout != LAYOUT
 }
 
 fn read_meta(workspace_root: &Path) -> Result<Option<SessionIndexMeta>> {
@@ -151,6 +187,12 @@ fn read_meta(workspace_root: &Path) -> Result<Option<SessionIndexMeta>> {
     if !path.exists() {
         return Ok(None);
     }
+    #[cfg(test)]
+    if FAIL_META_READS.with(|c| c.get()) {
+        return Err(LitecodeError::Config(
+            "io: injected meta read failure".into(),
+        ));
+    }
     let content =
         std::fs::read_to_string(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
     let meta: SessionIndexMeta = serde_json::from_str(&content)
@@ -158,14 +200,155 @@ fn read_meta(workspace_root: &Path) -> Result<Option<SessionIndexMeta>> {
     Ok(Some(meta))
 }
 
+/// The checkpoint: replaced whole and atomically, after the logs it describes
+/// are appended and fsynced.
 fn write_meta(workspace_root: &Path, meta: &SessionIndexMeta) -> Result<()> {
-    let path = meta_path(workspace_root);
-    if let Some(parent) = path.parent() {
-        std::fs::create_dir_all(parent).map_err(|e| LitecodeError::Config(e.to_string()))?;
+    let body = serde_json::to_vec_pretty(meta).map_err(|e| LitecodeError::Config(e.to_string()))?;
+    atomic_write_bytes(&meta_path(workspace_root), &body)
+}
+
+// ---- WP-144 persistence: IDE's WP-143 atomic/append API ------------------
+//
+// Logs go through `append_durable` (cuts any uncommitted tail first, M2),
+// whole files through `atomic_write_bytes`/`atomic_write_set`, temp leftovers
+// through `sweep_atomic_tmp`, load-time truncation through
+// `truncate_to_committed`. When the checkpoint cannot be honoured (a short
+// file, a torn line), `salvage_log` keeps the longest whole prefix instead of
+// rebuilding (M1, WP-140).
+
+/// Stream a JSONL log line by line (`read_until` + `from_utf8`, R2-1): a
+/// line that is not UTF-8 is skipped and counted like any corrupt line,
+/// blank lines are ignored. A missing file reads as empty; a real I/O error
+/// is returned (and never costs the meta).
+fn for_each_line(path: &Path, mut f: impl FnMut(&str)) -> Result<usize> {
+    let file = match File::open(path) {
+        Ok(file) => file,
+        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
+        Err(e) => {
+            return Err(LitecodeError::Config(format!(
+                "io: {}: {e}",
+                path.display()
+            )));
+        }
+    };
+    let mut reader = BufReader::new(file);
+    let mut buf = Vec::new();
+    let mut bad = 0usize;
+    loop {
+        buf.clear();
+        let n = reader
+            .read_until(b'\n', &mut buf)
+            .map_err(|e| LitecodeError::Config(format!("io: {}: {e}", path.display())))?;
+        if n == 0 {
+            return Ok(bad);
+        }
+        match std::str::from_utf8(&buf) {
+            Ok(line) if line.trim().is_empty() => {}
+            Ok(line) => f(line.trim_end_matches(['\n', '\r'])),
+            Err(_) => bad += 1,
+        }
     }
-    let body =
-        serde_json::to_string_pretty(meta).map_err(|e| LitecodeError::Config(e.to_string()))?;
-    std::fs::write(&path, body).map_err(|e| LitecodeError::Config(e.to_string()))
+}
+
+#[cfg(test)]
+thread_local! {
+    /// Fail the next N `load`s with an injected I/O error.
+    static FAIL_LOADS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
+    /// Fail every `read_meta` with an injected I/O error while set.
+    static FAIL_META_READS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
+}
+
+/// Bring a log to its committed length; if the file is shorter than the
+/// checkpoint (or missing), keep its longest prefix of whole records
+/// (`record` bytes) or whole lines instead. Returns the length kept and
+/// whether salvage was needed.
+fn salvage_log(path: &Path, committed: u64, record: Option<usize>) -> Result<(u64, bool)> {
+    if truncate_to_committed(path, committed).is_ok() {
+        return Ok((committed, false));
+    }
+    let io = |e: std::io::Error| LitecodeError::Config(format!("salvage {}: {e}", path.display()));
+    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
+    let keep = match record {
+        Some(r) => len - len % r as u64,
+        None => {
+            // Last newline, streamed (never the whole file in memory).
+            let mut keep = 0u64;
+            let mut pos = 0u64;
+            if let Ok(file) = File::open(path) {
+                let mut reader = BufReader::new(file);
+                let mut buf = Vec::new();
+                loop {
+                    buf.clear();
+                    let n = reader.read_until(b'\n', &mut buf).map_err(io)?;
+                    if n == 0 {
+                        break;
+                    }
+                    pos += n as u64;
+                    if buf.last() == Some(&b'\n') {
+                        keep = pos;
+                    }
+                }
+            }
+            keep
+        }
+    };
+    let file = std::fs::OpenOptions::new()
+        .create(true)
+        .truncate(false)
+        .write(true)
+        .open(path)
+        .map_err(io)?;
+    file.set_len(keep).map_err(io)?;
+    file.sync_data().map_err(io)?;
+    tracing::warn!(
+        path = %path.display(),
+        committed,
+        kept = keep,
+        "session index log shorter than its checkpoint; salvaged"
+    );
+    Ok((keep, true))
+}
+
+/// One line of the append-only settled log: a row entering the settled set,
+/// or (`op: "del"`) leaving it.
+#[derive(Debug, Clone, Serialize, Deserialize)]
+struct SettledDel {
+    op: String,
+    session_id: String,
+    seq: i64,
+}
+
+#[cfg(test)]
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+enum CrashAt {
+    /// After the vector append of commit N (1-based).
+    AfterVectors(usize),
+    /// After the chunk append of commit N.
+    AfterChunks(usize),
+    /// After the settled append, before the checkpoint of commit N.
+    BeforeCheckpoint(usize),
+    /// After commit N's checkpoint.
+    AfterCommit(usize),
+    /// Migration: logs rewritten, layout-0 meta not yet replaced.
+    MidMigration,
+}
+
+#[cfg(test)]
+thread_local! {
+    static CRASH: std::cell::Cell<Option<CrashAt>> = const { std::cell::Cell::new(None) };
+    static COMMITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
+}
+
+#[cfg(test)]
+fn crash_point(at: fn(usize) -> CrashAt) -> Result<()> {
+    let n = COMMITS.with(|c| c.get());
+    if CRASH.with(|c| c.get()) == Some(at(n)) {
+        return Err(LitecodeError::Config(format!(
+            "injected crash at {:?}",
+            at(n)
+        )));
+    }
+    Ok(())
 }
 
 fn new_ann_index() -> Result<Index> {
@@ -190,6 +373,15 @@ pub struct SessionSemanticIndex {
     /// needs. Empty on an index written before it existed, which is what makes
     /// the first reconcile a one-off full pass.
     settled: HashMap<(String, i64), SettledRow>,
+    /// Committed log lengths (the last checkpoint).
+    vectors_len: u64,
+    chunks_len: u64,
+    settled_len: u64,
+    /// A first build / rebuild has not reached its final checkpoint yet.
+    building: bool,
+    /// Memory ran ahead of disk (a failed commit whose reload failed too):
+    /// the next use reloads from the checkpoint before doing anything.
+    needs_reload: bool,
 }
 
 impl SessionSemanticIndex {
@@ -203,112 +395,438 @@ impl SessionSemanticIndex {
             last_change_id: 0,
             keys_hash: 0,
             settled: HashMap::new(),
+            vectors_len: 0,
+            chunks_len: 0,
+            settled_len: 0,
+            building: false,
+            needs_reload: false,
         })
     }
 
+    /// Load the last checkpoint: truncate every log to its committed length,
+    /// replay the settled log, keep only the chunks a settled row claims, and
+    /// rebuild the ANN graph in RAM from their logged vectors.
     pub fn load(workspace_root: &Path) -> Result<Self> {
-        let chunks_file = chunks_path(workspace_root);
-        let ann = new_ann_index()?;
-        restore_usearch(&ann, &vectors_path(workspace_root))?;
-
-        let meta_on_disk = read_meta(workspace_root)?;
-        let embedder_id = meta_on_disk
-            .as_ref()
-            .map(|m| m.embedder_id.clone())
-            .unwrap_or_else(|| production_embedder_id().into());
-
-        let mut index = Self {
-            chunks: HashMap::new(),
-            by_key: HashMap::new(),
-            ann,
-            next_id: 1,
-            embedder_id,
-            last_change_id: meta_on_disk.as_ref().map(|m| m.last_change_id).unwrap_or(0),
-            keys_hash: meta_on_disk.as_ref().map(|m| m.keys_hash).unwrap_or(0),
-            settled: HashMap::new(),
-        };
-
-        let file = File::open(&chunks_file).map_err(|e| LitecodeError::Config(e.to_string()))?;
-        let reader = BufReader::new(file);
-        for line in reader.lines() {
-            let line = line.map_err(|e| LitecodeError::Config(e.to_string()))?;
-            if line.trim().is_empty() {
-                continue;
+        Self::load_tagged(workspace_root).map_err(|e| e.error)
+    }
+
+    /// `load` with the failure's reason kept as a type (`LoadFailure`), not
+    /// guessed from the message.
+    fn load_tagged(workspace_root: &Path) -> std::result::Result<Self, LoadError> {
+        #[cfg(test)]
+        if FAIL_LOADS.with(|c| {
+            let n = c.get();
+            c.set(n.saturating_sub(1));
+            n > 0
+        }) {
+            return Err(LitecodeError::Config("io: injected load failure".into()).into());
+        }
+        let meta = read_meta(workspace_root)?
+            .ok_or_else(|| LitecodeError::Config("session index meta missing".into()))?;
+        let (vectors_len, v_salvaged) = salvage_log(
+            &vectors_path(workspace_root),
+            meta.vectors_len,
+            Some(VECTOR_RECORD),
+        )?;
+        let (chunks_len, c_salvaged) =
+            salvage_log(&chunks_path(workspace_root), meta.chunks_len, None)?;
+        let (settled_len, s_salvaged) =
+            salvage_log(&settled_path(workspace_root), meta.settled_len, None)?;
+        let mut salvaged = v_salvaged || c_salvaged || s_salvaged;
+
+        let mut index = Self::new_empty().map_err(LoadError::ann)?;
+        index.embedder_id = meta.embedder_id.clone();
+        index.last_change_id = meta.last_change_id;
+        index.keys_hash = meta.keys_hash;
+        index.vectors_len = vectors_len;
+        index.chunks_len = chunks_len;
+        index.settled_len = settled_len;
+        index.building = meta.building;
+        let (settled, bad_settled) = read_settled_lenient(workspace_root)?;
+        index.settled = settled;
+        salvaged |= bad_settled > 0;
+
+        // No settled rows at all = no diff basis (a migrated pre-settled index,
+        // or a build stopped before its first row settled): keep every logged
+        // chunk so the legacy pass can reuse vectors by text instead of
+        // re-embedding.
+        let keep_all = index.settled.is_empty();
+        let live_docs: HashSet<&str> = index
+            .settled
+            .values()
+            .flat_map(|row| row.doc_keys.iter().map(String::as_str))
+            .collect();
+        let mut by_key: HashMap<String, SessionChunk> = HashMap::new();
+        let mut max_id = 0;
+        // A missing chunks log reads as empty (M1): its rows are redone.
+        let mut bad_chunks = 0usize;
+        let bad_utf8 = for_each_line(&chunks_path(workspace_root), |line| {
+            // A torn or corrupt line is skipped, not fatal (M1); rows that
+            // claimed it are redone below.
+            let Ok(chunk) = serde_json::from_str::<SessionChunk>(line) else {
+                bad_chunks += 1;
+                return;
+            };
+            // Ids are never reused, even those of chunks no row claims anymore.
+            max_id = max_id.max(chunk.id);
+            if keep_all || live_docs.contains(chunk.key.as_str()) {
+                by_key.insert(chunk.key.clone(), chunk);
+            }
+        })?;
+        salvaged |= bad_chunks + bad_utf8 > 0;
+        index.next_id = max_id + 1;
+        let wanted: HashSet<u64> = by_key.values().map(|c| c.id).collect();
+
+        // Streamed record by record: peak memory is the wanted vectors only,
+        // never the whole log plus a copy.
+        let mut vectors: HashMap<u64, Vec<f32>> = HashMap::with_capacity(wanted.len());
+        if let Ok(file) = File::open(vectors_path(workspace_root)) {
+            let mut reader = BufReader::with_capacity(1 << 16, file);
+            let mut record = vec![0u8; VECTOR_RECORD];
+            while std::io::Read::read_exact(&mut reader, &mut record).is_ok() {
+                let id = u64::from_le_bytes(record[..8].try_into().expect("8 bytes"));
+                if wanted.contains(&id) {
+                    let vector = record[8..]
+                        .chunks_exact(4)
+                        .map(|b| f32::from_le_bytes(b.try_into().expect("4 bytes")))
+                        .collect();
+                    vectors.insert(id, vector);
+                }
             }
-            let chunk: SessionChunk = serde_json::from_str(&line)
-                .map_err(|e| LitecodeError::Config(format!("parse session chunk: {e}")))?;
-            let id = chunk.id;
-            index.next_id = index.next_id.max(id + 1);
-            index.by_key.insert(chunk.key.clone(), id);
-            index.chunks.insert(id, chunk);
         }
-
-        // A settled state that cannot be read is not a fatal index: dropping it
-        // costs one full pass (which rewrites it), while failing the load would
-        // cost the corpus its search.
-        match read_settled(workspace_root) {
-            Ok(settled) => index.settled = settled,
-            Err(error) => tracing::warn!(error = %error, "session index settled state unreadable"),
+        index.ann.reserve(by_key.len().max(64)).map_err(|e| {
+            LoadError::ann(LitecodeError::Config(format!("session ann reserve: {e}")))
+        })?;
+        for (key, chunk) in by_key {
+            // A claimed chunk without its vector is not searchable: leave it
+            // out, and drop the row's settled state so the next pass redoes it.
+            let Some(vector) = vectors.get(&chunk.id) else {
+                index.settled.retain(|_, row| !row.doc_keys.contains(&key));
+                continue;
+            };
+            index.ann_add(chunk.id, vector).map_err(LoadError::ann)?;
+            index.by_key.insert(key, chunk.id);
+            index.chunks.insert(chunk.id, chunk);
+        }
+        if !keep_all {
+            // A row whose chunk line was lost (salvaged log) is redone too.
+            let before = index.settled.len();
+            let by_key = &index.by_key;
+            index
+                .settled
+                .retain(|_, row| row.doc_keys.iter().all(|k| by_key.contains_key(k)));
+            salvaged |= index.settled.len() != before;
+        }
+        if salvaged {
+            // Re-checkpoint the kept prefix and resume as a build, so the
+            // dropped rows are diffed and redone, never a whole rebuild (M1).
+            index.building = true;
+            index.save_meta(workspace_root)?;
         }
         Ok(index)
     }
 
-    /// Publish everything this index holds.
-    ///
-    /// A reconcile writes only the artifacts whose own content moved, so a pass
-    /// that settles rows without deriving documents does not rewrite the vectors.
-    pub fn save(&self, workspace_root: &Path) -> Result<()> {
-        self.write_vectors(workspace_root)?;
-        self.write_chunks(workspace_root)?;
-        self.write_settled(workspace_root)?;
-        self.save_meta(workspace_root)
+    /// Append one batch and checkpoint it (WP-144): vectors, chunks and settled
+    /// lines are appended and fsynced, then the meta is replaced atomically
+    /// with the new committed lengths. The cost is the batch's own bytes plus
+    /// the small meta, whatever the index already holds. `watermark` is set
+    /// only on the final commit of a pass.
+    fn commit(
+        &mut self,
+        workspace_root: &Path,
+        vectors: &[(u64, Vec<f32>)],
+        chunk_ids: &[u64],
+        settled_lines: &[String],
+        watermark: Option<(i64, u64)>,
+    ) -> Result<()> {
+        // M2: a failed commit must not leave its lengths behind; the next
+        // commit's `append_durable` then cuts the uncommitted tail.
+        let committed = (
+            self.vectors_len,
+            self.chunks_len,
+            self.settled_len,
+            self.last_change_id,
+            self.keys_hash,
+            self.building,
+        );
+        let result =
+            self.commit_inner(workspace_root, vectors, chunk_ids, settled_lines, watermark);
+        if result.is_err() {
+            // The in-memory index already holds this batch (chunks, settled
+            // rows, removals) but the disk does not: reload the last checkpoint
+            // so memory and disk agree and the next pass redoes the batch.
+            // `load` cuts the uncommitted tails; `append_durable` would too.
+            match Self::load_tagged(workspace_root) {
+                Ok(reloaded) => *self = reloaded,
+                Err(error) => {
+                    tracing::error!(
+                        reason = error.reason.as_str(),
+                        error = %error.error,
+                        "session index reload after a failed commit failed; forcing reload on next use"
+                    );
+                    self.needs_reload = true;
+                    (
+                        self.vectors_len,
+                        self.chunks_len,
+                        self.settled_len,
+                        self.last_change_id,
+                        self.keys_hash,
+                        self.building,
+                    ) = committed;
+                }
+            }
+        }
+        result
     }
 
-    fn write_vectors(&self, workspace_root: &Path) -> Result<()> {
-        let dir = session_index_dir(workspace_root);
-        std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
-        persist_usearch(&self.ann, &vectors_path(workspace_root))
+    /// Forced reload after memory ran ahead of disk (see `needs_reload`).
+    fn reload_if_needed(&mut self, workspace_root: &Path) -> Result<()> {
+        if self.needs_reload {
+            *self = Self::load(workspace_root)?;
+        }
+        Ok(())
+    }
+
+    /// Whether the next use must reload from disk first.
+    pub fn needs_reload(&self) -> bool {
+        self.needs_reload
     }
 
-    fn write_chunks(&self, workspace_root: &Path) -> Result<()> {
-        let chunks_file = chunks_path(workspace_root);
-        let mut file =
-            File::create(&chunks_file).map_err(|e| LitecodeError::Config(e.to_string()))?;
-        let mut ids: Vec<u64> = self.chunks.keys().copied().collect();
-        ids.sort_unstable();
-        for id in ids {
-            let chunk = &self.chunks[&id];
-            let line =
-                serde_json::to_string(chunk).map_err(|e| LitecodeError::Config(e.to_string()))?;
-            writeln!(file, "{line}").map_err(|e| LitecodeError::Config(e.to_string()))?;
+    fn commit_inner(
+        &mut self,
+        workspace_root: &Path,
+        vectors: &[(u64, Vec<f32>)],
+        chunk_ids: &[u64],
+        settled_lines: &[String],
+        watermark: Option<(i64, u64)>,
+    ) -> Result<()> {
+        #[cfg(test)]
+        COMMITS.with(|c| c.set(c.get() + 1));
+        let mut vbuf = Vec::with_capacity(vectors.len() * VECTOR_RECORD);
+        for (id, vector) in vectors {
+            vbuf.extend_from_slice(&id.to_le_bytes());
+            for v in vector {
+                vbuf.extend_from_slice(&v.to_le_bytes());
+            }
+        }
+        if !vbuf.is_empty() {
+            self.vectors_len =
+                append_durable(&vectors_path(workspace_root), self.vectors_len, &vbuf)?;
+        }
+        #[cfg(test)]
+        crash_point(CrashAt::AfterVectors)?;
+        let mut cbuf = String::new();
+        for id in chunk_ids {
+            if let Some(chunk) = self.chunks.get(id) {
+                cbuf.push_str(
+                    &serde_json::to_string(chunk)
+                        .map_err(|e| LitecodeError::Config(e.to_string()))?,
+                );
+                cbuf.push('\n');
+            }
+        }
+        if !cbuf.is_empty() {
+            self.chunks_len = append_durable(
+                &chunks_path(workspace_root),
+                self.chunks_len,
+                cbuf.as_bytes(),
+            )?;
+        }
+        #[cfg(test)]
+        crash_point(CrashAt::AfterChunks)?;
+        if !settled_lines.is_empty() {
+            let mut sbuf = settled_lines.join("\n");
+            sbuf.push('\n');
+            self.settled_len = append_durable(
+                &settled_path(workspace_root),
+                self.settled_len,
+                sbuf.as_bytes(),
+            )?;
         }
+        #[cfg(test)]
+        crash_point(CrashAt::BeforeCheckpoint)?;
+        if let Some((latest, keys_hash)) = watermark {
+            self.last_change_id = latest;
+            self.keys_hash = keys_hash;
+            self.building = false;
+        }
+        self.save_meta(workspace_root)?;
+        #[cfg(test)]
+        crash_point(CrashAt::AfterCommit)?;
         Ok(())
     }
 
-    /// Record the watermark without touching the corpus artifacts.
     fn save_meta(&self, workspace_root: &Path) -> Result<()> {
         write_meta(
             workspace_root,
             &SessionIndexMeta {
                 last_change_id: self.last_change_id,
                 keys_hash: self.keys_hash,
+                vectors_len: self.vectors_len,
+                chunks_len: self.chunks_len,
+                settled_len: self.settled_len,
+                building: self.building,
                 ..SessionIndexMeta::shell(&self.embedder_id, self.chunks.len())
             },
         )
     }
 
-    /// One line per settled row, ordered so a read-back is stable.
-    fn write_settled(&self, workspace_root: &Path) -> Result<()> {
-        let path = settled_path(workspace_root);
-        let mut file = File::create(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
-        let mut rows: Vec<&SettledRow> = self.settled.values().collect();
-        rows.sort_by(|a, b| (&a.session_id, a.seq).cmp(&(&b.session_id, b.seq)));
-        for row in rows {
-            let line =
-                serde_json::to_string(row).map_err(|e| LitecodeError::Config(e.to_string()))?;
-            writeln!(file, "{line}").map_err(|e| LitecodeError::Config(e.to_string()))?;
+    /// One-time upgrade of a pre-WP-144 index (whole-file `vectors.usearch`,
+    /// layout 0) to the append-only layout without re-embedding: vectors are
+    /// read back out of the old ANN dump. Idempotent until the new meta lands.
+    fn migrate_legacy_layout(workspace_root: &Path) -> Result<bool> {
+        let Some(meta) = read_meta(workspace_root)? else {
+            return Ok(false);
+        };
+        let compatible_otherwise = meta.pipeline_version == PIPELINE_VERSION
+            && meta.model_id == MODEL_ID
+            && meta.embedder_id == production_embedder_id()
+            && meta.embed_dim == EMBED_DIM
+            && meta.doc_schema == DOC_SCHEMA;
+        let legacy = legacy_vectors_path(workspace_root);
+        if meta.layout != 0 || !compatible_otherwise || !legacy.is_file() {
+            return Ok(false);
         }
-        Ok(())
+        let ann = new_ann_index()?;
+        restore_usearch(&ann, &legacy)?;
+        let body = std::fs::read_to_string(chunks_path(workspace_root)).unwrap_or_default();
+        let mut vbuf = Vec::new();
+        let mut cbuf = String::new();
+        let mut kept = 0usize;
+        let mut vector = vec![0f32; EMBED_DIM];
+        for line in body.lines().filter(|l| !l.trim().is_empty()) {
+            let chunk: SessionChunk = serde_json::from_str(line)
+                .map_err(|e| LitecodeError::Config(format!("parse session chunk: {e}")))?;
+            if ann.get(chunk.id, &mut vector).unwrap_or(0) == 0 {
+                continue;
+            }
+            vbuf.extend_from_slice(&chunk.id.to_le_bytes());
+            for v in &vector {
+                vbuf.extend_from_slice(&v.to_le_bytes());
+            }
+            cbuf.push_str(line);
+            cbuf.push('\n');
+            kept += 1;
+        }
+        // The old settled file is whole rows, which the log format reads as-is.
+        let sbody = std::fs::read(settled_path(workspace_root)).unwrap_or_default();
+        let (vp, cp, sp) = (
+            vectors_path(workspace_root),
+            chunks_path(workspace_root),
+            settled_path(workspace_root),
+        );
+        let put = |bytes: &[u8], w: &mut dyn std::io::Write| {
+            w.write_all(bytes)
+                .map_err(|e| LitecodeError::Config(e.to_string()))
+        };
+        atomic_write_set(&[
+            AtomicPart {
+                target: &vp,
+                write: &|w| put(&vbuf, w),
+            },
+            AtomicPart {
+                target: &cp,
+                write: &|w| put(cbuf.as_bytes(), w),
+            },
+            AtomicPart {
+                target: &sp,
+                write: &|w| put(&sbody, w),
+            },
+        ])?;
+        // The layout-0 meta and the legacy file still stand: a crash here
+        // reruns the (idempotent) migration on the next load.
+        #[cfg(test)]
+        crash_point(|_| CrashAt::MidMigration)?;
+        write_meta(
+            workspace_root,
+            &SessionIndexMeta {
+                last_change_id: meta.last_change_id,
+                keys_hash: meta.keys_hash,
+                vectors_len: vbuf.len() as u64,
+                chunks_len: cbuf.len() as u64,
+                settled_len: sbody.len() as u64,
+                building: false,
+                ..SessionIndexMeta::shell(&meta.embedder_id, kept)
+            },
+        )?;
+        let _ = std::fs::remove_file(&legacy);
+        tracing::info!(
+            chunks = kept,
+            "session index migrated to append-only layout"
+        );
+        Ok(true)
+    }
+
+    /// Test helper: rewrite this index in the pre-WP-144 layout (whole
+    /// `vectors.usearch`, layout 0, no settled state when `drop_settled`).
+    #[cfg(test)]
+    pub(crate) fn write_legacy_layout_for_test(workspace_root: &Path, drop_settled: bool) {
+        let index = Self::load(workspace_root).unwrap();
+        crate::engines::code_search::persist_usearch(
+            &index.ann,
+            &legacy_vectors_path(workspace_root),
+        )
+        .unwrap();
+        let _ = std::fs::remove_file(vectors_path(workspace_root));
+        let mut lines = String::new();
+        for chunk in index.chunks.values() {
+            lines.push_str(&serde_json::to_string(chunk).unwrap());
+            lines.push('\n');
+        }
+        std::fs::write(chunks_path(workspace_root), lines).unwrap();
+        if drop_settled {
+            let _ = std::fs::remove_file(settled_path(workspace_root));
+        }
+        let mut meta = read_meta(workspace_root).unwrap().unwrap();
+        meta.layout = 0;
+        meta.vectors_len = 0;
+        meta.chunks_len = 0;
+        meta.settled_len = 0;
+        std::fs::write(
+            meta_path(workspace_root),
+            serde_json::to_vec(&meta).unwrap(),
+        )
+        .unwrap();
+    }
+
+    /// Start a first build / rebuild (WP-144): empty logs plus a compatible
+    /// meta with the watermark at zero and `building` set. From here the index
+    /// is formal and searchable, and an interrupted build resumes from its last
+    /// checkpoint instead of starting over.
+    fn begin_build(workspace_root: &Path) -> Result<Self> {
+        let dir = session_index_dir(workspace_root);
+        std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
+        let _ = std::fs::remove_file(legacy_vectors_path(workspace_root));
+        for path in [
+            vectors_path(workspace_root),
+            chunks_path(workspace_root),
+            settled_path(workspace_root),
+        ] {
+            atomic_write_bytes(&path, b"")?;
+        }
+        let mut index = Self::new_empty()?;
+        index.building = true;
+        index.save_meta(workspace_root)?;
+        Ok(index)
+    }
+
+    /// Test view of what the index *holds* (the append logs also carry
+    /// retired lines until the next build): one JSON line per live chunk and
+    /// per settled row.
+    #[cfg(test)]
+    pub(crate) fn live_artifact_lines(workspace_root: &Path) -> (Vec<String>, Vec<String>) {
+        let index = Self::load(workspace_root).expect("load");
+        let chunks = index
+            .chunks
+            .values()
+            .map(|c| serde_json::to_string(c).unwrap())
+            .collect();
+        let settled = index
+            .settled
+            .values()
+            .map(|r| serde_json::to_string(r).unwrap())
+            .collect();
+        (chunks, settled)
     }
 
     pub fn len(&self) -> usize {
@@ -343,9 +861,9 @@ impl SessionSemanticIndex {
         &mut self,
         chunks: Vec<SessionChunk>,
         embedder: &mut dyn Embedder,
-    ) -> Result<()> {
+    ) -> Result<Vec<(u64, Vec<f32>)>> {
         if chunks.is_empty() {
-            return Ok(());
+            return Ok(Vec::new());
         }
         let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
         let vectors = embedder.embed_batch(&texts)?;
@@ -354,12 +872,14 @@ impl SessionSemanticIndex {
                 "session chunks/vectors length mismatch".into(),
             ));
         }
+        let mut added = Vec::with_capacity(chunks.len());
         for (chunk, vec) in chunks.into_iter().zip(vectors) {
             self.ann_add(chunk.id, &vec)?;
             self.by_key.insert(chunk.key.clone(), chunk.id);
+            added.push((chunk.id, vec));
             self.chunks.insert(chunk.id, chunk);
         }
-        Ok(())
+        Ok(added)
     }
 
     fn ann_search(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>> {
@@ -475,6 +995,7 @@ impl SessionSemanticIndex {
         // marks this root in flight for stale-record recovery; any error inside
         // turns the record into `failed` instead of leaving `running` behind.
         let _in_flight = InFlightGuard::enter(workspace_root);
+        self.reload_if_needed(workspace_root)?;
         match self.reconcile_inner(reader, workspace_root, embedder) {
             Ok(changed) => Ok(changed),
             Err(error) => {
@@ -493,7 +1014,7 @@ impl SessionSemanticIndex {
         let live = reader.searchable_keys_blocking(None)?;
         let latest = reader.latest_change_id_blocking().unwrap_or(0);
         let keys_hash = hash_keys(&live);
-        if self.last_change_id == latest && self.keys_hash == keys_hash {
+        if !self.building && self.last_change_id == latest && self.keys_hash == keys_hash {
             clear_session_job(workspace_root);
             write_session_queue_counts(workspace_root, 0, 0);
             return Ok(false);
@@ -519,11 +1040,20 @@ impl SessionSemanticIndex {
         // A row the corpus no longer holds gives its documents back, and its
         // settled state goes with them. The row said what it owned when it was
         // derived, so this needs no corpus read.
+        let mut settled_lines: Vec<String> = Vec::new();
         for key in &to_remove {
             if let Some(previous) = self.settled.remove(key) {
                 for doc_key in &previous.doc_keys {
                     changed |= self.remove_doc(doc_key);
                 }
+                settled_lines.push(
+                    serde_json::to_string(&SettledDel {
+                        op: "del".into(),
+                        session_id: key.0.clone(),
+                        seq: key.1,
+                    })
+                    .map_err(|e| LitecodeError::Config(e.to_string()))?,
+                );
             }
         }
 
@@ -534,6 +1064,10 @@ impl SessionSemanticIndex {
         // `done` never steps down across queued → scanning (M1).
         write_session_running(workspace_root, self.chunks.len() as u64, None);
 
+        // Rows waiting for their chunks to commit: (staged chunks up to and
+        // including this row's, row). Committed strictly in row order.
+        let mut pending: Vec<(usize, SettledRow)> = Vec::new();
+        let mut next_pending = 0;
         if !to_add.is_empty() {
             let rows = reader.searchable_rows_for_blocking(&to_add)?;
             let mut staged: Vec<SessionChunk> = Vec::new();
@@ -585,8 +1119,11 @@ impl SessionSemanticIndex {
                     derived_keys.extend(derived.docs.iter().map(|doc| doc.key.clone()));
                 }
                 changed |= self.stage_row_docs(&previous, &derived.docs, &mut staged);
-                self.settled.insert(
-                    (row.session_id.clone(), row.seq),
+                // WP-144: a row enters the settled set (the checkpoint) only
+                // with the commit that holds its last staged chunk, so a
+                // resume never finds a half-settled row.
+                pending.push((
+                    staged.len(),
                     SettledRow {
                         session_id: row.session_id.clone(),
                         seq: row.seq,
@@ -594,7 +1131,7 @@ impl SessionSemanticIndex {
                         session_read_call: derived.session_read_call,
                         doc_keys: derived.docs.iter().map(|doc| doc.key.clone()).collect(),
                     },
-                );
+                ));
             }
             // One embed call per batch, not per document: the delta is small but
             // the first pass over an existing index is not, and a session run per
@@ -604,12 +1141,35 @@ impl SessionSemanticIndex {
             let mut done = self.chunks.len() as u64;
             let total = done + staged.len() as u64;
             write_session_running(workspace_root, done, Some(total));
+            let mut batch_end = 0;
             for batch in staged.chunks(EMBED_BATCH) {
-                self.embed_and_add(batch.to_vec(), embedder)?;
+                let vectors = self.embed_and_add(batch.to_vec(), embedder)?;
+                batch_end += batch.len();
+                let ids: Vec<u64> = batch.iter().map(|c| c.id).collect();
+                while next_pending < pending.len() && pending[next_pending].0 <= batch_end {
+                    let row = pending[next_pending].1.clone();
+                    settled_lines.push(
+                        serde_json::to_string(&row)
+                            .map_err(|e| LitecodeError::Config(e.to_string()))?,
+                    );
+                    self.settled.insert((row.session_id.clone(), row.seq), row);
+                    next_pending += 1;
+                }
+                // WP-144: one append + checkpoint per embed batch.
+                self.commit(workspace_root, &vectors, &ids, &settled_lines, None)?;
+                settled_lines.clear();
                 done += batch.len() as u64;
                 write_session_running(workspace_root, done, Some(total));
             }
         }
+        // Rows with nothing to embed (policy drops, echo copies) and any
+        // removals not yet checkpointed go with the final commit.
+        for (_, row) in pending.drain(next_pending..) {
+            settled_lines.push(
+                serde_json::to_string(&row).map_err(|e| LitecodeError::Config(e.to_string()))?,
+            );
+            self.settled.insert((row.session_id.clone(), row.seq), row);
+        }
 
         if legacy {
             // The pre-`settled` index is not evidence of anything: this pass read
@@ -626,8 +1186,6 @@ impl SessionSemanticIndex {
             }
         }
 
-        self.last_change_id = latest;
-        self.keys_hash = keys_hash;
         self.embedder_id = embedder.embedder_id().into();
         // Nothing derived is not nothing to record: the watermark is what retires
         // the pending hint, and rewriting the vectors for it would be the whole
@@ -637,14 +1195,16 @@ impl SessionSemanticIndex {
         // the watermark is what retires the pending hint. Rewriting the vectors
         // for either would be the whole corpus of work for none of the corpus of
         // change, so each artifact is written only when its own content moved.
-        if changed {
-            self.write_vectors(workspace_root)?;
-            self.write_chunks(workspace_root)?;
-        }
-        if changed || settled_changed {
-            self.write_settled(workspace_root)?;
-        }
-        self.save_meta(workspace_root)?;
+        // WP-144: the final checkpoint advances the watermark (and ends a
+        // build); everything before it is already durable batch by batch.
+        let _ = settled_changed;
+        self.commit(
+            workspace_root,
+            &[],
+            &[],
+            &settled_lines,
+            Some((latest, keys_hash)),
+        )?;
         clear_session_job(workspace_root);
         write_session_queue_counts(workspace_root, 0, 0);
         Ok(changed)
@@ -725,26 +1285,42 @@ fn hash_keys(keys: &[(String, i64)]) -> u64 {
 /// Read the settled state, one row per line. `Ok(empty)` when it was never
 /// written; an unreadable file is an error the caller may recover from.
 fn read_settled(workspace_root: &Path) -> Result<HashMap<(String, i64), SettledRow>> {
-    let path = settled_path(workspace_root);
-    if !path.is_file() {
-        return Ok(HashMap::new());
-    }
-    let file = File::open(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
+    read_settled_lenient(workspace_root).map(|(settled, _)| settled)
+}
+
+/// Replay the settled log; an unparsable line is skipped and counted (M1),
+/// its row simply redone.
+fn read_settled_lenient(
+    workspace_root: &Path,
+) -> Result<(HashMap<(String, i64), SettledRow>, usize)> {
     let mut settled = HashMap::new();
-    for line in BufReader::new(file).lines() {
-        let line = line.map_err(|e| LitecodeError::Config(e.to_string()))?;
-        if line.trim().is_empty() {
-            continue;
+    let mut bad = 0usize;
+    let path = settled_path(workspace_root);
+    // Append-only log (WP-144): a row line settles the row, a `del` line
+    // retires it; replayed in order.
+    let bad_utf8 = for_each_line(&path, |line| {
+        if line.contains("\"op\":\"del\"") {
+            match serde_json::from_str::<SettledDel>(line) {
+                Ok(del) => {
+                    settled.remove(&(del.session_id, del.seq));
+                }
+                Err(_) => bad += 1,
+            }
+            return;
         }
-        let row: SettledRow = serde_json::from_str(&line)
-            .map_err(|e| LitecodeError::Config(format!("parse settled row: {e}")))?;
-        settled.insert((row.session_id.clone(), row.seq), row);
-    }
-    Ok(settled)
+        match serde_json::from_str::<SettledRow>(line) {
+            Ok(row) => {
+                settled.insert((row.session_id.clone(), row.seq), row);
+            }
+            Err(_) => bad += 1,
+        }
+    })?;
+    Ok((settled, bad + bad_utf8))
 }
 
 fn index_files_exist(workspace_root: &Path) -> bool {
     vectors_path(workspace_root).is_file() && chunks_path(workspace_root).is_file()
+        || legacy_vectors_path(workspace_root).is_file()
 }
 
 // ---- WP-135: session job record -------------------------------------------
@@ -787,30 +1363,10 @@ fn read_session_job(workspace_root: &Path) -> Option<SessionJobFile> {
     serde_json::from_str(&content).ok()
 }
 
-/// Write-to-temp-then-rename in the same directory, so a poll never reads a
-/// half-written record.
-fn write_json_atomic(path: &Path, value: &impl Serialize) {
-    let Some(parent) = path.parent() else {
-        return;
-    };
-    let _ = std::fs::create_dir_all(parent);
-    let Ok(body) = serde_json::to_string_pretty(value) else {
-        return;
-    };
-    let name = path
-        .file_name()
-        .and_then(|n| n.to_str())
-        .unwrap_or("record");
-    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
-    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
-    let tmp = parent.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));
-    if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, path).is_err() {
-        let _ = std::fs::remove_file(&tmp);
-    }
-}
-
 fn write_session_job(workspace_root: &Path, job: &SessionJobFile) {
-    write_json_atomic(&session_job_path(workspace_root), job);
+    if let Ok(body) = serde_json::to_vec_pretty(job) {
+        let _ = atomic_write_bytes(&session_job_path(workspace_root), &body);
+    }
 }
 
 /// Roots with a reconcile running in this process (the worker that owns the
@@ -913,7 +1469,9 @@ fn write_session_queue(workspace_root: &Path, queue: SessionQueue) {
         let _ = std::fs::remove_file(&path);
         return;
     }
-    write_json_atomic(&path, &queue);
+    if let Ok(body) = serde_json::to_vec_pretty(&queue) {
+        let _ = atomic_write_bytes(&path, &body);
+    }
 }
 
 /// Exact signal for status and the periodic tick: queued work on disk.
@@ -1059,11 +1617,20 @@ pub fn session_corpus_progress(
 
 /// Absent / unloadable / embedder mismatch — warmup auto-rebuilds this corpus only.
 pub fn session_should_rebuild(workspace_root: &Path) -> bool {
-    let meta = read_meta(workspace_root).ok().flatten();
     let files = index_files_exist(workspace_root);
-    match meta {
-        None => true,
-        Some(m) => needs_rebuild(&m) || !files,
+    // R3-1: an unreadable meta is not a missing one. A parse failure was
+    // already turned into a deleted meta by `check_meta_or_drop` (-> None ->
+    // rebuild); a pure I/O error must never send `begin_build` to wipe the
+    // logs, so it reads as "no rebuild" and the load retries.
+    match read_meta(workspace_root) {
+        Err(_) => false,
+        Ok(None) => true,
+        Ok(Some(m)) =>
+        // Layout 1 logs may be missing or short: `load` salvages them (M1).
+        // Only a legacy index needs its whole-file vectors to exist.
+        {
+            needs_rebuild(&m) || (m.layout == 0 && !files)
+        }
     }
 }
 
@@ -1139,10 +1706,98 @@ pub fn load_session_index(workspace_root: &Path) -> Result<SessionSemanticIndex>
     recover_stale_session_job(workspace_root);
     let dir = session_index_dir(workspace_root);
     std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
+    // Worker load only: leftovers of an interrupted atomic write.
+    sweep_atomic_tmp(&dir);
+    check_meta_or_drop(workspace_root)?;
+    if let Err(error) = SessionSemanticIndex::migrate_legacy_layout(workspace_root) {
+        tracing::warn!(error = %error, "session index layout migration failed; kept legacy layout; retrying");
+    }
     if session_should_rebuild(workspace_root) {
         return SessionSemanticIndex::new_empty();
     }
-    SessionSemanticIndex::load(workspace_root)
+    match SessionSemanticIndex::load_tagged(workspace_root) {
+        Ok(index) => Ok(index),
+        Err(LoadError { reason, error }) => {
+            // R2-1: only an unparsable meta (handled above) costs the meta.
+            // Anything else (I/O, ANN) keeps the checkpoint and is retried on
+            // the next load.
+            tracing::error!(
+                reason = reason.as_str(),
+                error = %error,
+                "session index load failed; checkpoint kept, retrying next load"
+            );
+            Err(error)
+        }
+    }
+}
+
+/// Why a load failed (R2-1). `MetaCorrupt` is decided before `load`, from
+/// the meta bytes themselves (`check_meta_or_drop`); only it costs the meta.
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+enum LoadFailure {
+    Io,
+    Ann,
+}
+
+impl LoadFailure {
+    fn as_str(self) -> &'static str {
+        match self {
+            LoadFailure::Io => "io",
+            LoadFailure::Ann => "ann",
+        }
+    }
+}
+
+#[derive(Debug)]
+struct LoadError {
+    reason: LoadFailure,
+    error: LitecodeError,
+}
+
+impl LoadError {
+    fn ann(error: LitecodeError) -> Self {
+        LoadError {
+            reason: LoadFailure::Ann,
+            error,
+        }
+    }
+}
+
+/// Any untagged failure inside `load_tagged` is a read/write one.
+impl From<LitecodeError> for LoadError {
+    fn from(error: LitecodeError) -> Self {
+        LoadError {
+            reason: LoadFailure::Io,
+            error,
+        }
+    }
+}
+
+/// R2-1: the meta is deleted only when its bytes do not parse. A read error
+/// keeps it (returned as Err, retried next load).
+fn check_meta_or_drop(workspace_root: &Path) -> Result<()> {
+    let path = meta_path(workspace_root);
+    match std::fs::read(&path) {
+        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
+        Err(e) => {
+            tracing::error!(reason = "io", error = %e, "session index meta unreadable; kept");
+            Err(LitecodeError::Config(format!(
+                "io: {}: {e}",
+                path.display()
+            )))
+        }
+        Ok(bytes) => {
+            if let Err(e) = serde_json::from_slice::<SessionIndexMeta>(&bytes) {
+                tracing::error!(
+                    reason = "meta_corrupt",
+                    error = %e,
+                    "session index meta unparsable; dropping it and rebuilding"
+                );
+                let _ = std::fs::remove_file(&path);
+            }
+            Ok(())
+        }
+    }
 }
 
 /// Compare `sessions.db` watermark to the on-disk session index and write the
@@ -1157,7 +1812,15 @@ pub fn queue_session_dirty(workspace_root: &Path, reader: &SessionDataReader) {
     // cannot see the worker's in-flight state (WP-135 re-review). Recovery
     // happens only when the worker loads the index (`load_session_index`).
     let latest = reader.latest_change_id_blocking().unwrap_or(0);
-    let meta = read_meta(workspace_root).ok().flatten();
+    // R4-1: an unreadable meta has no watermark; treating it as 0 would mark
+    // every session dirty (an estimate). Keep the previous queue.json.
+    let meta = match read_meta(workspace_root) {
+        Ok(meta) => meta,
+        Err(error) => {
+            tracing::warn!(error = %error, "session index meta unreadable; queue.json kept");
+            return;
+        }
+    };
     let indexed = meta.as_ref().map_or(0, |m| m.last_change_id);
     let indexed_chunks = meta.as_ref().map_or(0, |m| m.indexed_chunks as u64);
     if latest == indexed {
@@ -1212,18 +1875,25 @@ pub fn consume_session_index(
     embedder: &mut dyn Embedder,
     index: &mut SessionSemanticIndex,
 ) -> Result<bool> {
+    // An unreadable meta says nothing about what is indexed: reconciling an
+    // (empty) in-memory index against it would re-embed everything. Stop and
+    // let the next load retry; only `check_meta_or_drop` may drop a meta.
+    if let Err(error) = read_meta(workspace_root) {
+        tracing::error!(
+            reason = "io",
+            error = %error,
+            "session index meta unreadable; consume skipped, retrying next load"
+        );
+        return Err(error);
+    }
     if session_should_rebuild(workspace_root) {
-        tracing::info!("session_search rebuilding semantic index");
-        // Only this lane's own marker: the directory also holds the sparse
-        // lane's `sparse.db`, and deleting the directory took that index with
-        // it, leaving a search to rebuild 282MB in its own thread. An absent
-        // `meta.json` is what makes `session_should_rebuild` true, the stale
-        // `chunks.jsonl` / `vectors.usearch` are never loaded without it
-        // (`load_session_index`), and `save` overwrites both.
-        let _ = std::fs::remove_file(meta_path(workspace_root));
-        let dir = session_index_dir(workspace_root);
-        std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
-        *index = SessionSemanticIndex::new_empty()?;
+        // WP-144: no more "delete meta, recompute everything in one go". The
+        // new formal index (empty logs + compatible meta, watermark 0,
+        // `building`) is written first; the build then commits batch by batch
+        // and an interruption resumes from the last checkpoint. The sparse
+        // lane's `sparse.db` in the same directory is untouched.
+        tracing::info!("session_search building semantic index (resumable)");
+        *index = SessionSemanticIndex::begin_build(workspace_root)?;
     }
     // `reconcile` itself turns any failure into a `failed` record (M2).
     index.reconcile(reader, workspace_root, embedder)
@@ -1272,7 +1942,7 @@ mod tests {
         assert!(!hits.is_empty());
         assert!(hits[0].summary.contains("semantic marker"));
 
-        index.save(root).unwrap();
+        // Already durable batch by batch (WP-144): no separate save step.
         let loaded = SessionSemanticIndex::load(root).unwrap();
         assert_eq!(loaded.len(), index.len());
     }
@@ -2022,4 +2692,725 @@ mod tests {
         assert_eq!(p.chunks_total, None, "no total before cutting");
         assert_eq!(p.chunks_pending, 0, "no chunk estimate from rows");
     }
+
+    // ---- WP-144: resumable build ------------------------------------------
+
+    struct CountingEmbedder {
+        texts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
+        /// Panic on this (1-based) embed call, to simulate a crash.
+        panic_on_call: Option<usize>,
+        calls: usize,
+    }
+
+    impl CountingEmbedder {
+        fn new() -> (Self, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
+            let texts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
+            (
+                Self {
+                    texts: texts.clone(),
+                    panic_on_call: None,
+                    calls: 0,
+                },
+                texts,
+            )
+        }
+    }
+
+    impl Embedder for CountingEmbedder {
+        fn embedder_id(&self) -> &'static str {
+            HashEmbedder.embedder_id()
+        }
+        fn inference_device(&self) -> &'static str {
+            "hash"
+        }
+        fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
+            self.calls += 1;
+            if self.panic_on_call == Some(self.calls) {
+                panic!("simulated crash in embed call {}", self.calls);
+            }
+            self.texts
+                .fetch_add(texts.len(), std::sync::atomic::Ordering::SeqCst);
+            HashEmbedder.embed_batch(texts)
+        }
+    }
+
+    fn committed_chunks(root: &Path) -> u64 {
+        read_meta(root).unwrap().unwrap().indexed_chunks as u64
+    }
+
+    fn reset_crash(at: Option<CrashAt>) {
+        CRASH.with(|c| c.set(at));
+        COMMITS.with(|c| c.set(0));
+    }
+
+    /// Crash at `at` during a first build, then resume: the resume embeds
+    /// exactly the chunks after the last checkpoint, and the result equals a
+    /// clean build.
+    fn crash_then_resume(at: CrashAt, expect_committed_batches: u64) {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 100);
+        let reader = crate::session::SessionDataReader::open(&db);
+
+        reset_crash(Some(at));
+        let (mut emb, _) = CountingEmbedder::new();
+        assert!(
+            ensure_session_index(root, &reader, &mut emb).is_err(),
+            "{at:?}"
+        );
+        reset_crash(None);
+        let committed = committed_chunks(root);
+        assert_eq!(
+            committed,
+            expect_committed_batches * EMBED_BATCH as u64,
+            "{at:?}"
+        );
+        assert!(
+            !session_should_rebuild(root),
+            "a build in flight is not a rebuild need"
+        );
+
+        let (mut emb, texts) = CountingEmbedder::new();
+        let index = ensure_session_index(root, &reader, &mut emb).unwrap();
+        let total = index.len() as u64;
+        assert!(total >= 100);
+        assert_eq!(
+            texts.load(std::sync::atomic::Ordering::SeqCst) as u64,
+            total - committed,
+            "{at:?}: only chunks after the checkpoint are embedded again"
+        );
+        assert!(!read_meta(root).unwrap().unwrap().building);
+
+        // Nothing lost or duplicated: equal to a clean build of the same rows.
+        let (chunks, settled) = SessionSemanticIndex::live_artifact_lines(root);
+        assert_eq!(chunks.len() as u64, total);
+        let keys: HashSet<String> = chunks
+            .iter()
+            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["key"].to_string())
+            .collect();
+        assert_eq!(keys.len() as u64, total, "no duplicate documents");
+        let live = reader.searchable_keys_blocking(None).unwrap();
+        assert_eq!(settled.len(), live.len(), "every row settled once");
+        let hits = index
+            .search(
+                &HashEmbedder
+                    .embed_one("row number 7 about topic 7")
+                    .unwrap(),
+                5,
+                None,
+            )
+            .unwrap();
+        assert!(!hits.is_empty());
+    }
+
+    #[test]
+    fn wp144_crash_after_commit_resumes_without_reembed() {
+        crash_then_resume(CrashAt::AfterCommit(2), 2);
+    }
+
+    #[test]
+    fn wp144_crash_between_vectors_and_chunks_redoes_one_batch() {
+        crash_then_resume(CrashAt::AfterVectors(2), 1);
+    }
+
+    #[test]
+    fn wp144_crash_between_chunks_and_settled_redoes_one_batch() {
+        crash_then_resume(CrashAt::AfterChunks(2), 1);
+    }
+
+    #[test]
+    fn wp144_crash_before_checkpoint_redoes_one_batch() {
+        crash_then_resume(CrashAt::BeforeCheckpoint(2), 1);
+    }
+
+    #[test]
+    fn wp144_panic_mid_build_resumes_from_checkpoint() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 100);
+        let reader = crate::session::SessionDataReader::open(&db);
+        reset_crash(None);
+        let (mut emb, _) = CountingEmbedder::new();
+        emb.panic_on_call = Some(3);
+        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
+            ensure_session_index(root, &reader, &mut emb)
+        }));
+        assert!(crashed.is_err());
+        assert_eq!(committed_chunks(root), 2 * EMBED_BATCH as u64);
+        // The panic left `running` behind; the worker load clears it.
+        let (mut emb, texts) = CountingEmbedder::new();
+        let index = ensure_session_index(root, &reader, &mut emb).unwrap();
+        assert_eq!(
+            texts.load(std::sync::atomic::Ordering::SeqCst) as u64,
+            index.len() as u64 - 2 * EMBED_BATCH as u64
+        );
+    }
+
+    /// Child half of the SIGKILL test: only acts when the parent set the env.
+    #[test]
+    fn wp144_sigkill_child_build() {
+        let Ok(root) = std::env::var("WP144_CHILD_ROOT") else {
+            return;
+        };
+        let root = std::path::PathBuf::from(root);
+        let kill_after: usize = std::env::var("WP144_KILL_AFTER").unwrap().parse().unwrap();
+        struct Stall {
+            root: std::path::PathBuf,
+            kill_after: usize,
+            calls: usize,
+        }
+        impl Embedder for Stall {
+            fn embedder_id(&self) -> &'static str {
+                HashEmbedder.embedder_id()
+            }
+            fn inference_device(&self) -> &'static str {
+                "hash"
+            }
+            fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
+                self.calls += 1;
+                if self.calls == self.kill_after + 1 {
+                    // `kill_after` batches are committed: tell the parent, wait
+                    // to be killed.
+                    std::fs::write(self.root.join("wp144.marker"), b"ready").unwrap();
+                    std::thread::sleep(std::time::Duration::from_secs(600));
+                }
+                HashEmbedder.embed_batch(texts)
+            }
+        }
+        let reader =
+            crate::session::SessionDataReader::open(&root.join(".litecode").join("sessions.db"));
+        let mut emb = Stall {
+            root: root.clone(),
+            kill_after,
+            calls: 0,
+        };
+        let _ = ensure_session_index(&root, &reader, &mut emb);
+    }
+
+    #[cfg(unix)]
+    #[test]
+    fn wp144_sigkill_mid_build_resumes_without_reembed() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 100);
+        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
+            .args([
+                "--exact",
+                "engines::session_search::semantic_index::tests::wp144_sigkill_child_build",
+                "--nocapture",
+                "--test-threads=1",
+            ])
+            .env("WP144_CHILD_ROOT", root)
+            .env("WP144_KILL_AFTER", "2")
+            .stdout(std::process::Stdio::null())
+            .stderr(std::process::Stdio::null())
+            .spawn()
+            .unwrap();
+        let marker = root.join("wp144.marker");
+        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
+        while !marker.exists() {
+            assert!(
+                std::time::Instant::now() < deadline,
+                "child never reached batch 3"
+            );
+            std::thread::sleep(std::time::Duration::from_millis(50));
+        }
+        child.kill().unwrap(); // SIGKILL
+        let _ = child.wait();
+
+        assert_eq!(committed_chunks(root), 2 * EMBED_BATCH as u64);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let (mut emb, texts) = CountingEmbedder::new();
+        let index = ensure_session_index(root, &reader, &mut emb).unwrap();
+        assert_eq!(
+            texts.load(std::sync::atomic::Ordering::SeqCst) as u64,
+            index.len() as u64 - 2 * EMBED_BATCH as u64,
+            "killed build resumes from its last checkpoint"
+        );
+        assert_eq!(
+            SessionSemanticIndex::live_artifact_lines(root).1.len(),
+            reader.searchable_keys_blocking(None).unwrap().len()
+        );
+    }
+
+    /// Acceptance 3 (session): bytes written for one batch do not depend on
+    /// how much the index already holds.
+    #[test]
+    fn wp144_bytes_per_batch_independent_of_index_size() {
+        fn bytes_for_one_batch(existing: usize) -> u64 {
+            let dir = TempDir::new().unwrap();
+            let root = dir.path();
+            let (data, id, db, _lease) = seeded(root, existing);
+            let reader = crate::session::SessionDataReader::open(&db);
+            let mut index = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
+            for i in 0..EMBED_BATCH {
+                data.insert_items(&id, &[user_text(&format!("fresh row {i:04} text"))])
+                    .unwrap();
+            }
+            crate::engines::code_search::WRITTEN_BYTES.with(|b| b.set(0));
+            consume_session_index(root, &reader, &mut HashEmbedder, &mut index).unwrap();
+            crate::engines::code_search::WRITTEN_BYTES.with(|b| b.get())
+        }
+        let small = bytes_for_one_batch(5);
+        let large = bytes_for_one_batch(400);
+        let payload = (EMBED_BATCH * VECTOR_RECORD) as u64;
+        assert!(small >= payload, "the batch's vectors are written: {small}");
+        assert!(
+            small.abs_diff(large) * 20 < small,
+            "per-batch bytes must not grow with the index: small={small} large={large}"
+        );
+        assert!(
+            large < payload * 2,
+            "about the batch's own size: {large} vs {payload}"
+        );
+    }
+
+    #[test]
+    fn wp144_build_is_searchable_and_refreshing_while_in_progress() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 100);
+        let reader = crate::session::SessionDataReader::open(&db);
+        struct Peek {
+            root: std::path::PathBuf,
+            calls: usize,
+            seen: Vec<(usize, CorpusProgress, bool)>,
+        }
+        impl Embedder for Peek {
+            fn embedder_id(&self) -> &'static str {
+                HashEmbedder.embedder_id()
+            }
+            fn inference_device(&self) -> &'static str {
+                "hash"
+            }
+            fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
+                self.calls += 1;
+                // What a reader of the disk sees mid-build.
+                let on_disk = SessionSemanticIndex::load(&self.root).unwrap().len();
+                let rebuild = session_should_rebuild(&self.root);
+                self.seen.push((
+                    on_disk,
+                    session_corpus_progress(&self.root, warm()),
+                    rebuild,
+                ));
+                HashEmbedder.embed_batch(texts)
+            }
+        }
+        let mut emb = Peek {
+            root: root.to_path_buf(),
+            calls: 0,
+            seen: Vec::new(),
+        };
+        let index = ensure_session_index(root, &reader, &mut emb).unwrap();
+        let total = index.len() as u64;
+        let mut last = 0;
+        for (i, (on_disk, p, rebuild)) in emb.seen.iter().enumerate() {
+            assert!(
+                !rebuild,
+                "never NeedsRebuild once the empty index is written"
+            );
+            assert_eq!(p.state, CorpusState::Refreshing, "{p:?}");
+            assert_eq!(
+                *on_disk,
+                i * EMBED_BATCH,
+                "committed batches are searchable"
+            );
+            assert_eq!(p.chunks_total, Some(total));
+            assert!(p.chunks_done >= last);
+            last = p.chunks_done;
+        }
+        assert_eq!(
+            session_corpus_progress(root, warm()),
+            CorpusProgress::ready(total, 0)
+        );
+    }
+
+    #[test]
+    fn wp144_resume_progress_starts_from_committed_and_is_monotonic() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 100);
+        let reader = crate::session::SessionDataReader::open(&db);
+        reset_crash(Some(CrashAt::AfterCommit(2)));
+        assert!(ensure_session_index(root, &reader, &mut HashEmbedder).is_err());
+        reset_crash(None);
+        let committed = committed_chunks(root);
+        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
+        let mut emb = ObservingEmbedder {
+            root: root.to_path_buf(),
+            seen: seen.clone(),
+            fail: false,
+        };
+        let index = ensure_session_index(root, &reader, &mut emb).unwrap();
+        let seen = seen.lock().unwrap().clone();
+        assert_eq!(
+            seen[0].chunks_done, committed,
+            "resume starts at the checkpoint"
+        );
+        let mut last = committed;
+        for p in &seen {
+            assert_invariants(p);
+            assert!(p.chunks_done >= last && p.chunks_done <= p.chunks_total.unwrap());
+            last = p.chunks_done;
+        }
+        let end = session_corpus_progress(root, warm());
+        assert_eq!(end, CorpusProgress::ready(index.len() as u64, 0));
+    }
+
+    #[test]
+    fn wp144_sweep_removes_leftover_temps_on_load() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 3);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
+        let stray = session_index_dir(root).join(".meta.json.tmp-1-1");
+        std::fs::write(&stray, b"half").unwrap();
+        let _ = load_session_index(root).unwrap();
+        assert!(!stray.exists());
+    }
+
+    #[test]
+    fn wp144_legacy_layout_migrates_without_reembedding() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 40);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let before = ensure_session_index(root, &reader, &mut HashEmbedder)
+            .unwrap()
+            .len();
+        SessionSemanticIndex::write_legacy_layout_for_test(root, false);
+        assert!(session_should_rebuild(root), "layout 0 alone would rebuild");
+        let (mut emb, texts) = CountingEmbedder::new();
+        let index = ensure_session_index(root, &reader, &mut emb).unwrap();
+        assert_eq!(index.len(), before);
+        assert_eq!(texts.load(std::sync::atomic::Ordering::SeqCst), 0);
+        assert!(!legacy_vectors_path(root).exists());
+        assert_eq!(read_meta(root).unwrap().unwrap().layout, LAYOUT);
+    }
+
+    fn built(rows: usize) -> (TempDir, SessionDataReader, usize, Box<dyn std::any::Any>) {
+        let dir = TempDir::new().unwrap();
+        let (data, _id, db, lease) = seeded(dir.path(), rows);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let (mut emb, _) = CountingEmbedder::new();
+        let total = ensure_session_index(dir.path(), &reader, &mut emb)
+            .unwrap()
+            .len();
+        (dir, reader, total, Box::new((data, lease)))
+    }
+
+    fn resume_embeds(root: &Path, reader: &SessionDataReader) -> (usize, usize) {
+        let (mut emb, texts) = CountingEmbedder::new();
+        let index = ensure_session_index(root, reader, &mut emb).unwrap();
+        assert!(!read_meta(root).unwrap().unwrap().building);
+        (index.len(), texts.load(std::sync::atomic::Ordering::SeqCst))
+    }
+
+    fn append_raw(path: &Path, bytes: &[u8]) {
+        use std::io::Write;
+        let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
+        f.write_all(bytes).unwrap();
+    }
+
+    #[test]
+    fn wp144_half_record_tail_is_cut_without_reembedding() {
+        reset_crash(None);
+        let (dir, reader, total, _keep) = built(60);
+        let root = dir.path();
+        append_raw(&vectors_path(root), &vec![7u8; VECTOR_RECORD / 2]);
+        append_raw(&chunks_path(root), b"{\"id\":99999,\"key\":\"torn");
+        append_raw(&settled_path(root), b"{\"session_id\":\"x");
+        let (len, embedded) = resume_embeds(root, &reader);
+        assert_eq!((len, embedded), (total, 0));
+        let meta = read_meta(root).unwrap().unwrap();
+        assert_eq!(
+            std::fs::metadata(vectors_path(root)).unwrap().len(),
+            meta.vectors_len
+        );
+        assert_eq!(
+            std::fs::metadata(chunks_path(root)).unwrap().len(),
+            meta.chunks_len
+        );
+    }
+
+    #[test]
+    fn wp144_log_shorter_than_checkpoint_salvages_instead_of_rebuilding() {
+        reset_crash(None);
+        let (dir, reader, total, _keep) = built(60);
+        let root = dir.path();
+        // Lose 3 whole vectors and half of a 4th.
+        let vp = vectors_path(root);
+        let len = std::fs::metadata(&vp).unwrap().len();
+        let cut = len - (3 * VECTOR_RECORD + VECTOR_RECORD / 2) as u64;
+        std::fs::OpenOptions::new()
+            .write(true)
+            .open(&vp)
+            .unwrap()
+            .set_len(cut)
+            .unwrap();
+        let (len, embedded) = resume_embeds(root, &reader);
+        assert_eq!(len, total);
+        assert!(
+            embedded > 0 && embedded < total / 2,
+            "redo only the lost rows: {embedded}/{total}"
+        );
+        assert!(meta_path(root).is_file());
+    }
+
+    #[test]
+    fn wp144_corrupt_line_and_missing_chunks_log_salvage_instead_of_rebuilding() {
+        reset_crash(None);
+        let (dir, reader, total, _keep) = built(60);
+        let root = dir.path();
+        // A corrupt line inside the committed range.
+        let cp = chunks_path(root);
+        let body = std::fs::read_to_string(&cp).unwrap();
+        let first = body.find('\n').unwrap();
+        let mut bad = body.clone().into_bytes();
+        for b in &mut bad[..first] {
+            *b = b'#';
+        }
+        std::fs::write(&cp, &bad).unwrap();
+        let (len, embedded) = resume_embeds(root, &reader);
+        assert_eq!(len, total);
+        assert!(embedded > 0 && embedded < total / 2, "{embedded}/{total}");
+        // A missing chunks log: every row redone, but the index is not rebuilt
+        // from a deleted meta, and vectors stay.
+        let vectors_before = std::fs::metadata(vectors_path(root)).unwrap().len();
+        std::fs::remove_file(&cp).unwrap();
+        let (len, _) = resume_embeds(root, &reader);
+        assert_eq!(len, total);
+        assert!(meta_path(root).is_file());
+        assert!(
+            std::fs::metadata(vectors_path(root)).unwrap().len() > vectors_before,
+            "salvage appends to the kept log; a rebuild would have emptied it"
+        );
+    }
+
+    #[test]
+    fn wp144_crash_mid_migration_reruns_without_reembedding() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 40);
+        let reader = crate::session::SessionDataReader::open(&db);
+        reset_crash(None);
+        let before = ensure_session_index(root, &reader, &mut HashEmbedder)
+            .unwrap()
+            .len();
+        SessionSemanticIndex::write_legacy_layout_for_test(root, false);
+        reset_crash(Some(CrashAt::MidMigration));
+        assert!(SessionSemanticIndex::migrate_legacy_layout(root).is_err());
+        reset_crash(None);
+        assert_eq!(read_meta(root).unwrap().unwrap().layout, 0);
+        assert!(legacy_vectors_path(root).is_file());
+        let (len, embedded) = resume_embeds(root, &reader);
+        assert_eq!((len, embedded), (before, 0));
+        assert_eq!(read_meta(root).unwrap().unwrap().layout, LAYOUT);
+        assert!(!legacy_vectors_path(root).exists());
+    }
+
+    #[test]
+    fn wp144_commit_after_failed_commit_does_not_count_the_stale_tail() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 100);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let mut index = load_session_index(root).unwrap();
+        reset_crash(Some(CrashAt::AfterChunks(2)));
+        let (mut emb, _) = CountingEmbedder::new();
+        assert!(consume_session_index(root, &reader, &mut emb, &mut index).is_err());
+        reset_crash(None);
+        // Same process, same index object: carry on.
+        let (mut emb, _) = CountingEmbedder::new();
+        consume_session_index(root, &reader, &mut emb, &mut index).unwrap();
+        let meta = read_meta(root).unwrap().unwrap();
+        assert!(!meta.building);
+        assert_eq!(
+            std::fs::metadata(vectors_path(root)).unwrap().len(),
+            meta.vectors_len
+        );
+        assert_eq!(
+            std::fs::metadata(chunks_path(root)).unwrap().len(),
+            meta.chunks_len
+        );
+        // Every committed byte parses: a fresh load needs no salvage.
+        let fresh = SessionSemanticIndex::load(root).unwrap();
+        assert!(
+            !read_meta(root).unwrap().unwrap().building,
+            "load did not salvage"
+        );
+        assert_eq!(fresh.len(), index.len());
+        let (chunks, _) = SessionSemanticIndex::live_artifact_lines(root);
+        assert_eq!(chunks.len(), fresh.len());
+    }
+
+    #[test]
+    fn wp144_invalid_utf8_line_keeps_meta_and_redoes_only_its_rows() {
+        reset_crash(None);
+        let (dir, reader, total, _keep) = built(60);
+        let root = dir.path();
+        let cp = chunks_path(root);
+        let mut body = std::fs::read(&cp).unwrap();
+        let first = body.iter().position(|b| *b == b'\n').unwrap();
+        for b in &mut body[1..first.min(9)] {
+            *b = 0xFF;
+        }
+        std::fs::write(&cp, &body).unwrap();
+        let vectors_before = std::fs::metadata(vectors_path(root)).unwrap().len();
+        let index = load_session_index(root).unwrap();
+        assert!(meta_path(root).is_file(), "meta kept");
+        assert!(index.len() < total);
+        let (len, embedded) = resume_embeds(root, &reader);
+        assert_eq!(len, total);
+        assert!(embedded > 0 && embedded < total / 2, "{embedded}/{total}");
+        assert!(std::fs::metadata(vectors_path(root)).unwrap().len() > vectors_before);
+    }
+
+    #[test]
+    fn wp144_io_error_on_load_keeps_meta_and_retries() {
+        reset_crash(None);
+        let (dir, reader, total, _keep) = built(60);
+        let root = dir.path();
+        let meta_before = std::fs::read(meta_path(root)).unwrap();
+        FAIL_LOADS.with(|c| c.set(1));
+        assert!(load_session_index(root).is_err());
+        assert_eq!(
+            std::fs::read(meta_path(root)).unwrap(),
+            meta_before,
+            "meta untouched"
+        );
+        let (len, embedded) = resume_embeds(root, &reader);
+        assert_eq!(
+            (len, embedded),
+            (total, 0),
+            "the retry resumes, no re-embed"
+        );
+    }
+
+    #[test]
+    fn wp144_corrupt_meta_is_the_only_case_that_drops_it() {
+        reset_crash(None);
+        let (dir, reader, total, _keep) = built(30);
+        let root = dir.path();
+        std::fs::write(meta_path(root), b"{not json").unwrap();
+        let (len, embedded) = resume_embeds(root, &reader);
+        assert_eq!((len, embedded), (total, total));
+    }
+
+    #[test]
+    fn wp144_failed_reload_after_failed_commit_forces_reload_on_next_use() {
+        let dir = TempDir::new().unwrap();
+        let root = dir.path();
+        let (_data, _id, db, _lease) = seeded(root, 100);
+        let reader = crate::session::SessionDataReader::open(&db);
+        let mut index = load_session_index(root).unwrap();
+        reset_crash(Some(CrashAt::AfterChunks(2)));
+        FAIL_LOADS.with(|c| c.set(1));
+        let (mut emb, _) = CountingEmbedder::new();
+        assert!(consume_session_index(root, &reader, &mut emb, &mut index).is_err());
+        reset_crash(None);
+        assert_eq!(FAIL_LOADS.with(|c| c.get()), 0, "the reload was attempted");
+        assert!(index.needs_reload(), "memory ahead of disk is flagged");
+        let (mut emb, _) = CountingEmbedder::new();
+        consume_session_index(root, &reader, &mut emb, &mut index).unwrap();
+        assert!(!index.needs_reload());
+        let meta = read_meta(root).unwrap().unwrap();
+        assert!(!meta.building);
+        assert_eq!(
+            std::fs::metadata(vectors_path(root)).unwrap().len(),
+            meta.vectors_len
+        );
+        let fresh = SessionSemanticIndex::load(root).unwrap();
+        assert!(
+            !read_meta(root).unwrap().unwrap().building,
+            "load did not salvage"
+        );
+        assert_eq!(fresh.len(), index.len());
+        let live = reader.searchable_keys_blocking(None).unwrap();
+        let (_, settled) = SessionSemanticIndex::live_artifact_lines(root);
+        assert_eq!(settled.len(), live.len(), "every row settled once");
+    }
+
+    #[test]
+    fn wp144_meta_read_io_error_is_not_a_rebuild_and_keeps_the_logs() {
+        use crate::engines::code_search::{IndexStatus, IndexWork};
+        reset_crash(None);
+        let (dir, reader, total, _keep) = built(40);
+        let root = dir.path();
+        let lens = |root: &Path| {
+            [
+                vectors_path(root),
+                chunks_path(root),
+                settled_path(root),
+                meta_path(root),
+            ]
+            .map(|p| std::fs::metadata(p).unwrap().len())
+        };
+        let before = lens(root);
+        // An existing queue.json must survive byte for byte (R4-1).
+        let queue_before = br#"{"pending_sessions":["kept"],"note":"previous"}"#.to_vec();
+        std::fs::write(session_queue_path(root), &queue_before).unwrap();
+        FAIL_META_READS.with(|c| c.set(true));
+        assert!(!session_should_rebuild(root));
+        assert!(!matches!(
+            session_work_from_disk(root),
+            IndexWork::Rebuild { .. }
+        ));
+        assert!(!matches!(
+            session_index_status(root),
+            IndexStatus::NeedsRebuild | IndexStatus::Absent
+        ));
+        queue_session_dirty(root, &reader);
+        assert_eq!(
+            std::fs::read(session_queue_path(root)).unwrap(),
+            queue_before,
+            "queue.json unchanged"
+        );
+        // The worker's own load refuses to start on it (meta kept, retried).
+        assert!(load_session_index(root).is_err() || meta_path(root).is_file());
+        FAIL_META_READS.with(|c| c.set(false));
+        assert_eq!(lens(root), before, "no begin_build: every log kept");
+        let (len, embedded) = resume_embeds(root, &reader);
+        assert_eq!((len, embedded), (total, 0));
+    }
+
+    #[test]
+    fn wp144_load_failure_reason_is_typed() {
+        reset_crash(None);
+        let (dir, _reader, _total, _keep) = built(20);
+        FAIL_LOADS.with(|c| c.set(1));
+        let error = SessionSemanticIndex::load_tagged(dir.path()).err().unwrap();
+        assert_eq!(error.reason, LoadFailure::Io);
+        assert_eq!(LoadError::ann(error.error).reason.as_str(), "ann");
+    }
+
+    #[test]
+    fn wp144_consume_with_unreadable_meta_errs_without_embedding() {
+        reset_crash(None);
+        let (dir, reader, _total, _keep) = built(40);
+        let root = dir.path();
+        let lens = |root: &Path| {
+            [
+                vectors_path(root),
+                chunks_path(root),
+                settled_path(root),
+                meta_path(root),
+            ]
+            .map(|p| std::fs::metadata(p).unwrap().len())
+        };
+        let before = lens(root);
+        FAIL_META_READS.with(|c| c.set(true));
+        let (mut emb, texts) = CountingEmbedder::new();
+        let mut index = SessionSemanticIndex::new_empty().unwrap();
+        let result = consume_session_index(root, &reader, &mut emb, &mut index);
+        FAIL_META_READS.with(|c| c.set(false));
+        assert!(result.is_err());
+        assert_eq!(
+            texts.load(std::sync::atomic::Ordering::SeqCst),
+            0,
+            "zero embeds"
+        );
+        assert_eq!(lens(root), before, "logs untouched");
+    }
 }
```
