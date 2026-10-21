# WP-204 方案 — agent 可见提示词文本面查漏补缺（只读）

**owner：** backend_mid  
**分支：** `feat/tool-contract-ptc`（只读；不合 main）  
**基线：** HEAD `62ccead`（WP-203 tip）  
**依据：** brief；Andy 2026-10-22；对照 WP-196…203 硬清后活跃面  
**性质：** **分析包**——产出 `review.md`；**本包不改码 / 不刷 fixture / 不 cargo**

---

## 1. 目标

审阅模型回合里能读到的**整面提示词文本**，按五轴出结论 + 证据；明确「无需动 / 建议开实现包」清单，供 GM 聊天给 Andy。

---

## 2. Dig 范围（只读；全进 review 附录）

| 层 | 路径 / 触点（示意；dig 时以实测为准） | 审什么 |
|---|---|---|
| **系统固定** | `src/context_pipeline/system.rs`；`src/config/global_db/builtin_prompts.rs`；runtime 拼装入口 | 跨工具不变量/工作流；是否点名或渗入单工具用法（轴 4/5） |
| **Code / PRELUDE / SDK** | `src/tool/code_manifest.rs`（PRELUDE）；`src/tool/fixtures/run_code_sdk.{txt,sha256}` | 与 WP-197…203 契约一致；软硬失败句；是否残留 Direct/`lsp`+action/`text` 定位 |
| **live descriptions** | `src/tools/descriptions/*.md`（现行全量） | 冗余/过期方言；工具间互渗；与实现不一致 |
| **卡注入 / 绑定文案** | optional/引擎卡、Settings 可拔插相关注入（registry / availability / permission presets 旁路文案）；LSP 卡 id=`lsp` 注入七工具等 | 未绑是否仍被系统面强推；卡文是否泄漏他域要求 |
| **对照基线** | WP-196…203 acceptance/plan + 现行 schema（registry） | 已硬清项是否仍出现在提示词 |

非 dig：`web/` UX、libs、合 main、重做 WP-193 架构总评。

---

## 3. 五轴方法（每轴：结论 → 严重度 → 证据摘录）

1. **冗余** — 同义反复、过期方言、与已硬清实现重复  
2. **精准** — 契约/坐标/软硬失败是否跟实现一致（snake LSP、file_path+1-based、ok:false vs ToolError、198.18 等）  
3. **自相矛盾** — 系统 vs 工具、PRELUDE vs description、工具 A vs B  
4. **域隔离（可拔插）** — 单工具用法是否渗系统面或其他工具；未绑定是否被强推  
5. **系统固定稳** — 固定层只谈不变量/工作流，不渗单工具要求  

报告形态（brief）：一句话总评 → 五轴可读段（严重度大→小）→ 附录路径/摘录 → **无需动 / 建议后续实现包** 清单。

---

## 4. 分工

| 角色 | 职责 |
|---|---|
| **mid** | 主导 dig + 写 `review.md`；交叉对照 WP-196…203；fill/submit/deliver |
| **Tools** | 按需答疑 PRELUDE/descriptions/SDK 投影（不主动改码） |
| **Runtime** | 按需答疑实现与提示词是否漂移（不主动改码） |

无重型 cargo；无串槽冲突。

---

## 5. 验收

1. `artifacts/WP-204/review.md` 覆盖五轴，证据可核。  
2. 有明确「无需动 / 建议开实现包」清单。  
3. 未改产品码；未合 main；GM 可读后可 close。

---

## 6. 拍板

| # | 议题 | 默认 |
|---|---|---|
| Q1 | 本包只产出 review；发现的改码项一律建议后续包，不本包改 | **是** |
| Q2 | dig 覆盖上表四层 + 对照 WP-196…203；不扩到 web/ UX | **是** |
| Q3 | 严重度排序：未定产品行为/系统渗单工具 > 矛盾/过时方言 > 冗余措辞 | **是** |
| Q4 | LSP 卡 stamp / contract_matrix 旧断言若仍仅测侧，提示词面无则记「另排测对齐」不扩本包 | **是** |
| Q5 | 不合 main | **是** |

