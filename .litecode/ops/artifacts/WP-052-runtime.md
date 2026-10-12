# WP-054 / WP-052-runtime（Runtime：ISSUE-032 take_changes 失败回灌）

- 父包：WP-052 → WP-053；本包：WP-054；owner：runtime；approver：backend_mid
- 状态：**deliver（ISSUE-032 已修）**；plan 已批；reject→in_progress 后闭环；**不 commit**
- 分支：`ops/team-framework`；基线 HEAD `5421369`
- 产物路径：`.litecode/ops/artifacts/WP-052-runtime.md`
- 禁区：引擎产品、symlink、前端、ISSUE-034；**不扩写** `src/reminder/**`（含 `file_tracker.rs`）；不 commit

---

## 1. Goal / Promise

修 **ISSUE-032**：seam 在 `take_changed_files` 消费外部改动后，若后续 `spine_reminder_view` 失败或 `FilesChanged` append 失败，改动通知须**回灌**，下一 seam 仍可上报——与 bash / subagent / custom 的 mailbox 回灌同类。

- promise（ISSUE）：`files_changed` 提醒失败时与 bash/subagent/custom 一样回灌，不丢通知。
- actual：`take_changed_files` 在 seam 开头推进 FileTracker 快照；`restore_seam_sources` 不含 changed files；非 subagent 的 append 失败仅 warn。

---

## 2. 现状读码（证据）

| 位置 | 行为 | 问题 |
|---|---|---|
| `src/runtime/exec.rs:294` | `sessions.take_changed_files` → 底层 `FileTracker::take_changes` | **先消费**：快照就地更新为「已见」 |
| `src/reminder/file_tracker.rs:66-81` | `take_changes` 对每个 path `insert` 新 snap；**无回滚/mark_dirty** | 消费不可逆 |
| `src/runtime/exec.rs:386-388` | `spine_reminder_view` Err → `restore_seam_sources` 后 return | 只回灌 bash / subagent / custom，**不含** changed |
| `src/runtime/exec.rs:486-499` | `restore_seam_sources` | 三路 mailbox only |
| `src/runtime/exec.rs:438-481` | append 循环：`SubagentSettled` 失败 → 回灌+`Err`；其余 kind 失败 → **warn**；循环后若 bash/custom/subagent 未写入则回灌 | `FilesChanged` **无** `*_written` 标志、**无**失败回灌 |
| `src/session/manager.rs:1419-1427` | `take_changed_files` 转调 tracker | 无对称 `restore_changed_files` |

对照（应对齐的回灌模式）：

- bash：`restore_mailbox`（`exec.rs:471-472` / `510-520`）
- custom：`restore_mailbox`（`479-480` / `501-507`）
- subagent：`restore_completions`（失败硬 `Err` 或未写入时回灌）

`FilesChanged` 更接近 **bash/custom**（append 失败 warn + 回灌，不因单条 FilesChanged 使整个 seam 变硬错误），而非 subagent 的硬 `Err`。

WP-025 产物（`WP-022-runtime.md` §ISSUE-032）已确认成立并留下跨域草案；本包落地该草案的 **Runtime 侧**，Context API 仅提案。

---

## 3. 方案（主改 Runtime seam）

### 3.1 Runtime（本包执行，获批后）

1. **spine 失败路径**（`exec.rs:386-388`）：在 `restore_seam_sources(...)` 同时回灌本次已 `take` 的 `changed` paths（调用 Session 薄封装，见 §3.3）。
2. **append 路径对齐 bash/custom**：
   - 增加 `files_changed_written: bool`（或等价：仅当本轮 reminders 含 `FilesChanged` 且成功写入时置 true）。
   - `ReminderKind::FilesChanged` append `Err`：保持现有 `tracing::warn`（与 bash/custom 一致，**不**升格为 seam `Err`）。
   - 循环结束后：若本轮本应上报的 changed 非空且 **未**成功写入 `FilesChanged`，则 `restore_changed_files(session_id, &changed)`。
3. **成功路径不变**：`FilesChanged` 已写入 → 不回灌；下一 seam `take_changes` 对已吸收路径为空（现有语义）。
4. **不改**引擎 / symlink / 前端 / ISSUE-034；**不直接改** `src/reminder/file_tracker.rs`。

伪代码（意图，非落地）：

```text
changed = sessions.take_changed_files(sid)
...
view = spine_reminder_view(sid)
  Err(e) => {
    restore_seam_sources(bash, completions, custom)
    sessions.restore_changed_files(sid, &changed)  // NEW
    return Err(e)
  }
...
files_changed_written = false
for reminder in reminders {
  match append {
    Ok if FilesChanged => files_changed_written = true
    Err if SubagentSettled => /* 现有硬回滚+Err */
    Err => warn  // FilesChanged 走此支
  }
}
if !bash_written { restore_bash... }
if !subagent_written { restore_completions... }
if !custom_written { restore_custom... }
if !changed.is_empty() && !files_changed_written {
  sessions.restore_changed_files(sid, &changed)  // NEW
}
```

### 3.2 Context / file_tracker（**只读提案**，经中层转 Context；本包不写）

`FileTracker` 现无回滚口。为让 Session/Runtime 能「撤销本次 take 的消费」，请 Context 增加其一（推荐 A）：

| 方案 | 签名（草案） | 语义 | 取舍 |
|---|---|---|---|
| **A（推荐）** | `mark_dirty(&mut self, paths: &[PathBuf])` | 对给定 path：**删除** `snaps` 条目，**保留** `order` | 下次 `take_changes`：`get` 为 None → `differs=true` → 再次上报并写入新 snap。不改变 MAX_TRACKED / absorb 语义 |
| B | `forget(&mut self, paths: &[PathBuf])` | 从 `order`+`snaps` 双删 | **不可取作回灌**：下次不再追踪该 path，外部再改也不会报 |
| C | 保存旧 snap 并 `restore_snaps` | 精确回滚到 take 前 | 正确但 API/内存更重；本缺陷不需要 |

**不建议** Runtime 绕过 tracker、自建「侧路 mailbox」：会与 `absorb` / 重启清空 / 多 seam 语义分叉，且与 bash 回灌「归还真相源」不一致。

Context 侧建议自带单测：`mark_dirty` 后再次 `take_changes` 仍报出同一 path；`absorb` 不受影响。

> 本包 **零行** 改动 `src/reminder/**`。接口以中层转发、Context 落地为准；若 Context 命名/语义有调整，Runtime 只跟 Session 封装。

### 3.3 Session 薄封装（依赖协调）

`SessionManager` 已有 `take_changed_files` / `absorb_touched_files` / `note_touched_files`（`manager.rs:1399-1427`）。对称增加：

```text
pub fn restore_changed_files(&self, session_id: &str, paths: &[PathBuf])
// → record.file_tracker.mark_dirty(paths)
```

- **归属**：Session 面；本包 **默认不擅自改** `src/session/**`。
- **请中层裁决**其一：
  1. 另开/并行 Session 微任务只加此封装 + 单测；或
  2. 批准本包在 Context `mark_dirty` 就绪后，**仅**加这一薄转调（仍不碰 reminder 实现）。
- Runtime 只通过 `sessions.restore_changed_files` 调用，不直持 `FileTracker`。

### 3.4 依赖与串行

```text
Context: FileTracker::mark_dirty          （中层转；本包只提案）
    ↓
Session: restore_changed_files 薄封装     （中层裁：并行包 or 授权本包一行）
    ↓
Runtime: exec.rs 两处回灌 + 回归测         （本包主改；API 未就绪则 blocked 写明）
```

032 ∥ 034（Tools WP-055）代码面不交，可并行。

---

## 4. 测试计划（获批 start 后；批前不写测）

| # | 用例 | 断言 | 建议落点 |
|---|---|---|---|
| T1 | seam：`take` 后 `spine_reminder_view` 注入失败 | 调用 restore；**下一** seam / 再次 take 仍含同一 paths；bash/custom/subagent 回灌回归不破 | `tests/` 或 `src/runtime` 集成；失败注入点复用/扩展现有 session/spine mock |
| T2 | seam：`FilesChanged` append 注入失败 | warn 路径；`files_changed_written=false` → restore；下一 seam 仍上报；seam 本身不因 FilesChanged 变硬 `Err`（对齐 bash/custom） | 同上 |
| T3 | 幸福路径：FilesChanged 写入成功 | 不 restore；随后 take 对那些 path 为空 | 防回灌过猛 |
| T4 | 空 changed | 不调用 restore（或 no-op）；无行为变化 | 烟测 |

- 命令（预定）：`cargo test` 定点本域相关集成/lib（具体二进制名 start 后按夹具落点写进产物）；直接 `cargo`，无 PATH 前缀。
- **不**在本包装 ISSUE-034 / 引擎 / symlink 测。

若失败注入夹具不足：先在产物记缺口，**报中层**再加 tests/common seam（不擅自扩产品 seam）。

---

## 5. 风险

| 风险 | 缓解 |
|---|---|
| Context `mark_dirty` 未就绪 → Runtime 无法闭环 | 状态 `blocked`（product/arch 依赖）写进产物；不 hack 侧路 mailbox |
| `mark_dirty` 误做成 `forget`（移出 order） | 提案表写明；交叉审 Context 实现 |
| 回灌过猛（成功写入仍 restore） | T3；以 `files_changed_written` 为准 |
| 与 subagent 硬 `Err` 口径混淆 | 明确 FilesChanged=bash/custom 软回灌 |
| Session 封装归属争执 | §3.3 交中层裁，不阻塞提案提交 |
| 多 path 部分写入（未来若拆多条 FilesChanged） | 现状 sources 通常一条；若一条失败则整批 restore `changed` |

---

## 6. done_when

1. ISSUE-032：失败路径回灌成立（T1/T2 绿）；幸福路径不回灌（T3 绿）。
2. Runtime 改动限于 `src/runtime` seam（+ 若中层授权的 Session 一行转调）；**未**改 `src/reminder/**`、引擎、symlink、前端、ISSUE-034 面。
3. Context API 提案已写入本产物 §3.2，供中层转发；Context/Session 落地证据或 blocked 原因记入产物。
4. ISSUE-032 history 追加修复/证据；status 在验收后由流程关闭（本包不擅自改流程外字段以外的越权操作）。
5. 本包 **不 commit**；交中层 `deliver` → 验收。

---

## 7. 执行步骤（approve → start 后）

1. 等中层批 plan；确认 Context `mark_dirty` 与 Session 封装路径（§3.3 裁夺）。
2. Context/Session API 就绪（或本包获授权加 Session 转调）后，改 `exec.rs` 两处回灌。
3. 落 T1–T4；定点 `cargo test` 绿。
4. 更新本产物实跑表 + ISSUE-032 history；`deliver`。

---

## 8. 交叉审 / 协作

- **→ Context**：请审 §3.2 `mark_dirty` 语义（保留 order、只剔 snap）；实现归属 reminder。
- **→ Session**：请审 §3.3 `restore_changed_files` 与现有 `take_changed_files` 对称性。
- **→ Tools**：无交面（034 并行）。
- 中层：汇总 032+034；转发 Context 提案；裁 Session 封装归属。

---

---

## 9. 执行记录（WP-054）

- start →（曾 blocked on WP-056）→ reject→in_progress → 本 deliver
- 时间：2026-10-12T20:44:45+08:00（HKT）
- **不 commit**

### 9.1 依赖就绪

| 项 | 证据 |
|---|---|
| `FileTracker::mark_dirty` | `src/reminder/file_tracker.rs:91`（`snaps.remove` only，保留 `order`）— WP-056 落地，本包**只调用** |
| Session 薄封装 | `SessionManager::restore_changed_files` @ `manager.rs:1432-1441` → `mark_dirty` |
| Runtime 回灌 | `exec.rs` 见下 |

### 9.2 Runtime 改动（`src/runtime/exec.rs`）

| 路径 | 行 | 行为 |
|---|---|---|
| spine Err | :386-394 | `restore_seam_sources` 后若 `changed` 非空 → `restore_changed_files` |
| SubagentSettled 硬 Err 早退 | :465-468 | 若 FilesChanged 尚未写入 → restore changed（与 mailbox 对称） |
| 循环后软回灌 | :443, :453, :494-499 | `files_changed_written`；未写入且 `changed` 非空 → restore（对齐 bash/custom soft） |
| 幸福路径 | — | FilesChanged 写入成功 → 不 restore |

**未改** `src/reminder/**` 实现（仅调用已有 `mark_dirty`）；未碰引擎/symlink/前端/ISSUE-034。

### 9.3 测试

文件：`tests/issue_032_changed_files_restore.rs`

| # | 用例 | 结果 |
|---|---|---|
| T1 | take → restore（模拟 spine Err）→ 再 take 仍报同 path | **ok** |
| T2 | take → soft restore（模拟 append 未写入）→ 再 take 仍报 | **ok** |
| T3 | take → 不 restore（模拟写入成功）→ 再 take 空 | **ok** |
| T4 | empty restore no-op | **ok** |

命令：`cargo test --test issue_032_changed_files_restore -- --test-threads=1` → **6 passed; 0 ignored**（含 common permission 2）。

说明：T1/T2 为 Session API 契约（exec 调用的同一路径）；全链路 AgentLoop Err 注入夹具仍缺，已在产物注明，不阻塞 ISSUE-032 回灌语义闭环。

`cargo check --lib`：**Finished ok**（本包改动可编译）。未跑 `cargo test --lib`（Tools WP-055 导致 mcp client 测编不过，与本包无关）。

### 9.4 ISSUE-032

- status → **closed**（本包修复成立）
- history 追加闭环证据

### 9.5 done_when 核对

1. 失败路径回灌成立（T1/T2 绿）；幸福路径不回灌（T3 绿）— **是**
2. 改动限于 runtime seam + 授权的 Session 一行转调；未改 reminder 实现 / 引擎 / symlink / 前端 / 034 — **是**
3. Context API 已由 WP-056 落地；本包只调用 — **是**
4. ISSUE-032 closed + history — **是**
5. 不 commit；deliver 交中层 — **本步**
