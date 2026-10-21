# WP-201 方案 — run_code 人看聚合 data（无前端）

**owner：** backend_mid  
**分支：** `feat/tool-contract-ptc`（不合 main）  
**基线：** 叠在 WP-200 硬清之后的工作树（`duration_ms` / 成功失败计数已齐）；提交时 tip 以收口为准  
**依据：** brief；与 WP-200 分工——200=模型 content；201=`ToolCallResult.data` 人看聚合  
**内存：** available ≥ 2GiB；中层串行（**等 WP-200 Tools 收口+commit 后再开本包 cargo**，避免同槽互抢）

---

## 1. 目标契约（人看 / FE；模型不依赖）

```ts
// ToolCallResult.data for run_code（成功跑起来后必有；跑不起来可省略或极简）
{
  succeeded: number;   // 已结算成功工具调用（同 WP-200 Head）
  failed: number;      // 已结算失败 / caught reject（同 WP-200）
  duration_ms: number; // BudgetClock 墙钟 ms
  by_tool: Record<string, { ok: number; fail: number }>;
  // 可选极简：
  ok?: boolean;           // 程序是否无 failure_kind（或等价）
  failure_kind?: string;  // 有过程失败时；勿堆 ledger 全文
  unsettled?: number;     // 若 >0；勿进 content 明细
}
```

- 数据源：现有 `RunReport` / 内部 `CallLedger` 汇总；**禁止**把 `by_tool` 或 ledger 行塞回模型 `content`。
- 跑不起来：`data` 省略或只带 `ok:false` + `failure_kind`；错因仍在 content head。
- **禁止**改 `web/`；acceptance 写「前端消费缺口」。

---

## 2. 分工

| 域 | 职责 |
|---|---|
| **Runtime** | `render_run_output`（或等价出口）填充稳定 `data`；单测证明形状与「content 无 by_tool 明细」；dig-runtime-impl |
| **Tools** | 本包**默认不动**教法（人看字段不进模型 PRELUDE）；若 contract_matrix/SDK 强制声明 data 形状则最小补一句「人看聚合，模型忽略」 |
| **mid** | 串在 WP-200 之后；交叉审；acceptance（含 FE 缺口：字段路径、示例 JSON、transcript/API 怎么读）；commit/push/deliver |

顺序：WP-200 deliver 前本包只交方案；批后 **WP-200 合入实验分支 tip 后再实现**。

---

## 3. 验收

1. 成功跑完的 `run_code` 出站带稳定 `data`；测可证。  
2. `content` 无按工具名计数明细；与 WP-200 三块视图不冲突。  
3. 无 `web/` 改动；`artifacts/WP-201/{acceptance,frontend-gap,diff-summary}.md`；cargo 绿；未合 main。

---

## 4. 拍板

| # | 议题 | 默认 |
|---|---|---|
| Q1 | data 形状按上（succeeded/failed/duration_ms/by_tool + 可选 ok/failure_kind/unsettled） | 采用 |
| Q2 | 计数口径与 WP-200 Head 一致（已结算；unsettled 另字段） | 是 |
| Q3 | 不改 web/；acceptance 写 FE 消费缺口 | 是 |
| Q4 | 实现排在 WP-200 tip 收口之后，避免 content/data 双轨回潮 | 是 |
| Q5 | 不合 main | 是 |

