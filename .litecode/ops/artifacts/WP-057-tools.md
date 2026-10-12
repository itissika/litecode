# WP-061 Tools 方案（tools，direct）— 工具可见性 + 废暖等 + sync% 结果头 + session 降级表面

父包 WP-058（in_progress）← WP-057；产物本文件 `.litecode/ops/artifacts/WP-057-tools.md`。  
状态：**已落地，待验收**；可与 IDE WP-060 并行交叉审。不 commit。

## 0 目标与范围

按 WP-057 锁定口径落地**工具面**（映射 ISSUE-023–026）：

| # | 必须 | Issue |
|---|---|---|
| V1 | Warming **或** `desired=false` ⇒ `code_search` / `session_search` **不可见且不可用**（禁「看得见、点了才难用」） | 023 |
| V2 | 废除 `CODE_SEARCH_WARM_WAIT` 同步轮询长等 | 024 |
| V3 | 文案跟可见性；仅需启用路径时指路 Settings→Engines | 025 |
| V4 | 非暖可搜；session 语义腿不可用时**显式降级表面**，禁止静默假完整 | 026 |
| V5 | 工具结果**正文开头第一行**固定 `sync_pct: <n>%`（契约 `WP-057-sync-pct-contract.md`） | 口径#6 |

**主改**：`src/tools/code_search.rs`、`src/tools/session_search.rs`；`src/tool/availability.rs`（及必要时 `registry`/`build_tool_list` 消费点）。  
**只消费** IDE 可搜 / Warming / `IndexingProgress`·`resolve_index_view` 信号；**不改** engines 重建核心、**不改** `WorkspaceRetrievalState.desired` 默认 false。  
**禁区**：symlink 产品口径；不抢改 `client_protocol`；不 commit/push。

## 1 现状锚点（只读核对）

| 点 | 位置 | 现状 vs 口径 |
|---|---|---|
| 暖等 | `code_search.rs:25-26,111-156` | `CODE_SEARCH_WARM_WAIT=30s` + `thread::sleep` 轮询；与 tool timeout 30s 对齐 → **废** |
| desired 可见性 | `workspace.rs` readiness：`desired`→`code_search` Ready；`availability.is_available` 对 optional 看 Ready | **desired=false 已藏 code_search**；**不改默认** |
| Warming 可见性 | `should_include_in_llm_list` 收 `_workspace_engines` **未用** | Warming 时工具仍可进 LLM 表 → **缺口** |
| session_search 身份 | `global_db/tools.rs`：**core**（非 optional） | 不受 desired/Warming 门 → **需加引擎门** |
| 等待文案 | `indexing_wait_message` 仅「Try again shortly」 | 与 webfetch「Enable … Settings → Engines」断裂 → **跟可见性改** |
| 语义静默降级 | `engines::search_sessions` gate≠Ready → `semantic=Vec::new()` 无表面；工具描述未声明 | Tools 结果面须**显式**降级（不改引擎融合算法也可在工具层标头） |
| sync% | 结果无首行 `sync_pct`；进度仅在 wait 文案 | 按契约在成功/空命中结果**开头**落行 |

## 2 设计（批后实施）

### 2.1 可见性门（V1 / ISSUE-023）

在 LLM 工具表组装路径（优先 `should_include_in_llm_list`，必要时对称改 `is_available` 对检索类的扩展）增加：

对 `code_search` **与** `session_search`：

1. **`desired=false`**（`workspace_engine_desired(root,"code_search")` / 等价 readiness 非 Ready）⇒ **排除**。  
   - code_search：保持现有 optional Ready 链路即可。  
   - session_search：虽为 core，本包对这两 id **额外**套同一 desired 门（产品口径：关 retrieval = 两工具都不可见）。
2. **Warming**（消费 IDE：`!is_warmed("code_search")` 且 state 为 Warming / 或 WP-060 书面「可搜=false」信号；**以 IDE 落地信号为准，交叉审对齐**）⇒ **排除**。  
   - 禁止仅靠「调用后再 warning」。
3. 非暖且 desired=true ⇒ **可见**；瘦索引/空命中仍可调用（没命中≠不能搜）。

防御：若绕过列表直接 `call`（测试/内部），`call_inner` / `execute` **立即失败或空可用**，**禁止**再 sleep 暖等。

### 2.2 废除暖机长等（V2 / ISSUE-024）

- 删除 `CODE_SEARCH_WARM_WAIT` / `CODE_SEARCH_WARM_POLL` 及 `call_inner` 内两段 `while … sleep`。  
- 删除/改写单测 `waits_up_to_thirty_seconds_then_searches_not_loading`。  
- 可见性已保证 Warming 不进表；若状态竞态仍进调用：直接 `error`/`warning`（短、可行动），**零阻塞轮询**。  
- 不在 Tools 侧重做「睡到 Ready」。

### 2.3 文案跟可见性（V3 / ISSUE-025）

| 场景 | 文案策略 |
|---|---|
| 工具因 desired=false / Warming **不可见** | 无「假可用」结果面；不依赖 agent 读 wait 文案 |
| 防御路径：desired=false 仍被调用 | 明确：「Enable code search / retrieval in Settings → Engines」（对标 webfetch） |
| 非暖可搜但索引 Building/Refreshing（若 IDE 仍允许可搜+后台刷） | 结果**仍可返回**；开头 sync_pct；正文可附短提示，**不代替**命中/空命中叙事 |
| 空命中 | 先 `sync_pct` 行，再「No matching…」；不把「库旧」当主叙事 |

废止主叙事：「空结果才谈库旧」；「Try again shortly」不再作为 desired 关/未启用时的唯一出口。

### 2.4 session 语义腿诚实表面（V4 / ISSUE-026）

Tools 层（`session_search.rs` 组结果时）：

1. 探测本趟是否实际带上语义腿（调用前读 `session_semantic_gate` / 或引擎 bundle 侧可观测标记；**优先只读信号**，避免改 `fuse` 核心除非必要且经中层）。  
2. 仅词法：结果开头（sync_pct 之后）加一行明确降级，例如 `semantic: unavailable (lexical only)`；**禁止**看起来像「完整融合检索」。  
3. schema/`description`：补一句——语义腿依赖 retrieval/code_search Warm；不可用时降级为词法并在结果中声明。  
4. **不**在查询前强制 refresh（保持现注释接受 stale）；滞后只许少召回，但表面诚实。

引擎 `search_sessions` 静默 `Vec::new()`：**本包以工具结果表面闭环**；若交叉审要求引擎返回 `semantic_used: bool`，开窄协助或登记 follow-up，不阻塞 V4 最小闭环。

### 2.5 结果头 sync_pct（V5 · 契约）

严格按 `.litecode/ops/artifacts/WP-057-sync-pct-contract.md`：

1. `code_search` / `session_search` **成功或可展示结果**（含空命中）时，正文**第一行**：  
   `sync_pct: <n>%`  
   可选同行列 `sync_pct: 87% (syncing)`。  
2. **数据源**：IDE `IndexingProgress` / `resolve_index_view`（或 WP-060 暴露的单一百分比 API）→ Tools **只消费、不发明第二套进度**。  
   - 换算：有 `files_total>0` 时 `files_done*100/files_total` clamp 0–100；Ready/非刷且可搜 → 100；Warming/无视图 → **无结果视图**（工具不可见），不发假 100%。  
3. 刷新中：中间值 + 可选 `sync_label`（syncing/building）。  
4. **不改** `client_protocol`；前端 WP-059/062 解析首行数字。

辅助：抽 `fn format_sync_pct_line(...)` 两工具共用，单测锁格式。

### 2.6 与 IDE WP-060 交叉点

| 信号 | IDE（060） | Tools（061） |
|---|---|---|
| Warming / 可搜 | 状态机 + 可搜语义 | 可见性门消费 |
| sync% 数据源 | progress→pct | 结果首行 |
| DirtyTooLarge / 重建 | IDE 废止 | **不改** |

交叉审：双方产物互写「可搜信号名 / sync% 字段对齐」；IDE 信号未稳时 Tools 可先按现有 `is_warmed`+`resolve_index_view` 接线，060 定稿后只改消费点。

## 3 文件与串行

| 序 | 文件 | 要点 |
|---|---|---|
| 1 | `src/tool/availability.rs`（± registry 组装） | desired + Warming 门；session_search 例外门；测 |
| 2 | `src/tools/code_search.rs` | 删暖等；防御失败；sync_pct 头；文案；测 |
| 3 | `src/tools/session_search.rs` | 可见性防御；降级表面；sync_pct 头；description；测 |
| 4 | 产物本文件 | 证据表 + 交叉审 + 与 023–026 状态建议 |

不改：`src/engines/**` 重建/`decide_index_work`、desired 默认、symlink、前端、协议文件。

## 4 测试计划（批后绿）

| # | 场景 | 期望 |
|---|---|---|
| T1 | desired=false | 两工具不在 LLM/available 表 |
| T2 | Warming（test set_state） | 两工具不可见；直接 call 无 sleep、快速失败 |
| T3 | desired=true + Warm | 可见；搜索可跑；无 WARM_WAIT 符号/循环 |
| T4 | 空命中 | 首行 `sync_pct:` + No matching |
| T5 | 有命中 | 首行 sync_pct；格式正则 |
| T6 | session 语义门非 Ready 且工具仍可见（若口径允许非暖可搜仅词法）或等价注入 | 结果含显式 lexical-only/unavailable，非静默 |
| T7 | 既有 code_search / session_search 单测回归 | 全绿；删/改暖等旧断言 |

## 5 非目标 / 禁区

- 不把 retrieval **改默认开**（口径#7 / ISSUE-023 产品解法是隐藏，不是默认开）。  
- 不改 engines DirtyTooLarge / 重建策略（属 WP-060）。  
- 不改 symlink；不 commit。  
- 不抢 `client_protocol`。  
- ISSUE-034 不在本包。

## 6 done-when

1. 暖等常量与轮询删除；Warming/desired=false 两工具不可见有测。  
2. 结果头 sync_pct 契约对齐；空命中亦有。  
3. session 降级表面诚实；相关测绿。  
4. 产物齐；交叉审（对 IDE）写入；023–026 建议 closed/改写注记交中层。  
5. 禁区未碰；未 commit。

## 7 风险与依赖

- **信号名**：以 WP-060 产物为准；未出前用现有 `is_warmed` / `resolve_index_view` / `IndexingProgress`。  
- **session_search 变相 optional**：core 工具加 desired 门是产品口径显式要求；若中层裁「仅 Warming 藏 session、desired 只藏 code_search」，按批注改门，不扩大。  
- 并行 WP-060：代码面分家；冲突仅信号，中层钉死。


---

## 落地证据（deliver · 2026-10-12 21:21 HKT）

状态：**已落地，待 backend_mid 验收**；**未 commit**。

### 实现对照

| 项 | 落点 | 证据 |
|---|---|---|
| V1 可见性 | `src/tool/availability.rs` `should_include_in_llm_list`：`desired=false` 或 `!is_searchable("code_search")` ⇒ 隐藏 `code_search`+`session_search` | 测：`retrieval_hidden_when_desired_false` / `retrieval_hidden_while_warming_even_if_desired` / `retrieval_visible_when_desired_and_warm` |
| V2 废暖等 | `src/tools/code_search.rs`：删除 `CODE_SEARCH_WARM_WAIT`/`POLL` 与 `thread::sleep` 轮询；防御路径零阻塞 | 测：`warm_wait_constants_removed` / `warming_returns_immediately_without_sleeping` / `warm_stale_is_searchable_without_warm_wait` |
| V3 文案 | desired 关 ⇒ Settings→Engines；Warming/未就绪短文案；不再以「长等后再试」为主 | 测：`desired_false_defensive_points_at_settings` / session 同路径 |
| V4 session 降级 | `session_search`：语义门非 Ready ⇒ 结果含 `semantic: unavailable (lexical only)`；description 已声明 | 测：`semantic_unavailable_is_explicit_and_leads_with_sync_pct` |
| V5 sync_pct | `tools::format_sync_pct_line` / `prepend_sync_pct` / `sync_pct_for_workspace` 消费 IDE `sync_pct_from_view`；成功/空命中首行 | 测：`empty_hits_lead_with_sync_pct` + session 首行断言 |
| 静默刷新对齐 | 与 IDE WP-060：`Refreshing`/`refresh_busy` 不 Wait | 测：`warm_refreshing_does_not_block_on_wait` / `warm_refresh_busy_does_not_block_on_wait` |

### 编译 / 回归

- `cargo test --lib --no-run`：**通过**（修复 Instant/Duration 与 session `.into()` E0283 后）
- 定点 30 测（availability retrieval_* + code_search 相关 + session_search 全组）：**30/30 绿**

### 未改

- `WorkspaceRetrievalState.desired` 默认 false
- engines 重建核心 / DirtyTooLarge（属 IDE）
- symlink / `client_protocol` / 未 commit

### 交叉审（对 IDE）

- 消费 `is_searchable` + `sync_pct_from_view`/`IndexingProgress`；与 WP-060 口径一致。
- IDE 产物若后补信号名，仅改消费点。

### ISSUE 建议

- ISSUE-023–026：建议中层在验收后 closed（本包已按口径落地工具面）。
