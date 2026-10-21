# WP-201 实现 — run_code 人看聚合 data 通路（无前端）

**分支：** `feat/tool-contract-ptc`（不合 main）  
**口径：** Andy+GM。纯后端小包；**禁止**改前端实现；**禁止**把按工具聚合塞进模型 `content`。

## 目标

为人 / 前端实测准备结构化出口：聚合调用计数等放在 `ToolCallResult.data`（或等价稳定出站字段），通路打通。模型继续只消费精简 `content`（WP-200 的 head+logs+result）。

## 建议契约（方案可微调，须说明）

```ts
// 仅人看 / FE；模型不依赖
data?: {
  succeeded: number;
  failed: number;
  duration_ms: number;
  by_tool: Record<string, { ok: number; fail: number }>; // 或等价清晰形状
  // 可选：ok: boolean / failure_kind — 勿堆回 ledger 全文
}
```

- 数据源：已有 `RunReport` / ledger 汇总即可，勿再造观测层进模型面。
- 跑不起来：`data` 可省略或极简；错因仍在 content head。
- **本包不做** `web/` ToolView；acceptance 写清「前端消费缺口」：字段路径、示例 JSON、如何从 transcript/API 读到，方便 Andy 本地写 FE 实测。

## 验收

1. 成功跑完的 `run_code` 出站带稳定 `data` 聚合；测例可证。
2. `content` 不出现按工具名计数明细；与 WP-200 模型面不冲突。
3. 无前端代码改动（或仅文档）；`artifacts/WP-201` 含缺口说明；cargo 绿；未合 main。

## 非目标

- 不做 FE RunCodeToolView。
- 不恢复全量 ledger 进模型视图。
- 不合 main。
