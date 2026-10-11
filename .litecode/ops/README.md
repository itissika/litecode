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
