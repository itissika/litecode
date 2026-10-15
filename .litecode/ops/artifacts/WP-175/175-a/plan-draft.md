# WP-178 / 175-a 方案（provider）— 仅诊断 H1

输入：`.litecode/ops/artifacts/WP-175/code-map.md`、`plan.md`、`175-a-brief.md`  
口径：Andy/GM — WP-175 只锁定；与 WP-176 无关；禁止修复代码；不改 AskOutcome/权限桥。

## 目标
稳定取证 Responses SSE（Luna / gpt-6-luna）路径上 H1：usage details / unknown type → 反序列化失败 / GiveUp。产出 wire/fixture 证据与 file:line 边界；不交付修复。

## 步骤
1. 只读梳理 Responses SSE codec 与 stream_contract 相关路径（对照 code-map），标出 unknown type / usage / GiveUp 分支 file:line。
2. 能复现则抓 `LITECODE_LLM_WIRE`（或等价）SSE 片段；不能实网则用 SSE fixture / 最小只读回放脚本取证。
3. 判定 H1：实锤（证据链完整）或证伪（同类帧可过 / 失败在它处）；写清与 H2/H3 的分界建议（不建 175-b，只建议）。
4. 产物写入 `.litecode/ops/artifacts/WP-175/175-a/`（evidence log、片段、结论 md）。

## 不做
harden、改 codec 行为、AskOutcome、权限桥、WP-176、175-b 开工。

## 验收
brief 中 Done when 全部勾选；deliver 时附证据路径。
