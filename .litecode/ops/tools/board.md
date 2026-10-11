# LiteCode Ops Tools Board

> 聚合面板：最近更新 · 使用教程 · 运行时版本感知
> 每次工具输出第一行均为版本头：`LITECODE_OPS tools_version=N tool=... tool_version=...`

## 当前版本

- `tools_version`: **5**
- registry: `.litecode/ops/tools/registry.json`
- 查询: `node .litecode/ops/tools/tools_version.mjs`

## 最近更新

| 时间 | 变更 |
|---|---|
| 2026-10-11 | v5：工具一览补 emit/ack；ops.board 版本对齐；emit/ack 补 stdout_header（工具集管家）|
| 2026-10-11 | v4：事件总线 emit/ack；工具维护权移交工具集管家；GM 仅 tick 兜底
| 2026-10-11 | v3：stdout 首行版本头；本 board；team-ops bot 已建且未发首条 |
| 2026-10-11 | v2：validate 允许人类审查员 andy |
| 2026-10-11 | v1：gate / tick / validate / append_log |

## 使用教程（强制）

1. 调用前感知版本：读输出首行或跑 `tools_version.mjs`
2. 推进 WP：`wp_gate.mjs --id WP-xxx --to <status>`；done 需产物 + tool_traces
3. 审计：`append_log.mjs --kind --source --summary [--wp]`
4. 每轮：`tick.mjs`
5. 热更：改完 `tools_version++`，并在本表追加一行；破坏性变更用新 tool id

## 工具一览

| id | 命令 |
|---|---|
| ops.tools_version | `node .litecode/ops/tools/tools_version.mjs` |
| ops.validate_board | `node .litecode/ops/tools/validate_all.mjs [--wp WP-xxx]` |
| ops.append_log | `node .litecode/ops/tools/append_log.mjs --kind --source --summary [--evidence] [--wp]` |
| ops.wp_gate | `node .litecode/ops/tools/wp_gate.mjs --id WP-xxx --to <status>` |
| ops.tick | `node .litecode/ops/tools/tick.mjs [--wp WP-xxx]` |
| ops.board | Read `.litecode/ops/tools/board.md` |
| ops.emit_event | `node .litecode/ops/tools/emit_event.mjs --type --source --summary [--wp]` |
| ops.ack_event | `node .litecode/ops/tools/ack_event.mjs --id E-... --by <agent_id> [--wp]` |

## 待发掘

- （空）使用中发现的缺项记在这里，由工具集管家热更

## 边界

- 仅 team-ops 维护；产品 bot / 群对本启动无感知，直至 Andy 群内统一启动