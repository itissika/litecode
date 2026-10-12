# WP-052 / WP-056 · Context：FileTracker::mark_dirty（ISSUE-032）

**WP**：WP-056（parent WP-053 ← WP-052）  
**Owner**：context **Approver**：backend_mid  
**对照**：`.litecode/ops/artifacts/WP-052-runtime.md` §3.2 方案 A  
**禁区遵守**：未改 Runtime seam / Session 封装 / 引擎 / symlink / 前端 / ISSUE-034。未 commit。

---

## 1. API（已落地）

文件：`src/reminder/file_tracker.rs`

```rust
/// Drop snapshots for `paths` but keep them in `order`, so the next
/// [`take_changes`] re-reports them (snap missing → differs).
///
/// This is **not** forget: paths stay tracked. Use it to undo a prior
/// `take_changes` consumption when the caller failed to persist the
/// notification (ISSUE-032 / seam FilesChanged restore).
///
/// Unknown / untracked paths are no-ops (no insert into `order`).
pub fn mark_dirty(&mut self, paths: &[PathBuf])
```

| 项 | 值 |
|---|---|
| 签名 | `pub fn mark_dirty(&mut self, paths: &[PathBuf])` |
| 实现行 | `file_tracker.rs:91–96`（doc `:83–90`） |
| 行为 | 对每个 path：`self.snaps.remove(path)`；**不**碰 `self.order` |
| 导出 | 已有 `pub use file_tracker::FileTracker`（`reminder/mod.rs:17`）→ Runtime/Session 可经 `FileTracker` 调用 |

### 与 forget 的差异

| | `mark_dirty`（本包） | forget（**未实现**；§3.2 方案 B 明确不可取） |
|---|---|---|
| `snaps` | 删除条目 | 删除 |
| `order` | **保留** | 移除 |
| 下次 `take_changes` | `get` 为 None → 再上报 | path 不再追踪，外部改动也不报 |

### 与现有方法关系

| 方法 | 行 | 关系 |
|---|---|---|
| `order` / `snaps` | 23–27 | mark_dirty 只动 snaps |
| `absorb` | 56–62 | 不变；mark_dirty 后 absorb 仍可重写 snap、隐藏自写 |
| `take_changes` | 66–81 | `is_none_or` 使缺 snap → differs；消费后仍 `insert` 新 snap |

---

## 2. 单测（同文件 `#[cfg(test)]`）

| 用例 | 行 | 断言 |
|---|---|---|
| `mark_dirty_then_take_changes_rereports` | 157–171 | take 消化 → mark_dirty → 内容未变再 take 仍报同 path |
| `mark_dirty_preserves_order_and_absorb` | 174–188 | mark_dirty 后 absorb 仍隐藏自写；外部改动仍报（order 保留） |
| `mark_dirty_unknown_path_is_noop` | 191–208 | 未 record 的 path：不入追踪；known 仍正常 |

既有 `absorb_hides_…` / `deletion_is_a_change` 保留。

---

## 3. 测跑证据

命令（按 mid 指定过滤）：

```text
cargo test --lib reminder::file_tracker -- --nocapture
```

**结果（2026-10-12 HKT）**：`litecode` **lib test 未能编译**，过滤器未执行到本域用例。

阻断（**域外**，Tools / ISSUE-034 相关，本包禁区未改）：

```text
error[E0502]/error[E0505]  src/mcp/client.rs:506–538
  r3_cancel_marks_dirty_next_client_ok：call_tool future 借用 client 期间
  使用 client.is_dirty() / drop(client)
```

`cargo build --lib`（无 cfg(test)）此前可通过；`mark_dirty` 已在源码树中。  
**本包不修 mcp**；lib 测绿依赖 Tools 修编译后复跑同一过滤命令。

源码状态：`src/reminder/file_tracker.rs` 含 `mark_dirty` + 3 新测（见上表行号）。

---

## 4. 给 Runtime / Session

- **Context API 已就绪**：`FileTracker::mark_dirty(paths)`。
- Runtime 可按 WP-052-runtime §3.2–3.3：经 Session 薄封装 `restore_changed_files` → `mark_dirty`（**不在本包**；由中层授权 Runtime/Session 另落）。
- 勿直持 tracker 旁路 mailbox；回灌走本 API 即可与 absorb / 多 seam 语义一致。

---

## 5. 验收对照

| done_when | 状态 |
|---|---|
| mark_dirty 语义：删 snaps、留 order | ✅ 源码 |
| 单测就位（再 take 同 path；absorb；unknown noop） | ✅ 源码；⏳ cargo --lib 被 mcp 编译阻断 |
| 产物齐 | ✅ 本文件 |
| 未 commit；未碰禁区 | ✅ |
