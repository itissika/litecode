# WP-057 / WP-060 IDE · 引擎口径落地产物

- owner：ide · reviewer：backend_mid · WP-060 → deliver
- 日期：2026-10-12（HKT）
- 工作区 HEAD（开工基线）：`843a167`（`843a1672d671b0239b96cfaca80c7ae7ef90b70d`）
- **未 commit / 未 push**
- 禁区确认：未改 `desired` 默认 `false`；未碰 symlink 产品行为；未改 `client_protocol` / 前端

## 1. 口径对照（改前 → 改后）

| 口径 | 改前 | 改后 |
|---|---|---|
| Warming = up but not searchable | `start`→Warming→Warm；Rebuild 时状态仍 Warm | Rebuild（`RefreshAcceptedMode::Rebuild`）期间置 **Warming**，完成→Warm / 失败→Failed（`mod.rs` spawn_scoped_refresh） |
| 可搜信号 | 仅 `is_warmed`；Tools 靠自等 | 新增 **`is_searchable(id)`**：`Warm` ∧ `code_search_call_gate==Ready` |
| 非暖可搜 | `code_search_call_gate`：Stale/Absent/NeedsRebuild → Wait；`retrieval_job_gate`：refresh_busy/Refreshing → Wait | **Warm+Stale/Refreshing/refresh_busy → Ready**；Building/Absent/NeedsRebuild → Wait；Warming → Wait |
| 废 DirtyTooLarge | `decide_index_work` embed≥50% → Rebuild | 枚举删除变体；脏量一律 **Update**；`merge_index_work` rank 仅 FirstDesired/Incompatible |
| 静默刷新 | 刷新挡搜 | 增量 Refreshing 不挡搜；Rebuild 回 Warming 桶 |
| 宁缺毋假 | `SEMANTIC_MIN_SCORE` + `gate_semantic_hits`；`hit_from_chunk` 跳过 | 保留门槛；注释标明 WP-057 不降阈值 |
| sync% | Tools 临时算 files% 文案 | **`sync_pct_from_view` / `SyncPct`**；`engines/detail` index 附 `sync_pct`/`sync_label` |

## 2. 改动文件摘要

| 文件 | 变更 |
|---|---|
| `src/engines/code_search/index_status.rs` | 删 `DirtyTooLarge`；`decide_index_work`；新增 `SyncPct` + `sync_pct_from_view`；单测 |
| `src/engines/code_search/mod.rs` | 导出 `SyncPct` / `sync_pct_from_view` |
| `src/engines/mod.rs` | `is_searchable`；gate 语义；Rebuild→Warming；相关测 |
| `src/engines/status_view.rs` | detail JSON 暴露 `sync_pct` / `sync_label` |
| `src/engines/session_search/mod.rs` | `gate_semantic_hits` 注释（宁缺毋假） |
| `src/tools/code_search.rs` | **仅**测试：`warm_stale_*` 对齐新 gate（产品暖等仍属 WP-061） |

说明：工作区另有 Tools WP-061 并行改动 `src/tools/mod.rs`（消费 `SyncPct` 的 `format_sync_pct_line` 等）——非本包写入，保留并行。

## 3. sync% API（供 WP-061 / WP-059）

```text
pub struct SyncPct { sync_pct: u8, sync_label: Option<&'static str> }
pub fn sync_pct_from_view(view: &ResolvedIndexView, pending_updates: usize) -> SyncPct
```

| status | sync_pct | label |
|---|---|---|
| Ready ∧ pending=0 | 100 | ready |
| Stale / Ready+pending | `100 - min(99, pending*2)` floor 1 | syncing |
| Building/Refreshing + progress | `files_done*100/files_total`（total=0→0；封顶 99） | building / syncing |
| Absent / NeedsRebuild | 0 | building |
| Failed | 0 | None |

契约对齐：`.litecode/ops/artifacts/WP-057-sync-pct-contract.md`。Warming 工具不可见时不发假 100%（Building 中 pct&lt;100）。

## 4. ↔ Tools WP-061 接口

**IDE：`EngineState::Warming`⇔不可搜（`is_searchable=false`）；`Warm`⇔可搜（含静默刷新）；提供 `sync_pct_from_view` / detail 字段。Tools 消费信号做可见性、废 `CODE_SEARCH_WARM_WAIT`、结果首行 sync%、session 降级——不改引擎状态机/重建策略。**

## 5. 测试结果

命令（`PATH=/home/box/.cargo/bin:$PATH`，`HOME=/workspace/litecode/.tmp/home`，`timeout 600`）：

1. 聚焦 13 测：`decide_index_work_*` / `sync_pct_*` / gate / `warming_is_not_searchable` / `warm_stale_is_searchable_*` → **13 passed**
2. `cargo test --lib -- engines::` → **272 passed；0 failed；1 ignored**

## 6. ISSUE 引擎腿

| Issue | 本包 | 余量 |
|---|---|---|
| 023 | 可搜/Warming 信号就绪；**未**改 desired 默认 | Tools 隐藏未就绪工具 |
| 024 | gate 不再因 Stale/Refreshing Wait | Tools 删暖机长等 |
| 025 | — | Tools 文案 |
| 026 | 非暖可搜 + 语义门槛宁缺毋假 | Tools 降级表面 |

## 7. 门禁清单

- [x] 不 commit
- [x] 不改 desired 默认 false（`src/config/workspace.rs` 仍 `desired: false`）
- [x] 禁 symlink
- [x] 不改 client_protocol / 前端
- [x] 尽量不改 tools 产品路径（仅测例对齐；WP-061 并行消费 SyncPct）
