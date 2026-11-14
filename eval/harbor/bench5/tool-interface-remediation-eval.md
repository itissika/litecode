# LiteCode 工具接口整改后评测报告

- 生成时间：`2026-11-14T17:45:25+08:00`
- 评测：Harbor 0.25.0；当前 4 题；每题 LiteCode 1 trial；DeepSeek Flash
- 运行 tag：`litecode-remediation-r3`
- LiteCode eval build：commit `af6358bb348d`，dirty `true`，built_at `2026-11-14T09:17:58Z`
- 数据源：`~/eval/jobs/bench5/`；LiteCode cell 均为本次 r3 trial
- 对照：上一份全量 `eval/bench5-report.json` 的 LiteCode 4 格

## 1. 通过结果

| 任务 | 旧结果 | 新结果 | 旧请求 | 新请求 |
|---|---|---:|---:|---:|
| prove-plus-comm | 4/4 中的通过 | 通过 | 12 | 9 |
| polyglot-c-py | 4/4 中的通过 | 通过 | 11 | 28 |
| password-recovery | 4/4 中的通过 | 通过 | 19 | 11 |
| cancel-async-tasks | 4/4 中的通过 | 通过 | 21 | 9 |
| **合计** | **4/4** | **4/4** | **63** | **57** |

没有出现 pass 回退。题目级方差很大：polyglot 从 11 次请求涨到 28 次，不能把总 token 下降全部归因于接口整改。

## 2. 聚合指标：旧 vs 新

| 指标 | 旧 | 新 | 变化 |
|---|---:|---:|---:|
| 通过 | 4/4 | 4/4 | 0 |
| 请求数 | 63 | 57 | -9.5% |
| prompt token | 1,385,721 | 1,110,847 | -19.8% |
| output token | 63,722 | 65,975 | +3.5% |
| total token | 1,449,443 | 1,176,822 | -18.8% |
| 成本 | $0.050744 | $0.046332 | -8.7% |

按请求看 prompt：旧 21,995 / 请求，新 19,489 / 请求，下降 11.4%。

## 3. 逐题 token / 成本

| 任务 | 通过 | 请求 | prompt | output | total | 成本 | agent 时间 |
|---|---:|---:|---:|---:|---:|---:|---:|
| prove-plus-comm | 是 | 9 | 67,971 | 3,953 | 71,924 | $0.003084 | 26.0s |
| polyglot-c-py | 是 | 28 | 730,152 | 34,777 | 764,929 | $0.024116 | 222.9s |
| password-recovery | 是 | 11 | 124,892 | 4,618 | 129,510 | $0.004476 | 31.0s |
| cancel-async-tasks | 是 | 9 | 187,832 | 22,627 | 210,459 | $0.014656 | 117.5s |
| **合计** | **4/4** | **57** | **1,110,847** | **65,975** | **1,176,822** | **$0.046332** | **99.3s/题** |

## 4. 工具调用与字段观测

本次 4 个 trial 的 `tool-audit.jsonl` 全部存在：

- 117 个调用 span（53 个外层 `run_code` + 64 个嵌套调用）
- 468 条 audit 事件，坏行 0
- direct / code 两种 caller 都有记录

嵌套工具分布：

| 工具 | 调用数 |
|---|---:|
| `exec_command` | 42 |
| `write` | 8 |
| `read` | 8 |
| `apply_edits` | 3 |
| `write_stdin` | 2 |
| `grep` | 1 |
| `edit` / `glob` / web / LSP | 0 |

结果字段已收口，实际观测到的 data 字段只有：

- `exec_command`: `output`、`exit_code`、`session_id`
- `write`: `path`
- `read`: `lines`
- `apply_edits`: `results`
- `grep`: `hits`
- `run_code`: 外层聚合字段 `ok` / `failed` / `succeeded` / `by_tool` / `duration_ms`

`path_risk`、`diagnostics`、`truncated`、`clamped`、`has_more`、`total_lines`、`skipped_in_context`、`lines_capped`、`state` 不再出现在嵌套调用的 data 里。

## 5. 上浮提醒

本次 8 条提醒，全部在 `polyglot-c-py`：

- 7 个 run_code 输出带提醒，8 条提醒合计 624 字符
- read 分页 4 条
- clamp 1 条：`yield_time_ms 30000 used instead of 60000`
- live session 1 条：`session bg_9a8eb287 ...`
- grep no-match 1 条
- swallowed failure 1 条

提醒去重后 8 条文本均唯一；没有重复的 path risk / stderr / live session。

需要如实记录：r3 运行时 live session 文案仍是旧长句，在 120 字符上限下被截成 `session_id=b…`。评测后已把文案缩短为：

```text
session <id> is live; use tools.process.write_stdin
```

并用单元测试锁定长度和完整 `session_id`。最终二进制随后在 `2026-11-14T09:49:20Z` 重建；因为用户要求每题只跑一次，这一条文案修正没有重跑评测。

## 6. run_code 的 null

- 旧手工复盘：43 次 run_code 输出中 42 次以 `null` 结尾
- 新：53 次 run_code 输出中 0 次以 `null` 结尾

这是本次最明确的行为变化之一：未 return 时不再把 `undefined` 渲染成 `null`。

## 7. 接口变化检查

- `edit` 已从 SDK、eval profile 和代码工具面移除；只剩 `apply_edits`
- `apply_edits` 始终返回 `results[]`，单条也是数组
- `write` 输入只剩 `path` / `content`；`create_only`、`expected_hash` 已删
- `write` 结果只剩 `path`
- `read` 结果只剩 `lines`
- `glob` 结果只剩 `paths`
- `grep` 结果只剩 `hits`
- 生成的 code SDK 从 28 个工具变为 27 个；文本从 26,950 bytes 降到 19,774 bytes（-26.7%）
- `libs` 未动；eval profile 仍保留 `grep` / `glob` / `webfetch` / `websearch`

## 8. 结论与限制

1. 通过率保持 4/4，没有回退。
2. total token 下降 18.8%、成本下降 8.7%；但单题方差明显，polyglot 反而从 11 次请求升到 28 次，聚合数字不能当作纯因果。
3. 最稳定的收益是结构性的：字段面变薄、提醒与结果分离、audit 自动落盘、未 return 不再输出 null。
4. `glob` / `webfetch` / `websearch` / LSP 本次仍然 0 调用，不能用这 4 题判断它们的实际使用；它们继续保留在 eval profile。
5. 本次只跑 4 题 x LiteCode x 1 trial；没有为了调参多跑。

## 9. Trial 产物

| 任务 | trial | 产物 |
|---|---|---|
| prove-plus-comm | `prove-plus-comm__Y584SAX` | `bench5-prove-plus-comm-litecode-remediation-r3-fix` |
| polyglot-c-py | `polyglot-c-py__HSJzjnf` | `bench5-polyglot-c-py-litecode-remediation-r3-fix` |
| password-recovery | `password-recovery__4cvyX7h` | `bench5-password-recovery-litecode-remediation-r3-fix` |
| cancel-async-tasks | `cancel-async-tasks__WQk7vAK` | `bench5-cancel-async-tasks-litecode-remediation-r3-fix` |

每个 trial 的 `agent/litecode/diagnostics/<session>/tool-audit.jsonl`、`agent/trajectory.json`、`agent/litecode/sessions.db` 都已落盘。

## 10. 验证

- `cargo check` 通过。
- `cargo test --lib`：1097 passed、0 failed、2 ignored；另过滤 2 个本机环境依赖测试：
  - 本地 provider 子 agent 超时测试
  - 本地 HTTP 非事件流测试
- SDK `tsc` / generated contract check 通过。
- collector unit tests：12 passed。
