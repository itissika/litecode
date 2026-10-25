# WP-214 compact point 可点击性调研 — brief

分支：`feat/tool-contract-ptc` @ `2735592`（**不合 main**）。来源：Andy 实测——手动点 compact 后看不到可点击的 compact point；需判断是前端问题还是数据结构错位。

**本包只调研锁定问题，不改产品代码。** 交付是调研报告（含 file:line 证据），不是修复。

## 目标

弄清：前端的**手动**与**自动** compact 之后，是否仍应出现可点击的 compact point；若不应或未出现，断点在管线哪一段——**FE 渲染**还是 **session/agent 投影数据结构**。

## 背景与已知线索（供对照，不代替调研）

Andy 近期已合入一批 compaction / context 相关改动（复用主模型与 request 前缀、持久 checkpoint 投影、request meter、机械裁剪、system guidance、summary 验收、去掉旧路径、简化 trigger、用所选 context window 算 pressure）。他本机手动点 compact 后**看不到**可点击的 compact point。

仓库内已有相关符号（调研须沿链路核实，勿当结论）：

- Session 投影：`src/session/data/sqlite/session.rs` — `write_compact_pointers` / `refresh_compact_pointers_from_log`（约 861+）
- FE 标记与列表：`web/src/components/transcriptMarks.tsx`（文案含 `compaction point`）、`MessageList` / 相关 test、`ContextUsageRing` 的 `compact(sessionId)`、`turnStore` / `messageStore` 对 compact checkpoint / `surface_op` replace 的处理

## 调研范围（必须走完）

1. **触发源**：手动 compact（UI 点击）与自动 compact（pressure / trigger）各自发出什么事件 / RPC / session 写路径；二者是否共用同一套 pointer 刷新。
2. **持久与投影**：compact 完成后 `write_compact_pointers` / log 刷新写出哪些字段（seq、kept、锚点 id 等）；出站协议 / agent 快照 / transcript 投影里 compact point 以何形状出现（或是否已消失、改名、被新 checkpoint 语义替换）。
3. **前端消费**：`transcriptMarks` / MessageList 等依据哪些字段判定「可点击 compact point」；手动与自动路径是否同一判断；点击回调期望的 id/seq 与后端投影是否对齐。
4. **对照**：设计上「应出现」还是「本就不该再以旧形态出现」（例如被 checkpoint / surface_op 替换）；若应出现，标出第一处断裂（FE vs 数据 / 协议）。

## 非目标（硬边界）

- **不改**任何产品代码（Rust / web / desktop / 协议 schema 实现）。
- **不合 main**；不顺手修 bug；不扩 scope 做实现包。
- 若结论是 FE 断了：报告里写清证据与建议接手方（`frontend_mid` / `frontend`），**本包不直接改 FE**。
- 若结论是数据/投影断了：报告写清字段错位与建议接手域（`session` / `context` / `runtime`），本包仍不改码。

## Owner 与协作

- **owner=`backend_mid`**：统筹整条投影链路（session 指针 → 出站/快照 → FE 消费条件），先钉数据是否该出、是否已出。
- **FE 会诊**：UI 点击性与 `transcriptMarks` / MessageList 判定逻辑需对照前端代码；若断点确认在 web/desktop，在报告中明确 **handoff → `frontend_mid`**（可建议子包，但本 WP 仍只出报告）。
- approver=`gm`。

## 验收（done_when）

1. 报告明确回答：**手动 / 自动 compact 之后，按当前设计是否应出现可点击 compact point**（分别写清）。
2. 若未出现或不完整：标出管线断裂位置，并判定 **FE bug** / **数据结构或协议错位** / **设计已废弃旧点位（非 bug）** 三者之一（可组合，但主因唯一）。
3. 每条关键结论附 **file:line**（或协议字段路径 + 示例 JSON）；手动与自动至少各一条证据链。
4. 产物落在 `.litecode/ops/artifacts/WP-214/`（如 `report.md`）；**无产品代码 diff**；未合 main。

## 建议产物结构

- `report.md`：结论摘要 → 手动链路 → 自动链路 → 对照表 → 断裂点 → 建议下一步（仍不实现）。
- 可选：最小复现步骤（Andy 本机操作路径），便于验收报告。
