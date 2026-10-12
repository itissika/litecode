# WP-069 方案（Session：ISSUE-041 death_list 假红 + ISSUE-036 turn_error 类型化消费）

- owner: session　approver: backend_mid　parent: WP-064　本包: WP-069
- 产物：`.litecode/ops/artifacts/WP-063-session.md`（本文件）
- 状态：**两腿齐 → deliver**（2026-10-12 22:05 HKT）· 腿 A ISSUE-041 已落地 · 腿 B ISSUE-036 已落地（WP-068 close→done 后 consume）· **未 commit**
- 禁区：ISSUE-004、引擎产品行为、symlink 复制；**不 commit / 不 push**
- 工具需求：**无**（wp.mjs / cargo / rg 足够；无额外工具）

## 0. Goal（对齐父包 WP-064 / WP-063-backend §5）

两腿：

1. **ISSUE-041**：收紧 `tests/session_data_sql_death_list.rs`，消除对 `engines/session_search` 自有 SQLite / 文件级 `#[cfg(test)]` 的假红；**不改引擎产品代码与行为**。批后可立即落地，不依赖 Runtime。
2. **ISSUE-036 consume**：`src/client_protocol/project.rs` 的 `turn_error` 改用 Runtime 类型化错误类别判定 `ErrorCode::CompactionFailed`，废止 `final_text == LitecodeError::CompactionFailed.to_string()`。**硬依赖：等 WP-068 Runtime deliver 后再改 `project.rs`**。

## 1. 现状证据（只读核实）

### 1.1 ISSUE-041 · death_list 假红

| 点 | 位置 | 说明 |
|---|---|---|
| 门禁测 | `tests/session_data_sql_death_list.rs:87-148` | `sessions_db_sql_and_rusqlite_stay_in_sqlite_module`：walk `src/**/*.rs`，对「生产行」扫 `rusqlite` / `CREATE TABLE … transcript_items` / leftover 符号 / `sessions.db` join 等 |
| 白名单过窄 | `:27-30` `is_sqlite_ok` | 仅放行 `/src/session/data/sqlite/`（另有 global_db / error.rs / workspace preview） |
| 内联 cfg(test) | `:54-84` `production_lines` | 只跳过**同文件内** `#[cfg(test)] mod { … }`；**不跳过**经父 mod `#[cfg(test)] mod X;` 挂进来的**整文件** |
| 生产自有索引 | `src/engines/session_search/sparse.rs:98` 等 | 生产 `use rusqlite` 打开 **sparse 自有索引**（`sparse_index_path`），≠ SessionData `sessions.db` 写路径；被误杀 |
| 文件级测试模块 | `session_search/mod.rs:8-20` | `#[cfg(test)] mod dense_parity/golden/parity/relevance`；`golden.rs`/`parity.rs` 等整文件 `use rusqlite`，walk 仍当生产扫 |
| 同模式扩大面 | `src/session/data/mod.rs:5-6` + `final_immutability_tests.rs:18` | 文件级 `#[cfg(test)] mod final_immutability_tests`；亦 `use rusqlite`，WP-047 已登记为假红扩大面（非产品回归） |

门禁意图不变：**sessions.db 的 SQL/rusqlite 只许在 SessionData sqlite（+ 既有白名单）**。假红来自「任意 rusqlite 子串」扫到引擎自有索引与文件级测试模块。

### 1.2 ISSUE-036 · turn_error 文本匹配

| 点 | 位置 | 说明 |
|---|---|---|
| 消费端 | `project.rs:537-565` `turn_error` | `TurnEndReason::Error` 时用 `final_text == LitecodeError::CompactionFailed.to_string()` → `ErrorCode::CompactionFailed` / `retryable:false`；否则按 reconnect 映 `llm_http`/`internal` |
| 发射端现状 | `InternalEvent::TurnCompleted`（`observer.rs:129-135`） | 仅有 `final_text` / `reason` / token stats / seq；**无类型化错误类别**；Runtime 只把 `err.to_string()` 塞进 `final_text`（ISSUE-036 actual） |
| 既有测 | `project.rs` `wp025_turn_error_tests`（~1207+） | 用文案构造 `final_text` 断言 `CompactionFailed`；消费改类型化后须改测 |
| 发射责任 | **WP-068 Runtime** | 在内存 `TurnCompleted`（或等价）落地类型化类别；**不改** `project.rs`（归本包） |

字段名 / 枚举形态 **TBD**：以 WP-068 deliver 产物与 diff 为准；本方案只约定消费契约与改法，不预发明 wire。

---

## 2. 腿 A · ISSUE-041（可与 WP-068 并行；批后即可改测试）

### 2.1 改法（仅 `tests/session_data_sql_death_list.rs`；零引擎 src 改动）

1. **路径白名单（引擎自有 SQLite）**  
   - 新增 `is_session_search_engine_ok(path)`：`norm(path).contains("/src/engines/session_search/")`  
   - 在 hit 扫描循环开头与现有 `is_sqlite_ok` 等并列 `continue`  
   - 理由：sparse/lexical/trimmer 等生产 rusqlite 服务的是引擎自有 FTS/稀疏索引，不是 SessionData `sessions.db` 架构越界；门禁目标是 sessions.db 方言集中，不是禁止引擎任何 SQLite。

2. **跳过文件级 `#[cfg(test)]` 模块**（收窄检测，非整目录 ignore）  
   - 预扫：walk 时解析各 `mod.rs`（及含 `mod name;` 的父文件）中形如  
     `#[cfg(test)]` 下一行（或紧邻）`mod <name>;`  
     收集「仅测试挂载」的 sibling 文件路径（如 `…/golden.rs`、`…/parity.rs`、`…/dense_parity.rs`、`…/relevance.rs`、`…/final_immutability_tests.rs`）。  
   - 对这些路径整文件跳过（与内联 `production_lines` 语义对齐：测试代码不参与生产方言门禁）。  
   - **不**因目录名含 test 而盲跳；**不**跳过生产 `sparse.rs` 以外且已由 (1) 白名单覆盖的引擎路径以外的真实生产违规。

3. **保持**  
   - `leftover_symbols_absent_from_repo`（`:151-188`）逻辑不动（除非复跑证明新假红；本方案不预期改）。  
   - 既有白名单：`session/data/sqlite`、`config/global_db`、`types/error.rs`、`workspace/sqlite_preview.rs`、path-construct 例外。  
   - 对**真正**在非 sqlite 模块写 `sessions.db` / leftover Session CRUD 的命中仍 fail。

### 2.2 验收（041）

- `PATH=/home/box/.cargo/bin:$PATH cargo test --test session_data_sql_death_list`：**全绿、0 假红**。  
- 抽查：故意在非白名单生产文件插入 `use rusqlite::` 仍应失败（门禁未废）。  
- `rg` / diff：**无** `src/engines/**` 产品行为改动；仅测试文件。  
- 产物补「041 落地证据」：命令、pass 数、白名单/跳过路径列表。

### 2.3 不做

- 不改 `src/engines/session_search/**` 产品代码。  
- 不 `#[ignore]` 整测、不删 death_list。  
- 不扩大白名单到无关 crate 路径（若复跑另发现假红，写入产物报 backend_mid，不擅扩）。

---

## 3. 腿 B · ISSUE-036 consume（**硬串行：等 WP-068 deliver**）

### 3.1 依赖门闩

| 条件 | 动作 |
|---|---|
| WP-068 **未** deliver | **禁止**改 `project.rs` / 相关测；041 可先行 |
| WP-068 已 deliver | 读 `.litecode/ops/artifacts/WP-063-runtime.md` + Runtime diff，锁定类型化字段 API 后再改消费端 |

假定接口（**待 Runtime 落地后以实际为准，本方案不锁死命名**）：

- 内存事件 `InternalEvent::TurnCompleted`（或 Runtime 文档指明的等价载体）新增可选类别，例如 `error_kind` / `fail_kind` / `turn_error_category`，可区分 **CompactionFailed**（及必要时与 `CompactionFailKind` 对齐）。  
- **不扩 JSON-RPC wire schema**（父包禁区）；`message` 仍可用 `final_text` / `error.to_string()`。  
- 数字 JSON-RPC code 行为与现网一致；客户端继续读 `StructuredError.code`（`compaction_failed`）+ `retryable:false`。

### 3.2 消费改法（WP-068 后）

文件：`src/client_protocol/project.rs`（主）；必要时同文件内测 / `tests/client_protocol_*` 跟随。

1. 投影 `TurnCompleted` 时把 Runtime 新字段传入 `turn_error`（签名增参或从 event 解构）。  
2. `TurnEndReason::Error` 分支：  
   - **若**类别 == CompactionFailed（精确匹配 Runtime 枚举/常量）→ `ErrorCode::CompactionFailed`，`retryable: false`，`message` 仍取 `final_text`（或 Display），**不再**与 `LitecodeError::CompactionFailed.to_string()` 做字符串相等。  
   - **否则**保持现逻辑：`retryable = reconnect.is_failed()` → `llm_http` / `internal`。  
3. **删除**（首选）或降级为「仅 debug_assert / 测中证明不可达」的文案匹配分支；产物写明选用哪种及理由。  
4. 更新 `wp025_turn_error_tests`：用类型化类别构造事件/参数，断言 `CompactionFailed`；另加「文案碰巧相同但类别非 compaction → 不得误判为 compaction_failed」对照（若 API 允许）。  
5. 与 manual compact 路径已有的 `compact_fail_code(CompactionFailKind)`（`project.rs:146-150`）口径对齐：auto 失败与 manual Failed 均映 `compaction_failed` / non-retryable。

### 3.3 验收（036）

- 类型化路径单测绿；文案匹配代码路径移除或有测证明不再依赖。  
- 相关：`cargo test -p … client_protocol::project::wp025_turn_error_tests`（及本域 compact/turn 投影相关测）绿。  
- 交叉审：与 WP-068 产物字段表一致（名、Option、serde、是否落盘=否）。  
- 若 WP-068 接口与假定严重偏离：停手写清差异，报 backend_mid，**不臆造第二套类别**。

### 3.4 不做

- 不改 Runtime emit、不改 `TurnEndReason` 持久化 schema（除非 WP-068 已改且要求跟随——以 Runtime 交付说明为准，默认不扩盘）。  
- 不改前端；若 `data.code` 语义不变则无需前端中层动作（有变则写入产物请 mid 转发）。

---

## 4. 时序与交付

```
approve WP-069
  ├─ 立即：腿 A ISSUE-041（改 tests/session_data_sql_death_list.rs → 复跑绿 → 写证据）
  └─ 等待：WP-068 deliver
        └─ 腿 B ISSUE-036（改 project.rs + 测 → 复跑绿 → 写字段对齐表）
一并 deliver WP-069 → backend_mid 验收
```

- 若 068 长期未交付：041 可先在产物标明「036 未开工原因=依赖 WP-068」；是否部分 deliver 听 mid（默认两腿齐再 deliver）。  
- 交叉审：本包 deliver 前在 `WP-063-runtime.md` / 本 md 互留「交叉审」节（字段 ↔ `turn_error`）。

## 5. 禁区与工具

- **ISSUE-004** 不碰；引擎产品 / symlink 不碰；不 commit/push。  
- 共享 `target/` 遇锁排队；只跑本域相关测。  
- 工具需求：无。

## 6. 请 backend_mid

请 `approve --id WP-069`。批后 Session start：先落地 041；036 待 WP-068 deliver 后再改 `project.rs`。

---

## 7. 执行记录（WP-069 start 后）

### 7.1 Start

- `wp.mjs start --as session --id WP-069` → ok，状态 `in_progress`（约 2026-10-12 21:53 HKT）。
- 工具链：`PATH=/home/box/.cargo/bin:$PATH`，`rustc 1.99.0`。

### 7.2 腿 A · ISSUE-041 落地证据

**改动文件（仅测试）：** `tests/session_data_sql_death_list.rs`（+62 行）

| 改法 | 位置 | 说明 |
|---|---|---|
| `is_session_search_engine_ok` | `:54-57` | 白名单 `/src/engines/session_search/`（sparse/FTS 自有索引 ≠ sessions.db） |
| `file_level_cfg_test_modules` | `:59-110` | 预扫 `#[cfg(test)]` + `mod name;` → 跳过 sibling `name.rs` / `name/mod.rs` |
| 扫描循环 | `:150-161` | 并列 `continue`：既有白名单 + session_search + cfg_test 文件 |

**预扫跳过的文件级测试模块（本仓实测）：**

- `src/engines/session_search/{dense_parity,golden,parity,relevance}.rs`
- `src/session/data/final_immutability_tests.rs`
- 以及同模式：`knowledge/spec_tests.rs`、`provider_catalog/tests.rs`、`reminder/sources/tests.rs`、`tools/bash_jobs_contract.rs`、`tools/subagent/spawn_contract.rs`、`types/death_list_gate.rs`、`types/golden_item_fixtures.rs`

**引擎产品：** `git diff --stat src/engines/` → **空**（零引擎 src 改动）。

**测试命令与结果：**

```
PATH=/home/box/.cargo/bin:$PATH cargo test --test session_data_sql_death_list
```

| 用例 | 结果 |
|---|---|
| `leftover_symbols_absent_from_repo` | **ok** |
| `sessions_db_sql_and_rusqlite_stay_in_sqlite_module` | **FAIL · 1 真实命中**（见下） |

**假红清零：** 收紧前失败列表含 session_search 生产 `rusqlite`（如 `sparse.rs:98`）与文件级测（`golden`/`parity`/`final_immutability_tests` 等）。收紧后这些 **全部消失**。

**剩余真实命中（不扩白名单，按方案报 mid）：**

| 文件:行 | 规则 | 判定 |
|---|---|---|
| `src/tools/litecode_workspace.rs:821` | `SessionDataReader::open(` | **真违规（Tools 域）**：`Tool::call_inner` 生产路径自行 `open(sessions_db)`，非 session_search / 非 cfg(test) 文件。见 §7.5 mid 裁决：本波不修、不白名单。 |

门禁意图保持：sessions.db SQL/rusqlite 仍须集中在 SessionData sqlite（+ 既有白名单）；引擎自有索引与文件级测试不再误杀。

### 7.3 腿 B · ISSUE-036 落地证据（WP-068 close→done 后）

**契约（WP-063-runtime.md §4）：** `InternalEvent::TurnCompleted.error_kind: Option<TurnErrorKind>`；`CompactionFailed` → serde `"compaction_failed"`；controller 已传透；落盘 `turn/end` **无** `error_kind`。

**改动文件：**

| 文件 | 行 / 改动 |
|---|---|
| `src/client_protocol/project.rs` | import 增 `TurnErrorKind`（:3-6） |
| 同文件 | `TurnCompleted` 解构传 `error_kind` → `turn_error(..., *error_kind, ...)`（:482-491） |
| 同文件 | `fn turn_error` 签名增 `error_kind: Option<TurnErrorKind>`（:538-542）；Error 分支：`error_kind == Some(CompactionFailed)` → `ErrorCode::CompactionFailed` / `retryable:false` / message=`final_text`（:545-556）；**删除** `final_text == LitecodeError::CompactionFailed.to_string()` |
| 同文件 | `wp025_turn_error_tests`（:1208-1251）：用 `Some(TurnErrorKind::CompactionFailed)` 构造；另加「文案碰巧相同但 `error_kind=None` → 仍为 `internal`」对照 |
| `tests/client_protocol_project.rs` | import `TurnErrorKind`；`turn_finished_auto_compact_failure_is_compaction_failed` 改 `error_kind: Some(CompactionFailed)`，message 用任意文案（不再依赖 Display 字符串） |

**未改：** Runtime emit、`TurnEndReason`、wire schema、引擎产品、ISSUE-004；**未 commit**。

**llm_http / internal 映射：** 未改（仍按 `reconnect.is_failed()`）。

**测试命令与结果（2026-10-12 ~22:04 HKT，`rustc 1.99.0`，无 PATH 前缀）：**

```
cargo test --lib wp025_turn_error
  auto_compact_failure_maps_to_compaction_failed_not_retryable  ok

cargo test --test client_protocol_project turn_finished
  turn_completed_projects_to_turn_finished                      ok
  turn_finished_auto_compact_failure_is_compaction_failed       ok
  turn_finished_marks_a_failed_reconnect_retryable              ok
```

→ **4 passed / 0 failed**（相关过滤）。

### 7.4 Deliver 状态

- **两腿齐**：腿 A ISSUE-041 ✓；腿 B ISSUE-036 ✓。
- 下一步：`wp.mjs deliver --as session --id WP-069` → backend_mid 验收。
- **未 commit / 未 push**。

### 7.5 后端中层裁决（2026-10-12，WP-069 / 父包 WP-064）— 历史保留

1. **ISSUE-041 假红目标达成（认可）** — 不扩白名单、不改引擎。
2. **`litecode_workspace.rs:821` 真违规（Tools）** — 本波不修、不白名单；门禁可继续红该条 → 已挂 **ISSUE-042**（owner=tools）。
3. **ISSUE-036** — 原「等 WP-068」；现 WP-068 close→done，mid 放行 consume → **已落地见 §7.3**。

### 7.6 交叉审对齐（字段 ↔ turn_error）

| Runtime emit | Session consume |
|---|---|
| `TurnErrorKind::CompactionFailed` | `error_kind == Some(CompactionFailed)` → `ErrorCode::CompactionFailed` |
| `error_kind: Option<…>` / `serde(default)` | 缺省 / None → 不走 compaction；走 llm_http/internal |
| 不写入 `turn/end` | wire 仍只暴露 `StructuredError.code`（`compaction_failed`）+ `retryable:false` |
| message 可用 `final_text` | message = `final_text`（无则 `"compaction failed"`） |
