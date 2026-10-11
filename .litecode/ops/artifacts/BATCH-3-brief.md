# Batch 3 · 实现对照核实（GM 启动）

- 日期：2026-10-12（HKT）
- 前置：Batch 2 目录齐 + 对齐报告；Andy 已定本轮口径（见下）
- 工作区：`/workspace/litecode`，分支 `ops/team-framework`
- **不 commit / 不 push**（除非 GM 另行通知）
- 对 Andy 的报告：由 GM 聊天附件交付，**不要**把「给 Andy 看的汇总」当成唯一渠道依赖仓库

## 目标

对 Batch 2 钉下的**本域节点**（含你负责的存量父节点），逐条核实：**实现是否兑现节点承诺**。

每条结论二选一（须带证据链）：

1. **一致，无实质漂移** — 承诺 ↔ 代码/测试符号 ↔ 行为，链完整。
2. **不能当稳定知识扩写** — 因严重问题（按 issue 分级）；记正式 issue，证据链说明卡在哪。

严重 **architecture** 且触达阻断阈值 → 停本域并向 GM 升级，勿自行改产品形状。

## Andy 已定口径（全员遵守）

| 题 | 口径 |
|---|---|
| 知识目标 | 构建**唯一知识真相**；仓内杂 md 仅参考；节点稳定后，冲突/过时的其他来源 md **删除**（本批先标「待删清单」，删前经 GM） |
| SessionLog vs SessionEvent | **内部词汇/文档噪声**：以代码侧 `SessionEvent` 为准，改文档或过时门禁；**不是**用户功能选型 |
| 多 session Ask | **一 session 只管自己的 Ask**；跨 session 用整条 WS 串行卡住别人 = **错，要改**（DRAFT-CUX-1 → 按 bug/drift 核实落盘） |
| config-commit 半写入 | 边界情况。为更强一致性/健壮性可增强；不是「日常写崩」。节点写清或实现加固均可，Batch 3 给证据与建议 |
| llm reconnect / compact | 重点不是「有没有锁」，而是能否锁住**真实砸体验**路径。口径：**auto compact 失败 → turn 终止不继续**；**手动 compact 失败 → 可重试**。subagent / 不可重试路径优先挖 |
| turn-reservation | **保留**为重要节点；挂在 `turn-lifecycle` 下（session 体系内），与 agent-loop / tool-pipeline 分层 |
| 构建路径前缀（REL-2） | 构建卫生，非用户功能面；默认不升产品形态题 |
| 直接修 | 仅当：确认 bug/明确 drift + 证据链齐 + 不改主张。否则先 issue。**ISSUE-012 webfetch** 优先核实后可修；其余先证据 |

## 交付物（每域）

路径：`.litecode/ops/artifacts/BATCH-3-<domain-slice>.md`

须含：

1. 节点对照表：key → 结论（一致 / 不能扩）→ 证据（文件+符号+测试或行为观测）
2. 正式 issue 列表（用 ops issue schema；可从 Batch 2 草稿升级/关闭/降级）
3. 待删/待改非知识 md 清单（仅本域撞车来源）
4. 互引补全：Batch 2 开放的 `相关：` 本域侧可自补（不改主张）
5. 阻断声明：有/无；有则立刻私信 GM

## 范围切片（负责人）

| 负责人 | 切片 | artifact 建议名 |
|---|---|---|
| Session | harness-core session 节点 + client-ux protocol（serve/ask registry） | BATCH-3-harness-core-session.md, BATCH-3-client-ux-protocol.md |
| Runtime | harness-core runtime + model-config | BATCH-3-harness-core-runtime.md, BATCH-3-model-config.md |
| 上下文 | harness-core context | BATCH-3-harness-core-context.md |
| IDE 基座 | ide-substrate | BATCH-3-ide-substrate.md |
| Tools | tools-permission + knowledge | BATCH-3-tools-permission.md, BATCH-3-knowledge.md |
| 前端 | client-ux web + release-desktop | BATCH-3-client-ux-web.md, BATCH-3-release-desktop.md |

## 门禁

- 跑本域相关 `kb_check`；全库 error 保持 0
- 死亡清单：辩证处理；删/松须 issue 记理由，禁止静默为绿
- enabled 节点：可抛光措辞/补证据/下挂，**不改主张含义**；contracts 若意图与实现冲突 → issue 先于改节点
