# Batch 2 · harness-core / context（上下文）目录清单

- 负责人：上下文（agent id `context`）
- 基线短哈希：`075308b`（`git rev-parse --short HEAD`，分支 `ops/team-framework`）
- 作用域：`src/context_pipeline/*`、`src/reminder/*`；知识节点挂在 `framework/context`、`contracts/context-view` 与 reminder
- 本轮：只钉目录 + pending 节点；**未做**实现核实（Batch 3）；**未** commit/push

## 1. 拟新增节点

| key | 路径 | 上层/父节点 | A# | 一句话承诺 |
|---|---|---|---|---|
| `reminder-log` | `knowledge/contracts/reminder-log.md` | 上层：`context` | A1 / A3 | 提醒是带独立 seq 的日志事实；正文写入时冻结，视图只渲染冻结文本 |
| `context-budget` | `knowledge/contracts/context-budget.md` | 父节点：`context-view` | A1 | provider prompt_tokens 是已发送前缀权威；后缀本地估算；超阈 compact、硬顶拒绝 |
| `context-compact` | `knowledge/contracts/context-compact.md` | 父节点：`context-view` | A5 / A7 | 唯一有损改写经 `SessionMutation::Compact`；落库后从 fold 重载；失败还原快照 |
| `context-keep-recent` | `knowledge/contracts/context-keep-recent.md` | 父节点：`context-view` | A1 | keep-recent 切割不拆 tool 对/assistant 段；无切点则跳过 compact |
| `context-media` | `knowledge/contracts/context-media.md` | 父节点：`context-view` | A1 / A3 | 媒体 show-once 只改瞬时 LLM 视图；不回写持久日志 |

未准入（刻意跳过）：
- prepare/commit 单入口单独成节点：已由 `context` / `context-view` 主张覆盖；Batch 2 只补符号依据，不拆新主节点。
- reminder kinds / sources 清单：函数/字段枚举，违反排除项。
- types/authority：非本切片（GM 指定留给他人）。

## 2. 拟保留 / 拟改的存量节点

| key | 动作 | 说明 |
|---|---|---|
| `context`（`knowledge/framework/context.md`） | **保留并打磨** | 补 `domain : harness-core`、`基线：075308b`；`契约展开` 增加 `reminder-log`；依据保持文件级（框架节点）。主张未改。已 `append_log`。 |
| `context-view`（`knowledge/contracts/context-view.md`） | **保留并打磨** | 补 domain/基线；`契约展开` 挂四个子契约；依据升级为 `PreparedView` / `build_view` / `commit_step` 符号。主张未改。已 `append_log`。 |

本切片拥有的上下文相关节点合计：**7**（存量 2 + 新增 5）。

## 3. 疑似主节点重复项

对照 `kb_check` `dup_primary_suspect`（本域最终跑次）：
- **无**对本切片节点的 dup 警告（已把父子共用的 `compact_transcript` / `build_view` / `render_item` 证据拆开：主节点持 prepare/commit，子节点持各自专属符号）。
- 启发式备注（非 warning）：`context-view` 与 `context-compact` 在文字上都谈「唯一有损出口是 compact」——主主张仍在 `context-view`；`context-compact` 只钉落库路径/fold 重载/死亡清单（A7），不复述保真条款。Batch 3 若仍觉得过近，可把 compact 段落收成半句指向语。

全库其它域的 dup（如 `contracts/tools/*`）不在本切片范围。

## 4. 本域 issue 草稿（Batch 3 再落盘）

| 临时 id | grade | layer | core | basis | title | promise / actual / evidence 要点 |
|---|---|---|---|---|---|---|
| DRAFT-CTX-001 | suspect | product | 是（context-view） | A5 | commit 后 `clear_prepared` 仅在 Idempotent 路径清空 PreparedView | 承诺：提交边界清晰。代码：`commit_step_with_turn` 仅 `CommitKind::Idempotent` 时 `clear_prepared=true`，其它成功提交保留旧 prepared。需 Batch 3 核对调用方是否在每次 build_view 前强制重建，否则可能读到过期视图。证据：`impl ContextPipeline › fn commit_step_with_turn`。缺：跨调用方复现。 |
| DRAFT-CTX-002 | suspect | product | 否（reminder-log） | A2 | Mentions 标注「compaction 后不恢复」，与其它 Diff 种恢复策略并置 | 承诺：spine 基线决定 Restore。`ReminderKind::Mentions` 注释写 Not restored after compaction；`restore_fires_once_after_compaction` 覆盖部分 kind。需 Batch 3 列全 kind 的 Restore 表，确认 Mentions/KnowledgeStatus 与注释一致。证据：`kinds.rs` Mentions 注释；`mod tests › fn restore_fires_once_after_compaction`。 |
| DRAFT-CTX-003 | drift | gate | 否 | A7 | （观察）本域死亡清单 `session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept` 以源码子串断言 Compact 后重载 | 守门有效但脆弱（改注释/换 API 名即红）。属 gate 层，不阻断；Batch 3 可记是否改成符号/行为测试。证据：`src/types/death_list_gate.rs` 该 fn。 |

均未触达 B1–B5。无 arch。不阻断。

## kb_check（本轮）

```
node .litecode/ops/tools/kb_check.mjs
```

- 全库：exit **0**（`ok=true`）；errors=**0**；warnings≈92（多为他域存量 domain/baseline/parent 提示）
- 本域新节点（`--node` 各跑）：`reminder-log` / `context-budget` / `context-compact` / `context-keep-recent` / `context-media` → **errors=0 warnings=0**
- 本域存量：`context` → errors=0，warnings=1（`inactive_target` → pending `reminder-log`，准入允许）
- `context-view` → errors=0，warnings=4（`inactive_target` → 四个 pending 子契约，准入允许）

## 统计

| 项 | 数 |
|---|---|
| 新增 pending 节点 | 5 |
| 打磨 enabled 节点 | 2 |
| 本切片上下文相关节点合计 | 7 |
| 本轮 git commit/push | 无 |

