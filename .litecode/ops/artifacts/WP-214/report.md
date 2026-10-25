# WP-214 调研报告 — compact point 可点击性

**性质：** 只调研，无产品代码 diff；不合 main；未跑 cargo。  
**基线：** brief 钉产品 tip `2735592`；dig 时分支 tip `f49e6fb`（ops 批准提交；compaction 产品路径相对 `2735592` 未在本包改码）。  
**可点击定义（Q4）：** FE `CompactCutMark` 在 `summary.trim().length > 0` 时渲染 button + Popover；静态文案「compaction point」**不算**可点击。

---

## 0. 结论摘要

| 问题 | 结论 |
|---|---|
| 手动 compact 后，按设计是否应出现**可点击** compact point？ | **仅当**写出了带非空 `summary` 的 `kind=compacted` 行时：**应出现**。若走 **prune-only**（机械裁剪已降压、跳过 LLM summary）：**不应出现**（连切痕 mark 都没有）。 |
| 自动 compact 后？ | **同上**。手动 / 自动共用 `compact_transcript`；触发不同，checkpoint / prune 写出规则相同。 |
| Andy「手动后看不到可点击点」主因？ | **设计路径：prune-only 成功**（非 FE bug、非协议 kind 错位）。证据：产品明确「prune 减压则跳过 summary、不写 checkpoint」；FE 只认 `kind==="compacted"`。 |
| 主因归类 | **设计已废弃/本就不可点（prune-only）**。数据侧在 summary 成功时会正确写出 `compacted`；FE 消费条件与 wire 对齐。 |
| FE handoff？ | **否**（主因不在 FE）。可选产品跟进：prune-only 成功时是否给静态切痕 / 文案——属产品决策，另开包；本包不开实现子包。 |

---

## 1. 手动链路（证据）

```
ContextUsageRing compact 按钮
  → turnStore.compact(sessionId)
  → RPC session/compact
  → SessionController::start_manual_compact
  → CompactPolicy::compact_now(Manual)
  → CompactPolicy::compact_transcript(…)
```

| 步 | file:line | 要点 |
|---|---|---|
| UI 闸 | `web/src/components/ContextUsageRing.tsx` ~264, ~452 | `!connected \|\| runState!=="idle" \|\| compacting \|\| !compactEligible` 时禁用；点击 `compact(sessionId)` |
| FE RPC | `web/src/stores/turnStore.ts` ~655–676 | `sendRpc("session/compact", { session_id })`；本地先 `compacting:true` |
| 服务闸 | `src/client_protocol/controller/session_lifecycle.rs` ~323–335 | `meter.manual_eligible()`（含 30% of W）；否则 `NothingToCompact`。无 summary 切点时仍可因 `can_prune` 放行 |
| 执行 | `src/context_pipeline/compact.rs` `compact_now` ~727–802 | 调同一 `compact_transcript`，`CompactionTrigger::Manual` |
| prune-only 成功 | 同文件 ~951–998；`compact_now` ~755–764 | `Ok(None)` + lifecycle Succeeded，`summary_attempts=0`；**不写** `compacted`；`compact_now` 在减压时 `Ok(true)` |
| summary 成功 | 同文件 ~1219–1254；`session.rs` `apply_compact_checkpoint_checked` ~3376–3438 | `SessionMutation::Compact` → INSERT `kind="compacted"`，`CompactedBody{summary,from,to}` |
| 出站 | `controller/mod.rs` ~537–547 + `bump_buffer_revision` ~626–655 | Compaction `Succeeded` → bump → 加载 `[old_next,new_next)` → `buffer_log_row`（`kind` + `body`） |
| FE 消费 | `adapter.ts` ~33–35；`transcriptMarks.tsx` ~47–98, ~223–226 | `isCompactCutRow` ⟺ `kind==="compacted"`；`row.body.summary` 非空 → Popover 可点 |

**集成对照：** `tests/pr4_mechanical_prune.rs` ~830–898：手动 prune-only → lifecycle Succeeded、`summary_attempts=0`、**无** `EventType::Compacted`、仅有 `SurfaceOp::Replace` 的 item 行。

---

## 2. 自动链路（证据）

```
agent 缝隙 → CompactPolicy::compact_if_needed
  → ContextMeter::should_compact (used > compact_at)
  → 同一 compact_transcript(…, CompactionTrigger::Auto)
```

| 步 | file:line | 要点 |
|---|---|---|
| 触发 | `context_meter.rs` ~160+；`compact.rs` `compact_if_needed` ~806–910 | 严格 `used_tokens > compact_at` |
| 汇合 | `compact_transcript` ~919+ | 与手动同一执行体：先机械 prune，减压则跳过 summary |
| 成功语义 | ~868–905 | `did_compact`（有 artifact/checkpoint）或 `prune_applied` 都算本阶段改过 surface；prune-only 仍 **无** compacted 行 |
| FE 通知 | `turnStore.ts` `onCompactLifecycle` ~924–950 | auto `succeeded` → toast「Context compacted」；**不**因此造 mark |

结论：自动与手动在「是否出现可点击点」上**无分叉**——只取决于有没有非空 summary 的 `compacted` 行。

---

## 3. 对照表

| 路径结果 | 持久 | Wire / HumanRow | FE UI | 可点击？ |
|---|---|---|---|---|
| Summary checkpoint 成功 | `kind=compacted`，body.summary 非空 | `buffer/item`：`kind:"compacted"`, `body:{summary,from,to}`，通常**无** `surface_op` | `CompactCutMark` + Popover | **是** |
| Prune-only 成功 | 仅 item 级 `SurfaceOp::Replace`；**无** compacted | 替换后的 `item/*` 行 | 无 compact mark（可能仅结束 compacting / auto toast） | **否（设计）** |
| Compact 失败 | 无 checkpoint（已提交的 prune 不回滚） | Failed lifecycle | 无 mark；manual 走 operation error | 否 |
| 空 summary compacted | 验收拒绝，不该落库 | — | 若 equ 空 summary → 静态「compaction point」 | 否（Q4） |

---

## 4. 断裂点判定

### 4.1 Andy 现象最可能解释（主因）

**第一处「看不到可点击点」并非管线断裂，而是 prune-first 产品语义：**

1. `docs/compaction-architecture.md` ~17–20, ~101–102：prune 降到压力线以下则成功且可无 LLM；prune-only `summary_attempts = 0`。  
2. `compact_transcript` ~951–998：减压后 `return Ok(None)`（无 checkpoint）。  
3. FE `isCompactCutRow` 只认 `kind==="compacted"`（`adapter.ts:33-35`）。  
4. 测试钉死手动 prune-only 不写 Compacted（`pr4_mechanical_prune.rs:889-893`）。

因此：手动点 compact → UI 成功收尾 → **没有**可点击（甚至没有静态）compact point，在当前设计下是**预期**。

主因：**设计已废弃旧「每次 compact 都有可点切痕」预期 / prune-only 本就不可点**。

### 4.2 若 session 里其实有 `compacted` 仍不可点

那才进入 FE / 投影错位排查。本 dig **未**在 UI 复现；代码路径显示 summary 成功时：

- 持久 kind 字符串与 wire kind 均为 `"compacted"`（`apply_compact_checkpoint_checked` ~3418；`buffer_log_row` 用 `event.event_type.as_str()`）。  
- FE `HUMAN_VIEW_KINDS` 含 `"compacted"`；`MessageList` markOnly → `TranscriptMarkForRow`；未见 virtualizer 按 `compact_cut` 过滤掉。  
- 未发现「必须靠 `surface_op=Replace` 才认 cut」的 FE 逻辑（`messageStore.test.ts` 夹具也是 `kind:"compacted"`）。

**本包不判 FE bug**；Andy 可用附录步骤确认是否存在 `kind=compacted` 行。

### 4.3 `surface_op: None` vs 「Compact = Replace」叙事（澄清，非 bug）

| 层 | 行为 |
|---|---|
| Draft 写入 | `apply_compact_checkpoint_checked` ~3407：`surface_op: None`（刻意） |
| 校验 | `event.rs` ~300–308：Compacted **禁止**带 `surface_op`（「derives its spine replacement from body」） |
| 脊骨折叠 | `surface.rs` `plan_surface` ~147–169：用 `CompactedBody.from/to` 派生 `SurfacePlan::Replace`（或空脊 Append） |
| 机械 prune | `replace_surface_items` ~3311：item 行显式 `SurfaceOp::Replace`——**另一条** surface 重写，不是 compacted 切痕 |
| Wire → FE | compacted 行可无 `surface_op` 字段；FE 靠 **kind**，不靠 surface_op |

结论：draft 上 `surface_op: None` 与文档「Compact 在脊骨上 Replace `[from,to)`」**并存且一致**；Replace 落在 `plan_surface`，不是 wire 的 `surface_op`。不是 FE/数据错位。

---

## 5. DATA.md vs FE（契约张力）

- **DATA.md** `src/session/DATA.md` ~146：HumanView =「切痕，不展示摘要」；摘要由 `working.rs` projector 投成模型可见 user checkpoint，不回写。  
- **FE**：切痕文案「compaction point」；有 summary 时可点 Popover 展示摘要（`transcriptMarks.tsx` ~44–98）。

解读：

- 「切痕」与 FE 主展示一致（不是气泡正文）。  
- 「不展示摘要」若严格=UI 永不展示 → FE Popover 为**超设计**；若=不作为 transcript 正文展示 → FE 仍合规。  
- **该张力不是 Andy「完全看不到可点击点」的主因**（prune-only 时连切痕都没有）。  
- 建议后续：改 DATA.md 一句「HumanView 切痕；摘要仅 Popover 可选展示」或改 FE 去掉 Popover——产品二选一，**非本包**。

---

## 6. Prune-only 专节（Q5）

| 项 | 结论 |
|---|---|
| 是否写 `compacted` 行？ | **否** |
| 设计应否出 compact mark？ | **否**（无可点、也无静态 cut；替换的是 tool_result 等 item） |
| 与「可点击」关系 | **无关**——可点击前提是 compacted + 非空 summary |
| 用户体感 | compacting 结束；auto 另有「Context compacted」通知；transcript **无** compaction point 行 |
| 是否 bug？ | **否**（架构 + 测试明确允许） |

---

## 7. 建议下一步（仍不实现）

1. **Andy 复现确认（附录）：** 手动 compact 后查是否存在 `kind=compacted`。无 → 证实 prune-only，关 WP-214 即可。有且不可点 → 另开 FE 包。  
2. **产品（可选）：** prune-only 成功是否插入静态切痕 / 区分 toast「trimmed」vs「summarized」——需 Andy 拍板后新包。  
3. **文档（可选）：** DATA.md HumanView 与 FE Popover 对齐一句。  
4. **本包：****不** handoff `frontend_mid`；**不**开 session/context 实现子包。

---

## 8. Handoff → frontend_mid

**无。** 主因不在 FE。

（若后续 Andy 确认有 `compacted` 行仍无 Popover，再 handoff；建议范围：`adapter.isCompactCutRow` / `TranscriptMarkForRow` summary 传递 / `messageStore` 是否丢 kind。本包不立项。）

---

## 附录 A — Andy 本机最小复现

1. 打开一会话，上下文占用超过手动 30% 闸（环上 compact 可点）。  
2. 点 ContextUsageRing 的 Compact；等 compacting 结束。  
3. 在 transcript 中搜索文案 **「compaction point」**：  
   - **完全没有** → 高度疑似 prune-only（设计）。  
   - **有但不可点**（无 hover/button）→ summary 空或未传入（少见）。  
   - **可点且弹出摘要** → 当前设计正常；与「看不到」不符。  
4. 数据侧（任选）：对该 session 查 `transcript_items` 是否存在 `kind='compacted'`，读 `body.summary` 是否非空；或看 compaction lifecycle 遥测 `summary_attempts`（0 = prune-only）。  
5. 对照：故意堆超大 tool output 再手动 compact，更易走 prune-only；要逼出 summary，需 prune 后仍 `should_compact` 且存在 keep-recent 切点。

---

## 附录 B — 路径索引（dig-survey 补强）

见同目录 `dig-survey.md`；本报告引用以本节 file:line 为准。
