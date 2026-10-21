# WP-200 方案 — run_code 管线硬清冗余分层

**owner：** backend_mid  
**分支：** `feat/tool-contract-ptc`（不合 main）  
**基线：** HEAD `c688b0a`  
**依据：** brief；Andy+GM 定稿硬清；学 pi 宽容；现有 `src/tools/code/render.rs` 模型面  
**内存：** available ≥ 2GiB 即可 cargo（中层串行放行）

---

## 1. 目标模型视图（唯一）

默认 agent 面**只**三块，顺序固定：

```
Head
[Logs?]     # 脚本自己写的过程日志；没有整段不出现；成功工具调用不自动占位
[Result?]   # 脚本返回值按现有窗口策略尽量保留；失败尽量保已有 result/partial
```

### Head（不变量，一两行）

| 情形 | 内容 |
|---|---|
| **跑不起来**（syntax / 校验失败 / 建 runtime 失败 / 未真正 drive 起来） | 精准：哪一行（+列若有）+ 错因；**不**堆完整 stack / 不复述工具信封 |
| **跑起来了**（含 timeout/cancel/exception/uncaught ToolError 等过程失败） | **成功数 / 失败数 / 耗时**（一两行够用）；过程失败时 head 可带一行失败标签（如 timeout / uncaught ToolError code），**不**另开独立 failure 大段 |

成功静默：无独立 failure / diagnostics / ledger / error_details 段。  
失败可见：错因落在 head（及脚本自愿 logs / 保留的 result），不另堆观测层。

### Logs / Result

- Logs：仅 `console` 等脚本主动写出；空则整段省略。
- Result：按现有 `RUN_CODE_VIEW_MAX_BYTES` 窗尽量保；失败**不空抹**已产出 value/partial（学 pi）。
- 嵌套工具结果是脚本数据：默认不进主视图；模型要过程自己 `log`/`return`。
- 工具契约错误归工具自己的 `{ok,message}`（或 `ToolError`）；**禁止**在 run_code 再包一层同义复述。

---

## 2. 硬清（默认模型面一行不留）

| 砍（默认 agent 视图） | 现状位置 / 符号（示意） |
|---|---|
| 独立 **failure** 大段 | `render_failure`、FAILURE_CAP、failure 正文/stack 段 |
| **diagnostics** 层 | `render_diagnostics`、`diagnostics:` 段、DIAGNOSTICS_CAP |
| 全量成功+失败 **ledger** | `render_ledger` / `calls (N of M shown)`、LEDGER_CAP、逐 call 行 |
| **error_details** 复述段 | `render_error_details`、ERRORS_CAP（与工具 message 重复） |
| 与工具 ok+message / ToolError 同义的外层复述 | failure 段再写 `code: message`、ledger 行再挂 error 细节等 |
| 其它「兼容观测」分层 | 凡不进 head/logs/result 的默认模型面段 |

**内部通道：** host/测试/人看若仍需账本或诊断，可留内部结构与测通道；**不进**默认 `render_model_view`。能删的渲染路径/常量/测例期望一并删，**不双轨**（不保留「开关开旧面」）。

**测例必须改写：** `tests/run_code_model_view.rs`、`render.rs` 单测等凡断言 `calls (`、`diagnostics:`、独立 failure 段、`ledger_reads_sessions` 依赖模型面文本的，改为断言 head/logs/result 形态。session-read 标记若仍产品需要，迁入 head 一句或内部通道——**默认不**靠全量 ledger 文本探测。

---

## 3. 分工

| 域 | 职责 |
|---|---|
| **Runtime** | `RunReport`/`render_model_view` 收敛为 head+logs+result；硬删默认面 failure/diagnostics/ledger/error_details 渲染；补耗时进 head；失败保 partial；相关集成测改写；dig-runtime-impl |
| **Tools** | run_code description / SDK fixture / 教法：只描述三块视图；扫残留「ledger/diagnostics/failure 段」文案；dig-tools-teaching |
| **mid** | 串槽、交叉审、acceptance/diff-summary、commit/push、deliver |

顺序：Runtime 硬清模型面 → Tools 对齐文案/fixture → mid 收口。cargo 独占串行，门槛 ≥2GiB。

**非目标：** 不改 shell/edit/grep/glob/read 已定契约（除非为删 run_code 冗余所必需的最小触碰）；不合 main。

---

## 4. 验收

1. 默认模型视图仅 head +（自愿）logs + result；冗余分层清零（代码与测例可证：`rg` 无默认面 `diagnostics:` / `calls (` ledger 段等）。
2. 跑不起来：行+错清晰；跑起来：成功数/失败数/耗时；成功静默、失败可见。
3. 结果按窗尽量保；失败不空抹。
4. 相关 cargo 绿；`artifacts/WP-200/{acceptance,diff-summary,dig-*}.md`；push 实验分支；**未合 main**。

---

## 5. 拍板

| # | 议题 | 默认 |
|---|---|---|
| Q1 | 默认模型面只留 head + logs? + result?；硬清 failure/diagnostics/ledger/error_details 段 | 是 |
| Q2 | Head「跑起来」含成功数/失败数/**耗时**（现 head 无耗时则补） | 是 |
| Q3 | 失败保 partial result/logs；不空抹 | 是 |
| Q4 | 内部诊断/账本可留测试或人看通道，**不进**默认 agent 视图；不双轨开关 | 是 |
| Q5 | 不合 main | 是 |

