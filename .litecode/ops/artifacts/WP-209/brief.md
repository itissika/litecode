# WP-209 brief — run_code 模型视图整形 + token 预算

- 类型：implement；owner：backend_mid；approver：gm
- 基线：`feat/tool-contract-ptc` 当前 tip（开包时 `c0147ac`）；**不合 main**
- 来源：GM 对 run_code 上下文膨胀的核对（Andy 已拍板以下各项）

## 与 WP-208 协调（必读）
WP-208（测试大幅删除）正在同一分支进行。WP-209 **二选一**，由 owner 决定并在 plan 中写明：
- (a) 等 WP-208 deliver 后再开始编码；或
- (b) 提前开工，但只触碰 render / code_manifest 相关文件（`src/tools/code/render.rs`、`src/tool/code_manifest.rs`、`src/tools/code.rs` 中 schema 文案、SDK fixture），并在提交前 rebase 到最新 tip，避免与 WP-208 的删测冲突。

## 现状（核对结论，HEAD 依据）
- 视口按**字节**封顶：`render.rs:18` `RUN_CODE_VIEW_MAX_BYTES = 65_536`；`LOGS_FLOOR = 12KB`（`:22`），`logs_budget` `.max(LOGS_FLOOR)`（`:206-219`）与 result 抢预算。
- 非字符串结果用 `to_string_pretty`（`render.rs:512-516`）。
- `max_result_size = usize::MAX`（`code.rs:1230`），渲染层是唯一上限。
- 已有精确 cl100k 估算器：`session/estimate.rs:55` `count_text_tokens`（tiktoken-rs），`:61` 按 token 截前缀 helper。
- CJK：仅保证 UTF-8 边界；64KB CJK ≈ 2–3 万+ token，字节上限对 CJK 欠保护。
- PRELUDE 已有 "Return a summary…"（`code_manifest.rs:26`）但埋在长段末尾、无数字。

## Scope（Andy 已定）
1. **渲染层整形（只改 pipeline/render，工具不变）**
   - **同构对象数组**（每个元素是对象、键集合相同、值均为原始类型；行数阈值如 ≥3）渲染为 TOON 风格表格：表头一次 + 每行一行。
   - 其余一律 **紧凑 JSON**（`serde_json::to_string`）。字符串结果原样输出。
   - 人类/前端 data 通道保持 pretty JSON 不变。
   - plan 必须给出**精确格式**：分隔符、含逗号/换行/引号的值如何转义、null/bool/number 表示、嵌套值（对象/数组）的回退规则（整体回退紧凑 JSON 或该单元格写 JSON）、表格嵌在外层对象中时如何呈现（如 `key[N]{a,b,c}:` 后接行）。
   - PRELUDE 加一行解释表格形态。
2. **按 token 预算**（复用 cl100k 估算器）
   - result：**8k tokens**。
   - logs：**成功时整体省略**，只留一行 `logs omitted (N lines)`；**失败时**最多 **2k tokens**。
   - 保留字节上限作为兜底；修复 CJK 欠保护（预算以 token 计）。
   - 注意估算开销：仅在接近上限时精确计数或分块计数，plan 说明策略。
3. **超限提示**：仅在超限时，于 result 开头加一行，如 `result 18.2k tok exceeds 8k; truncated — return summary/counts/paths`。
4. **PRELUDE / schema 前置指引**：把"返回摘要而非原始大块数据；想让模型看到的数据必须 return；成功时 logs 被省略"移到最前，并写明数字（8k / 2k）。同步刷新 SDK fixture sha + 快照测试。
5. **测量**：对每个工具典型调用（read 200 行、grep 50 hits、glob 200 paths、LSP symbols、run 输出等）在整形前/后测 result 的 token 成本，表格入 `artifacts/WP-209/measurements.md`。

## Out of scope（明确不做）
- 旧工具结果的老化/裁剪/替换为 stub —— **Andy：永不做**。
- 工具自身输出契约（data 形态）不改。

## 验收标准
1. 测试：表格整形（同构数组→表格）、回退（异构/嵌套/少于阈值→紧凑 JSON）、特殊字符转义。
2. 测试：token 预算（含 CJK 结果按 token 正确截断）、字节兜底仍生效。
3. 测试：成功时 logs 省略且出现 `logs omitted (N lines)`；失败时 logs 保留且 ≤2k tokens。
4. 测试：超限提示仅在超限时出现，且位于 result 开头。
5. PRELUDE/schema 文案更新，SDK fixture sha + 快照测试绿。
6. `measurements.md` 测量表（前/后 token、节省比例）。
7. 最终 tip 全量 `cargo test`（lib + tests/）绿，日志入 artifacts。
8. push 实验分支；**不合 main**。

## 约束
- 分支 `feat/tool-contract-ptc`；不合 main。
- 跑 cargo 前确认空闲内存 ≥ 2GiB；建议 `-j 2`。
- 与 WP-208 协调见上；不碰 `desktop/build/icon.ico`；不改 web/。
