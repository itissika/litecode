# Failure retention（WP-188 Obs-2.2）

## 现状（`src/llm/codec/wire_dump.rs`）

| 常数 | 值 | 含义 |
|------|-----|------|
| `KEEP_REQUESTS` | 400 | 单 session 文件夹内保留的最新请求对数量 |
| `KEEP_SESSIONS` | 30 | wire base 下保留的最新 session 文件夹数 |

未设 `LITECODE_LLM_WIRE` 时 capture 全 no-op。

## 本包策略（默认）

- **不改** `KEEP_*` 常数。
- TurnError / GiveUp 路径打结构化日志（`wire_dir` / `wire_rel` 或 `wire disabled`），并写 `diagnostics/<sid>/wire-note.txt`。
- 排障时：**立刻复制** `$LITECODE_LLM_WIRE/<sid>/` 尾部（`summary.jsonl` + 最新 request/response）到 bug 附件，避免被后续 GC 冲掉。

## 未实现（后续）

- `LITECODE_LLM_WIRE_FAIL_ONLY=1`（仅失败保留）— **本包不实现**，标后续。
