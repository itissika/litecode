# Runbook — Luna + grep 排障（WP-183 / WP-188 Obs-2.3）

对齐 WP-180 §8。目标：整页刷新 / session idle 时留下可关联证据，**先分类再开 Fix**，禁止未分类改 codec。

## 0. 前置

1. 开 wire：`scripts/serve.sh --wire /tmp/lc-wire`（或设 `LITECODE_LLM_WIRE=/tmp/lc-wire`）。
2. 开审计日志：`RUST_LOG=litecode.tool_audit=info,litecode=info`（可再加 `llm` / `runtime` debug）。
3. 确认 diagnostics 目录会落在工作区 `.litecode/diagnostics/<session_id>/`。

## 1. 复现

1. 用 Luna（或等价）发一段正常文本。
2. 让模型走约 2 次工具调用。
3. 再触发一次 **grep** 类调用。
4. 若出现整页刷新 / session 变 idle：立刻进入 §2 收证。

## 2. 整页刷新时立即收集

| # | 证据 | 路径 / 命令示意 |
|---|------|-----------------|
| a | wire 目录 | `$LITECODE_LLM_WIRE/<sanitized_session_id>/`（`summary.jsonl` + 最新 `*.response.sse.jsonl`） |
| b | 进程是否仍在 | `ps` / 服务状态；若已死看是否有 exit |
| c | transcript 尾 | session 库 / UI 尾部约 20 行；是否 `turn_finished` |
| d | diagnostics | `.litecode/diagnostics/<sid>/`：`crash-*.log`、`tool-audit.jsonl` 尾、`last-turn.json`、`wire-note.txt` |
| e | 浏览器 console | 前端 ErrorBoundary / network / WS 断开 |

复制 wire 与 diagnostics 到 bug 附件目录（现 retention：`KEEP_REQUESTS=400`、`KEEP_SESSIONS=30`，见 `failure-retention.md`）。

## 3. 用审计回答「grep 卡在哪」

```bash
SID=...   # sanitized session id
rg '"tool":"grep"' .litecode/diagnostics/$SID/tool-audit.jsonl
```

期望相位顺序：`start` → `auth_ask` → `auth_done` → `exec_done`（或 `panic` / `cancel`）。

- 只有 `start` / 停在 `auth_ask`：卡在权限 Ask。
- 有 `auth_done` 无 `exec_done`：执行中崩或未返回。
- 有 `panic`：工具 / join 恐慌（对照 `crash-*.log`）。
- 有完整 `exec_done` 但整页刷新：偏 FE / 进程外问题（对照 wire + console）。

## 4. 分类后再开 Fix（WP-180 A/B/C）

| 类 | 含义 | 下一步 |
|----|------|--------|
| A | 工具/执行层（panic、auth、超时） | Fix 工具或权限桥（**非本 Obs 包**） |
| B | LLM/transport（GiveUp、TurnError、wire 异常） | 看 wire + TurnError 日志中的 `wire_dir`；**禁止未分类改 H1 codec** |
| C | FE / 连接（刷新、WS） | FE ErrorBoundary / 会话投影包 |

**禁止：** 未完成上表分类就改 `responses`/`chat` codec 解析。

## 5. Checklist（填 bug 时勾）

- [ ] `LITECODE_LLM_WIRE` 已开并附上 session 文件夹
- [ ] `tool-audit.jsonl` 最后若干行已附（含 grep）
- [ ] `last-turn.json` 已附
- [ ] 若有 `crash-*.log` 已附
- [ ] transcript 尾 / 是否 turn_finished
- [ ] 进程存活与否
- [ ] 浏览器 console 摘要
- [ ] 已标 A / B / C（或组合）后再开 Fix 包
