# WP-204 acceptance

**口径：** Andy + GM；只读审阅 agent 可见提示词文本面（五轴）。  
**分支：** `feat/tool-contract-ptc`（**不合 main**；**本包不改产品码**）  
**方案：** `plan.md`（Q1–Q5 全认）  
**基线：** `62ccead`（WP-203 产品 tip）  
**本 tip（artifacts）：** `8f37382`

| # | done_when | 结果 |
|---|-----------|------|
| 1 | `review.md` 覆盖五轴，证据可核，GM 可读 | **通过**。见 `review.md` + `dig-survey.md` |
| 2 | 有明确「无需动 / 建议开实现包」清单 | **通过**。P1：PRELUDE 可拔插闸；`session_search` 去 bash。P2：负向句瘦身 / `stale_cursor` 枚举 |
| 3 | 不改码；不合 main；精华可聊天给 Andy | **通过**。未改 `src/` / fixture / web/；未合 main |

## 总评（可转 Andy）

WP-196…203 硬清后正向契约在 description + PRELUDE/SDK **基本对齐、无活跃旧方言可调入口**；主要缺口：**PRELUDE 对可拔插工具无条件点名**，以及 **`session_search` 仍写 bash**。建议另开实现包；本包无需动码。

## Digs

- `dig-survey.md` / `review.md` / `brief.md` / `plan.md`

## 非目标 / 未入库

- 不合 main；不改产品码 / web/  
- 未 stage ops board/events/log  
- 建议实现包不在本包开工
