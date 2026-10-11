# Batch 3 · harness-core / context（上下文）实现对照

- 负责人：上下文（agent id `context`）
- 基线短哈希：`075308b`（`git rev-parse --short HEAD`，分支 `ops/team-framework`）
- 作用域：`src/context_pipeline/*`、`src/reminder/*`；对照调用方 `src/agent`、`src/runtime`、`src/client_protocol`（compact 失败路径）
- **未** commit / push

## 1. 节点对照表

| key | 结论 | 证据（文件 + 符号 + 测试/行为） |
|---|---|---|
| `context` | **一致** | `build_view` / `take_prepared_view`；`compact_transcript`；`drop_stale_media`；`agent::run` 中 `compact_if_needed` Err → `TurnOutcome::Error`；`reminder::render_item` |
| `context-view` | **一致** | `PreparedView` + `build_view`（副本上 pad/project/media）；`commit_step_with_turn`；compact 失败还原 snapshot；死亡清单 g2（gate 脆弱 → ISSUE-013） |
| `reminder-log` | **一致** | `Reminder::text` 冻结；`sync` 只追加；Restore=Tasks/Background；Mentions 不恢复与注释一致；DATA.md 种表缺项 → ISSUE-014（文档，不改节点主张） |
| `context-budget` | **一致** | `token_count_with_baseline`；`should_compact`；`enforce_hard_limit_with_baseline`；测试 `token_count_prefers_provider_when_set` |
| `context-compact` | **一致** | `SessionMutation::Compact` → `working_set_blocking`；失败还原 snapshot；auto/manual 口径见 §6；g2 门 → ISSUE-013 |
| `context-keep-recent` | **一致** | `find_keep_recent_cut` + `adjust_cut_for_tool_pairs` / assistant 段；`cut==0`→None；测试 `tool_pair_not_split` |
| `context-media` | **一致** | `drop_stale_media` 仅作用于 `build_view` 的 `llm_items` 副本；`strip_media_for_summary`；模块钉「persisted transcript is never mutated」 |

七节点均已按准入 §7.2 追加「核实结论：一致，无漂移。」**无结论 B。无 core 节点无法拓展 → 不触 B2。**

## 2. 正式 issue（ops schema → `.litecode/ops/issues/`）

| id | 来源 | grade | layer | core | status | blocks | 处置 |
|---|---|---|---|---|---|---|---|
| **ISSUE-013** | DRAFT-CTX-003 升级 | drift | gate | 否 | open | 否 | 死亡清单 g2 以源码子串断言 Compact 后重载；产品不变量仍立。建议后续改符号/行为测试（删/松须另记理由，禁静默为绿） |
| **ISSUE-014** | DRAFT-CTX-002 连带 | drift | product（文档） | 否（pending reminder-log） | open | 否 | `DATA.md` reminder 种表缺 Mentions / KnowledgeStatus / CustomToolSettled；代码与节点一致，文档待改 |
| DRAFT-CTX-001 | Batch2 草稿 | — | — | — | **关闭** | — | 见下：非 bug |
| DRAFT-CTX-002 | Batch2 草稿 | — | — | — | **关闭** | — | 见下：意图如此；文档缺口转 ISSUE-014 |

### DRAFT-CTX-001 关闭证据（clear_prepared 仅 Idempotent）

- `commit_step_with_turn`：仅 `CommitKind::Idempotent`（Discarded）时 `clear_prepared=true`；Appended/Sealed 不主动清。
- Agent 环每步：`prepare_view`（`build_view` 覆写 prepared）→ `call_model`（`take_prepared_view` 取走）→ 再 `persist`/`commit`。成功提交时 prepared 已被 take；Idempotent 清空是丢弃路径卫生。
- `begin_turn` / `end_turn` 亦清 prepared。
- **结论：** 无过期视图读回主路径；不改产品代码；不升 formal open issue。

### DRAFT-CTX-002 关闭证据（Mentions 不恢复）

- `ReminderKind::Mentions` 注释：「Not restored after compaction。」
- `engine::sync` 不含 Mentions/KnowledgeStatus；Restore 源仅 `tasks` / `background`（看 `compacted_head` / `kinds_after_compacted`）。
- Mentions / KnowledgeStatus 经 `append_user_message_with_mentions` → `append_mentions_for` / `append_knowledge_status_for` 附着。
- 测试 `restore_fires_once_after_compaction` 覆盖 Tasks Restore。
- **结论：** 节点主张与实现一致；DATA.md 缺种 → ISSUE-014。

## 3. 待删 / 待改非知识 md（本切片撞车）

| 路径 | 动作 | 理由 |
|---|---|---|
| `src/session/DATA.md`（reminder/* 表，约 L143–166） | **待改**（经 GM） | 缺 Mentions / KnowledgeStatus / CustomToolSettled；与 `ReminderKind` + reminder-log 节点不一致。SessionLog 词汇噪声按 Andy 口径交 Session 域，本切片不扩写 SessionEvent 题。 |
| （无待删本域专属旧 md） | — | 本切片无第二套 context 契约 md 与节点撞车到「须删」级别。 |

## 4. 互引补全（本侧，不改主张）

| 节点 | 补全 |
|---|---|
| `reminder-log` | 新增 `相关：[@ key="session-model"]` |
| `context-compact` | 新增 `相关：[@ key="session-model"]` |
| `context` / `context-view` | 已有契约展开 / 相关；未改主张 |

## 5. 阻断声明

**无。**

- arch=0；无结论 B（B2 否）
- ISSUE-013 `layer=gate` 不计入 B4/B5
- ISSUE-014 `core=false`（pending）不计入 B4
- 不触 B1/B3

## 6. Compact 口径（Andy）专项

| 口径 | 实现 | 判定 |
|---|---|---|
| **auto compact 失败 → turn 终止** | `CompactPolicy::compact_if_needed`（`CompactionTrigger::Auto`）`Err` 经 `ContextPipeline::compact_step` → `AgentDeps::compact_if_needed` → `agent::run`：`Err(e) => return TurnOutcome::Error(e)`；runtime 再 `emit_turn_completed(..., Error, ...)` | **兑现** |
| **手动 compact 失败 → 可重试** | `start_manual_compact` 拿 `SessionOperationKind::Compact` lease → spawn `compact_now`；失败走 `CompactionStage::Failed`；任务结束 `drop(lease)`，会话可再次 `start_manual_compact` | **兑现** |
| subagent / 不可重试 | 子会话走同一 `agent::run` + `compact_if_needed`；auto 失败同样终止**该** child turn（非「锁住父 session 永久」）。无第二套 dig 旁路 compact。 | **与口径一致**；未发现「有锁但锁不住砸体验路径」的本域缺口 |
| 硬顶 | 无法 compact 仍超 `budget_limit` → `enforce_hard_limit_*` Err → 同上终止 turn | 与 auto 失败同路 |

## 7. kb_check

```
node .litecode/ops/tools/kb_check.mjs
node .litecode/ops/tools/kb_check.mjs --node <key>
```

- 本域 7 节点 `--node`：**errors=0**（context warnings≈7 dup/inactive；context-view≈8；reminder-log≈1；context-budget=0；context-compact≈3；context-keep-recent=0；context-media≈2）
- 全库：errors=**15**（**均非本切片节点**）。他域 Batch 3 结论里写了「守护：/结局：/实现：/测试：」等未准入关系标签，或 symbol 链写错（agent-loop、item-authority、llm-stream-contract、platform-knobs、stream-projection、provider-catalog、ask-surface、layout-ssot、turn-llm-binding、theme-tokens）。本切片已把结论里的「守护：/门：」改成「测试见 / 死亡清单」避免同病。
- 结论模板 §7.2 示例含「守护：」会在 pending 上变成 error——建议 GM 修订模板用词。

## 7.1 跨域知晓（非本切片主责）

- **ISSUE-018**（runtime，`drift` / harness-core）：`LitecodeError::CompactionFailed` 文案称 3 attempts 与空摘要路径不符。不改变本切片「auto 失败终止 / 手动可重试」结论；若 GM 归并 compact 文案债可挂 context-compact 旁听。

## 8. GM 决策（可选，非阻断）

1. ISSUE-013：是否排期把 g2 门从子串改为符号/行为测试（gate 债）。
2. ISSUE-014：授权改 `DATA.md` reminder 种表（本切片已标待改，未动文件）。
3. DRAFT-CTX-001 关闭是否需 GM 签收（证据已在上）。

