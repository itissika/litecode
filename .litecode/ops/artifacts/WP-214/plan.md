# WP-214 方案 — compact point 可点击性调研（只读）

**owner：** backend_mid  
**分支：** `feat/tool-contract-ptc`（不合 main）  
**基线：** brief 钉 `2735592`（`fix(compaction): use selected context window for pressure`）；ops tip `39ad549` 仅开包。本包不改产品码。  
**依据：** brief；Andy 实测——手动 compact 后看不到可点击 compact point  
**性质：** **分析包**——交付 `report.md`；**本包不改码 / 不刷 fixture / 不 cargo / 不 start 实现**

---

## 1. 目标

弄清手动 / 自动 compact 之后，按**当前设计**是否仍应出现**可点击**的 compact point；若未出现或不完整，标出管线第一处断裂，主因归为三者之一（可组合但主因唯一）：

1. **FE bug**（渲染 / 判定 / 点击条件）
2. **数据结构或协议错位**（session 指针 / 投影 / wire / HumanRow）
3. **设计已废弃旧点位**（非 bug；例如被 checkpoint / surface_op / 机械 prune 语义替换）

若断点在 web：报告 **handoff → `frontend_mid`**（可建议子包），本包仍只出报告。

---

## 2. 已知触点地图（fill 前只读扫过；结论以 dig 为准）

| 段 | 路径（示意） | 要核实什么 |
|---|---|---|
| **触发·手动** | `web/.../ContextUsageRing.tsx` → `turnStore` `compact_session` → `client_protocol` `start_manual_compact` → `context_pipeline/compact.rs` `compact_now` | UI 点击是否真走到 commit；eligibility 闸（meter 30%）是否挡住 |
| **触发·自动** | `ContextMeter::should_compact`（`used > compact_at`）→ `compact_if_needed` → 同 `compact_transcript` | 与手动是否共用同一套 checkpoint 写出 |
| **执行** | `compact_transcript`：机械 prune 可单独成功；否则 summary 循环 → checkpoint | **无 summary 的 prune-only** 是否根本不产「可点击」切痕 |
| **持久** | `Session::apply_compact_checkpoint_checked`（`session.rs` ~3376+）：kind=`compacted`，`CompactedBody{summary,from,to}`；`write_compact_pointers` / `refresh_compact_pointers_from_log`（~861+） | 写出字段；`surface_op` 实际落在哪一行（draft 上曾见 `surface_op: None` + `source_seqs`——须核对 Replace 如何进脊骨） |
| **出站 / 投影** | `client_protocol/project.rs`、session snapshot / transcript 推送；`working.rs` projector（模型可见 checkpoint ≠ UI 行） | Human/UI 行是否带 `kind:"compacted"` + `body.summary`；与模型投影是否两套 |
| **FE 消费** | `api/adapter.ts` `isCompactCutRow` / `transcriptMarkKind` → `transcriptMarks.tsx` `CompactCutMark` → `MessageList` `TranscriptMarkForRow` | **可点击条件 = summary 非空**（有 summary → Popover button；无 → 静态「compaction point」）；点击期望的 id/seq 是否还存在 |
| **契约文档** | `docs/compaction-architecture.md`；`src/session/DATA.md`（compacted：「切痕，不展示摘要」） | 设计上「应可点」还是「只切痕不可点」——与 FE 实现是否一致 |

**初步张力（待 dig 证伪/证实，不作结论）：** DATA.md 写 compacted「不展示摘要」，FE 却在有 summary 时可点开摘要；Andy「看不到可点击点」可能是：行未出现 / 行出现但 summary 空（静态）/ 行 kind 改名未对齐 / prune-only 无 compacted 行 / 设计本就不该可点。

---

## 3. 调研步骤（批准后执行；全部只读）

### 3.1 触发源对照

1. 从 `ContextUsageRing` / `turnStore` 画手动 RPC：`compact_session` → `start_manual_compact` → `compact_now`，标 file:line。  
2. 从 `compact_if_needed` / meter pressure 画自动路径，标是否汇入同一 `compact_transcript`。  
3. 确认二者 commit 是否都走 `apply_compact_checkpoint_*`（或 prune-only 旁路）。

### 3.2 持久与投影

1. 读 `apply_compact_checkpoint_checked`：INSERT kind、body 形状、`surface_op` / `source_seqs`、sessions 指针四元组（`checkpoint_seq` / `compacted_seq` / `kept_from_seq` / `spine_from`）。  
2. 对照 `refresh_compact_pointers_from_log`（查 `kind='compacted'`）与 reload 后 UI history SQL（含 `compact_checkpoint` 注释处）——**命名是否两套**。  
3. 追出站：snapshot / event push 里 compacted 行如何变成 FE `HumanRow`（字段路径 + 示例 JSON，可从现有 test fixture 摘）。  
4. 机械 prune 成功且无 LLM summary 时：日志里有无 `compacted` 行、FE 是否仍应出 mark。

### 3.3 前端消费（对照，本包不改 FE）

1. `adapter.isCompactCutRow`：仅 `kind==="compacted"`？其他 Replace / checkpoint 形态是否漏判。  
2. `CompactCutMark`：可点击 ⟺ `summary.trim().length > 0`；无 summary 时仍渲染静态文案——Andy「看不到可点击」是否其实是看到静态切痕。  
3. `MessageList` / `NodeView`：mark 是否被 virtualizer / filter（如 `n.kind !== "compact_cut"`）藏掉。  
4. `messageStore` / `turnStore` 对 `surface_op=replace` 与 compact checkpoint 的归类是否把 cut 当 user anchor 误吞（已有 test 线索，须核对现 tip）。

### 3.4 设计对照与主因判定

1. 用 `compaction-architecture.md` + DATA.md 回答：手动 / 自动**各自**「应出现可点击 compact point？」  
2. 若不应：记 **设计已废弃/本就不可点**，并写清现行应出现的 UI 形态。  
3. 若应出现：沿链路标**第一处断裂**（FE vs 数据/协议），主因唯一；FE 断则 **handoff → frontend_mid** 段落（证据 + 建议子包范围，不改码）。

### 3.5 产物

- `artifacts/WP-214/report.md`：结论摘要 → 手动链路 → 自动链路 → 对照表 → 断裂点 → 建议下一步（仍不实现）。  
- 可选：Andy 本机最小复现步骤。  
- 可选：`dig-survey.md` 附录（路径摘录）。  
- **无产品代码 diff**；不合 main；不跑 cargo（分析包）。

---

## 4. 分工

| 角色 | 职责 |
|---|---|
| **backend_mid** | 统筹整条链路 dig + 写 `report.md`；fill/submit/deliver |
| **Session** | 按需答疑 pointer / `apply_compact_checkpoint` / surface fold（不改码） |
| **上下文** | 按需答疑 `compact.rs` / meter / prune vs summary（不改码） |
| **frontend_mid** | **仅当**报告判定 FE 断点时作为 handoff 接手方；本包不派 FE 改码子包 |

无重型 cargo；无串槽实现冲突。

---

## 5. 验收（与 brief done_when 对齐）

1. 报告分别回答手动 / 自动：设计上是否应出现可点击 compact point。  
2. 未出现或不完整时：断裂位置 + 主因 ∈ {FE bug, 数据/协议错位, 设计已废弃}。  
3. 关键结论附 **file:line**（或协议字段路径 + 示例 JSON）；手动与自动至少各一条证据链。  
4. 产物在 `artifacts/WP-214/`；无产品码 diff；未合 main。

---

## 6. 拍板（Q）

| # | 议题 | 推荐 |
|---|---|---|
| Q1 | 本包只产出 `report.md`（+可选 dig-survey）；发现的改码项一律建议后续包 / handoff，本包不改 | **是** |
| Q2 | dig 覆盖触发→持久→出站→FE 消费全链路；FE 只对照读码，不改 web/desktop | **是** |
| Q3 | 基线产品行为以 `2735592`（及其上 feat 分支至 dig 时 tip 的 compaction 相关提交）为准；ops 开包提交不计入产品基线 | **是** |
| Q4 | 「可点击」采用现行 FE 定义：`CompactCutMark` 在 summary 非空时出 button/Popover；静态「compaction point」文案不算可点击 | **是** |
| Q5 | 机械 prune-only（无 LLM summary / 无 `compacted` 行）单独成节：设计应否出 mark、与「可点击」是否无关 | **是** |
| Q6 | 若 DATA.md「不展示摘要」与 FE 可点摘要冲突：报告写清契约 vs 实现，主因可判「设计文档过时」或「FE 超设计」，但仍给 Andy 可执行的下一步建议 | **是** |
| Q7 | FE 断点 → 报告内 handoff `frontend_mid`；数据断点 → 建议域 `session` / `context`；本包都不开实现子包 | **是** |
| Q8 | 不跑 cargo / 不改产品码 / 不合 main | **是** |
| Q9 | Andy 本机复现步骤写入报告附录（基于代码路径推断；无法在本环境点 UI） | **是** |

---

## 7. 风险与备注

- **命名漂移**：代码/注释混用 `compacted` / `compact_checkpoint` / CompactionCheckpoint——dig 必须钉死 **SQL kind 字符串** 与 **wire kind** 是否一致，避免假错位。  
- **prune vs summary**：自动压力路径可能只 prune 成功——「看不到可点击点」在设计上可能是预期。  
- **双投影**：`working.rs` 把 checkpoint 投成模型可见 user 消息；UI 消费另一套 HumanRow——勿把模型侧有摘要误当成 UI 可点。  
- **脏工作区**：`desktop/build/icon.ico`（D）与 `tool_traces.jsonl` 勿入本包提交。  
- fill 前提示：`apply_compact_checkpoint_checked` draft 上 `surface_op: None` 与「Compact = Replace」文档叙事并存——优先澄清 Replace 落点，再谈 FE。
