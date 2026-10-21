# WP-201 Runtime 实现笔记 — run_code 人看聚合 data

> **作者：** Runtime（Grok Bot 执行子代理）  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支 / HEAD 基线：** `feat/tool-contract-ptc` @ `7dd9617`（实现**未** commit；工作树有改动）  
> **依据：** `brief.md`、`plan.md`；mid 放行（叠 WP-200 模型面硬清之后）  
> **硬约束遵守：** **201 cargo 绿**（见下）；**未 commit/push**；**未合 main**；**未改** `web/`；**未改** Tools 教法 / SDK fixture

---

## 1. 关闭/替换层（一句话）

成功跑起来的 `run_code` 出站 `ToolCallResult.data` 带稳定人看/FE 聚合；模型 `content` 仍只三块（WP-200 head+logs+result）。**禁止**把 `by_tool` / ledger 塞回 content。

---

## 2. 契约（人看 / FE；模型忽略）

### 2.1 程序已启动（含过程失败）

```json
{
  "succeeded": 1,
  "failed": 1,
  "duration_ms": 42,
  "by_tool": {
    "read": { "ok": 1, "fail": 0 },
    "write": { "ok": 0, "fail": 1 }
  },
  "ok": true
}
```

| 字段 | 口径 |
|---|---|
| `succeeded` / `failed` | **已结算**工具调用；与 WP-200 Head 同一套 `success_failure_counts`（caught reject=`error:*`→failed；unsettled / host-cancelled **不计**） |
| `duration_ms` | `Outcome.duration_ms`（BudgetClock 墙钟 ms） |
| `by_tool` | `CallLedger` 结算时累计的 per-tool `{ok,fail}`（省略行仍计入；未结算不计入） |
| `ok` | `failure_kind.is_none()`（程序无过程失败） |
| `failure_kind` | 有过程失败时：`syntax` / `exception` / `timeout` / `cancelled` / `resource_limit` / `output_limit` / `illegal_output` / `internal` |
| `unsettled` | 仅当 `>0` 时出现；**不**进 content 明细（Head 旁注仍由 WP-200 负责） |

### 2.2 跑不起来（syntax / setup）

极简，**无** succeeded / by_tool / duration_ms：

```json
{ "ok": false, "failure_kind": "syntax" }
```

错因仍在 content head（行/列 + snippet）。

### 2.3 示例（真实单测 `render_run_output_attaches_by_tool_data_without_leaking_into_content`）

```json
{
  "succeeded": 1,
  "failed": 1,
  "duration_ms": 42,
  "by_tool": {
    "read": { "ok": 1, "fail": 0 },
    "write": { "ok": 0, "fail": 1 }
  },
  "ok": true
}
```

对应 content 头：`run_code ok · 1 succeeded, 1 failed · 42ms`；content **无** `by_tool` / `calls (` / `@@run_code_data`。

---

## 3. 实现要点

| 位置 | 做了什么 |
|---|---|
| `CallLedger` | 新增 `by_tool: Mutex<BTreeMap<String,(ok,fail)>>`；`record` 首次结算时 `note_settled`（含省略未保留行） |
| `LedgerSnapshot` / `RunReport` | 透传 `by_tool` |
| `render::human_data` | 从 `RunReport` 组 JSON；`did_not_start`→极简 |
| `finish_run_result` | `render_model_view` → content；`human_data` → `result.data` |

数据源：现有 `RunReport` + ledger 结算汇总；**未**新造观测层进模型面。

---

## 4. 改动文件（Runtime）

- `src/tools/code.rs` — ledger by_tool、`finish_run_result`、outcome 单测  
- `src/tools/code/render.rs` — `human_data` + 单测  
- `.litecode/ops/artifacts/WP-201/dig-runtime-impl.md`（本文件）  
- `.litecode/ops/artifacts/WP-201/cargo-run-code-data.txt`

**未改：** `web/`、`src/tool/fixtures/run_code_sdk.*`、`code_manifest` PRELUDE、shell/edit/grep 等已定契约。

---

## 5. cargo（已跑）

```bash
LC_MIN_MEM_GIB=2 CARGO_BUILD_JOBS=2
# available ≈10.9Gi ≥2Gi；独占槽

cargo test --lib -- tools::code:: --test-threads=2
# EXIT 0 → 23 passed（原 18 + 本包 5）

cargo test --test run_code_model_view -- --test-threads=2
# EXIT 0 → 17 passed
```

合计 **40 passed**。tee → `artifacts/WP-201/cargo-run-code-data.txt`。

---

## 6. content 面确认

- 默认面仍 head +（自愿）logs +（自愿）result  
- 负向：content **无** `by_tool`、**无** `calls (`、**无** `@@run_code_data`  
- 与 WP-200 不冲突

---

## 7. 前端消费缺口（给 mid acceptance / Andy）

| 项 | 说明 |
|---|---|
| **字段路径** | 出站 `ToolCallResult.data`（transcript / API 上对应 run_code 工具结果的 structured `data`，**不是** `content` 文本） |
| **何时有全量** | 程序已启动（含 timeout/exception 等过程失败）→ 必有 `succeeded/failed/duration_ms/by_tool` |
| **何时极简** | syntax 等「跑不起来」→ 仅 `{ok:false, failure_kind}`；错因读 content head |
| **模型** | 只看 `content`；**不要**把 `data.by_tool` 拼进 prompt |
| **本包不做** | `web/` ToolView；FE 需自行从 transcript/API 读 `data` 渲染聚合（缺口留给 FE 实测） |
| **示例 JSON** | 见 §2.3 |

---

## 8. Tools SDK？

**本包默认不动教法 / SDK fixture（否，不需本包刷）。**  
`contract_matrix` 里仍写着旧文案 `calls/logs/result`——属 Tools/mid 后续对齐，**非**本 Runtime 包范围。

---

## 9. 非目标 / 未做

- **未 commit / 未 push / 未合 main**  
- 未改 `web/`  
- 未恢复 ledger 进模型视图  
- 未改 Tools PRELUDE / run_code_sdk fixture  
