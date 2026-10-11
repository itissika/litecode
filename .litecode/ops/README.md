# LiteCode Ops（本地，已 gitignore）

路径：`.litecode/ops/`（不进开源仓库）

| 文件 | 用途 |
|---|---|
| `board.schema.json` | 面板 JSON Schema（数据类型校验） |
| `board.json` | 当前域 / 人员 / 工作包 / 管线 |
| `log.jsonl` | append-only：怀疑、证实错误、决策、关键信息 |
| `artifacts/` | 工作包产物（结论+证据链） |
| `handoffs/` | 依赖交接说明（事实发现，不含指引） |

校验：`node .litecode/ops/validate.mjs`（需本机有 node）

规则摘要见总经理与 Andy 对齐的执行方式；enabled 知识节点不得擅改。

## 提交纪律（Andy，2026-10-11）

不要频繁提交推送。只有积累了大量变更，或即将进行高风险变更时，才提交并推送到独立分支（不碰 main）。
