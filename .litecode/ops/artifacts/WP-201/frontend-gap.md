# WP-201 前端消费缺口（无 web/ 改动）

> 本包只打通后端 `ToolCallResult.data` 聚合；**禁止**改 `web/`。供 Andy 本地写 FE 实测。  
> 细节与实现笔记见 `dig-runtime-impl.md` §2 / §7。

## 字段路径

| 层 | 路径 |
|---|---|
| 后端出口 | `ToolCallResult.data: Option<serde_json::Value>`（`src/types/result.rs`） |
| 组装 | `render::human_data(&RunReport)` → `finish_run_result` 写入 `result.data` |
| 模型面 | **只用** `ToolCallResult.content`（WP-200 head+logs+result）；**不要**把 `data.by_tool` 拼进 prompt |
| FE / 人看 | 从 transcript / API 上对应 **run_code 工具结果**的 structured `data` 读取（**不是** `content` 文本） |

本包**未**改 client protocol / web ToolView；FE 需确认现有 transcript/`item/tool_result`（或等价）是否已透传 `ToolCallResult.data`。若线尚未透传，那是 FE/协议消费缺口，不在本包范围。

## 何时有什么

| 场景 | `data` 形状 |
|---|---|
| 程序已启动（含 timeout/exception 等过程失败） | 全量：`succeeded` / `failed` / `duration_ms` / `by_tool` + `ok`；有过程失败时加 `failure_kind`；`unsettled>0` 时加 `unsettled` |
| 跑不起来（syntax / setup） | 极简：`{ "ok": false, "failure_kind": "syntax" }`（无 succeeded/by_tool/duration_ms）；错因读 **content head** |

## 示例 JSON（真实单测形状）

程序已启动、有成败：

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

对应 content 头（模型面）：`run_code ok · 1 succeeded, 1 failed · 42ms` —— **无** by_tool 明细。

跑不起来：

```json
{ "ok": false, "failure_kind": "syntax" }
```

## 计数口径（与 WP-200 Head 一致）

- `succeeded` / `failed`：已结算工具调用；caught reject（`error:*`）→ failed；unsettled / host-cancelled **不计**入成败。  
- `duration_ms`：`BudgetClock` 墙钟 ms（`Outcome.duration_ms`）。  
- `by_tool`：ledger 结算时累计的 per-tool `{ok,fail}`（省略行仍计入）。

## 本包明确不做

- `web/` 任何 ToolView / RunCode 面板实现  
- 把 `by_tool` 或 ledger 行塞回模型 `content`  
- Tools PRELUDE / SDK fixture 刷教法（人看字段不进模型面）
