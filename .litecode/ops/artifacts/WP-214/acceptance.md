# WP-214 acceptance

## Tip / scope
- Branch `feat/tool-contract-ptc`（未合 main）。
- Dig tip：`f49e6fb`（ops 批准）；产品基线 brief：`2735592`。本包**零产品代码 diff**；**未跑 cargo**（调研包）。
- Deliverable：`artifacts/WP-214/report.md`。

## Done-when（对照 brief）
| 项 | 状态 |
|---|---|
| 手动 compact 后可点击点是否仍出现 | ✅ 答：仅 summary→`compacted` 时应出现；prune-only 设计上不出现 |
| 自动 compact 后是否仍出现 | ✅ 答：与手动同规则（共用 `compact_transcript`） |
| 断点归类（FE / 数据结构 / 设计） | ✅ 主因：**设计（prune-only）**；非 FE bug、非 kind 错位 |
| FE 断点则 handoff frontend_mid | ✅ **不 handoff**（主因非 FE）；报告 §8 写明 |
| 附录 Andy 复现步骤 | ✅ report 附录 A |
| prune-only 专节 | ✅ report §6 |
| DATA.md vs FE / surface_op:None | ✅ report §4.3、§5 |

## Protected / suite
- 无代码改动；无保护集变更；无 cargo。

## Leftovers for GM
- `board.json` / `log.jsonl` / `tool_traces.jsonl` / `board.md` / `icon.ico` 混脏 — **未**提交，留给 GM。
- 可选产品跟进（另包）：prune-only 静态切痕 / toast 文案；DATA.md 与 Popover 对齐。
