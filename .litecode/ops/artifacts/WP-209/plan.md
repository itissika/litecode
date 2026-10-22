# WP-209 plan — run_code 模型视图整形 + token 预算
基线 90654c1（feat/tool-contract-ptc）；brief 中「与 WP-208 协调」作废（WP-208 已 close）。不合 main。

## Andy 已定（固定，不再讨论）
1. 字段一致的对象数组 → 表头 + 行；其余紧凑 JSON；字符串原样。
2. 前端/data 通道继续 pretty（human_data 不动，web 不改）。
3. result 预算按 token：8k。
4. 成功：logs 只留一行「logs 已省略（N 行）」（英文实现 `logs omitted (N lines)`，见 Q7）。
5. 失败：logs 完整带上，预算 2k tokens。
6. 超窗提示放 result 开头（仅超限时）。
7. 绝不裁剪旧结果。

## 定位（只读，@90654c1）
- 模型视图入口：src/tools/code.rs:953-971 finish_run_result → render::render_model_view；data=render::human_data（前端 pretty 来源，保持）。
- logs 汇入 RunReport：src/tools/code.rs:~1003 (run_report, outcome.logs)。
- 渲染/预算：src/tools/code/render.rs:18 RUN_CODE_VIEW_MAX_BYTES=65536；:22 LOGS_FLOOR=12KB；:79-112 render_model_view 字节循环；:179 assemble；:205-219 logs_budget；:423 render_logs；:500 render_result；:512-516 render_result_body（to_string_pretty → 改此处）。
- max_result_size=usize::MAX：src/tools/code.rs:1230（不变，渲染层唯一上限）。
- token 计数器已存在：src/session/estimate.rs:56 count_text_tokens（cl100k/tiktoken-rs），:62 truncate_text_tokens。
- PRELUDE：src/tool/code_manifest.rs:26（"about 64KB…Return a summary" 埋在段尾）。
- SDK fixture/快照：src/tool/fixtures/run_code_sdk.txt、src/tool/sdk_snapshot.rs。
- 前端 pretty：web/src/components/ToolContentView.tsx:22 JSON.stringify(...,2)（不改）。

## 精确格式（表格）
- 触发：数组长度 ≥3，元素均为对象，键集合相同（顺序无关，列序取首元素键序），所有值为原始类型（string/number/bool/null）。
- 顶层：`[N]{a,b,c}:` 换行后每行 `v1,v2,v3`。嵌在外层对象：外层紧凑 JSON 不再拆；仅当外层是对象且某键值满足触发条件时，渲染为 `key[N]{a,b,c}:` 块，其余键紧凑 JSON 单行（`{...}` 去掉该键）。只做一层，不递归。
- 单元格：number/bool 原样；null → 空；字符串若含 `,` `"` 换行 前后空白 或为空串/看起来像数字/bool/null → JSON 字符串转义带引号，否则裸写。
- 任一元素含嵌套对象/数组 → 整个数组回退紧凑 JSON（不做单元格 JSON）。

## 预算策略
- result 8k tok，失败 logs 2k tok；字节上限 64KB 保留作兜底。
- 开销：先按字节判断，len ≤ 8k*1（ASCII 下 1 字节≥… 取保守：bytes/4 < 预算*0.9 且无非 ASCII）直接放行；否则 count_text_tokens；超限用 truncate_text_tokens 前缀截断（UTF-8 安全）。
- 截断处尾部标记 `… [truncated: X of Y tok]`；开头加 `result Y tok exceeds 8k; truncated — return summary/counts/paths`。
- LOGS_FLOOR 与 logs 抢预算逻辑删除：成功无 logs 竞争；失败 logs 独立 2k。

## 分域
- Runtime（主）：render.rs 整形+预算+logs 规则+超限提示；code.rs 只改传参（若需）；单测在 render.rs。
- Tools：code_manifest.rs PRELUDE 前置指引（摘要优先 / 必须 return / 成功 logs 省略 / 8k·2k / 表格形态一行）；刷 run_code_sdk.txt sha + sdk_snapshot 快照。
- 我：measurements.md（read 200 行、grep 50 hits、glob 200 paths、LSP symbols、run 输出，前/后 token 与节省比）、acceptance、提交推送。
- 顺序：Runtime 先 → Tools 依 Runtime 最终文案刷 fixture（不并行编译）。

## cargo 顺序（同一时刻只一个，经我批准，free available ≥2GiB，-j 2）
1. Runtime：cargo test -j 2 --lib tools::code::render
2. Tools：cargo test -j 2 --lib sdk_snapshot code_manifest
3. 我：cargo test -j 2 --tests --no-run（防 WP-206 漏编译）
4. 我：最终 tip 全量 cargo test -j 2 → artifacts/WP-209/cargo-full.txt

## 新增测试（核心路径，入 protected）
- 同构数组→表格；键序不同仍判同构；缺字段/异构/嵌套/<3 行→紧凑 JSON；特殊字符转义；嵌在外层对象 key[N]{..}:。
- 非表格非字符串结果为紧凑 JSON（无换行缩进）；字符串原样。
- result 按 token 截断 ≤8k（含 CJK 用例）；字节兜底仍生效。
- 成功：logs 仅一行省略提示且 N 正确；失败：logs 保留且 ≤2k tok。
- 超限提示仅超限出现且位于 result 开头。
- human_data 仍 pretty 不变。
- PRELUDE/SDK 快照绿。

## 验收
brief 1–8；新增测试列入 protected；deliver 前 --tests --no-run 通过；最终 tip 全量绿日志入 artifacts；push 实验分支，不合 main；不碰 icon.ico / tool_traces.jsonl / web。

## Q
- Q1 同字段定义：键集合相同，顺序不计（列序取首元素）；缺字段即不同构→紧凑 JSON。推荐此。
- Q2 表格阈值：≥3 行。推荐。
- Q3 嵌套值：整组回退紧凑 JSON，不写单元格 JSON。推荐（格式简单、可预测）。
- Q4 tokenizer：复用 cl100k count_text_tokens；字节快速放行，接近上限才精确计数。推荐。
- Q5 超限提示/截断标记是否计入 8k：不计入（预算只算 result 正文，提示另加，最多 ~30 tok）。推荐。
- Q6 截断标记：前缀截断+尾部 `… [truncated: X of Y tok]` + 开头提示；表格按整行截断，不截半行。推荐。
- Q7 省略行文案：模型视图英文 `logs omitted (N lines)` 与现有英文标签一致；若 Andy 要中文原文则用「logs 已省略（N 行）」。推荐英文，请 GM 定。
- Q8 失败 logs 超 2k：保留尾部（错误通常在末尾），头部加 `… [N earlier lines omitted]`。推荐保尾。
- Q9 字节兜底：保留 64KB 总视图上限。推荐。
