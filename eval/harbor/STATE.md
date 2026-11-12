# 测评实现：共享面板

这是两个 bot 的共享工作面板，不是文档：只记当前状态、口径和谁在做什么。架构师bot 负责口径与验收，实现bot 每次提交前更新第 1、3、4 节。

## 1. 当前状态

- 构建：由 `eval/harbor/bin/build-info.json` 的 `commit` 决定，不以记忆为准。
- 已生效：一次跑的 token/成本四个字段写进 Harbor 的 result.json。位置：job 在 `stats.n_input_tokens/n_cache_tokens/n_output_tokens/cost_usd`，trial 在 `agent_result.*`。**顶层没有这四个键**，读错键会得到 None。
- 最近实跑：ds3-litecode（3 题 × 5，deepseek-flash），14/15，prompt 4.44M，输出 176k，$0.138。
- 未生效：ATIF 轨迹 `agent/trajectory.json`（进行中）。

## 2. 口径（改这里要架构师bot 点头）

- 分母跟着数走；没有分母的数不印。
- 异常单列：harness 起不来、非零退出，不混进成功率，也不混进成本。
- 无数据写「无数据」，不写 0。
- 成本标来源：harness 自报，还是价格表推算。
- 同一张表里必须同模型、同思考档位。LiteCode 的 `thinking = "medium"` 就是 high 档，不构成差异。
- 数字只从产物读：job 的 `stats`、trial 的 `agent_result`、`agent/trajectory.json`。不手算，不凭印象。

## 3. 进行中

| 事项 | 负责 | 状态 |
|---|---|---|
| ATIF 轨迹（sessions.db → agent/trajectory.json） | 实现bot | 进行中 |
| 聚合报告 `eval/report.py` | 待排 | 方案已定 |
| pi / codex 冒烟 | 待排 | |
| 换有梯度的题集 | 待排 | |

## 4. 变更记录

- 2026-11-12 `74680d85` 推送；记账四字段实测有数（57111/55424/3401/0.00492 一次 1 题验收）。
- 2026-11-12 推翻一处误判：`3e305bda` 起记账就是对的，之前三次"还是 null"是读错键。
