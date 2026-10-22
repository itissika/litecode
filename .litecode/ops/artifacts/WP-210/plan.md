# WP-210 plan — run_code 体验闭环
基线 e2a2e46（feat/tool-contract-ptc，不合 main）。不碰 desktop/build/icon.ico、tool_traces.jsonl。

## Andy 已定（固定，原样，不改范围）
A 失败保留部分结果（预算内附已完成调用+已收集结果摘要；头部区分「工具调用 N 成功/M 失败」与「程序结局 failed/ok」）。
B 运行时异常带 line/column + 简短栈（与语法错误同级）。
C 超预算结构化截断：数组截尾 `… N more`；对象逐字段降级；表格整行截；不在字符串/记号中间切；只留开头、无找回；提示仍在开头。
D 成功 logs 窄预算≈200 tok，超出丢弃标 `N lines omitted`；失败保留尾部≈2k；文档写「想看过程就省着用，只打关键日志」。
E 保留 agent 给的 key 顺序（贯通 QuickJS→Rust）；表格列按首行 key 序；人看 pretty 路径不受影响。
F run 自身修复（不动管线、不加 timeout_ms/background）：声明 shell 身份 + MSYS 路径转换提示；非 UTF-8(CP936) 转码；stdout/stderr 分字段，output=纯 stdout；文档 wait(id) 跨调用有效性按实现更正。
G 统一结果契约：每工具 bool ok=「活干好没」；read 缺文件/edit 未匹配/grep 参数错/参数校验失败 → ok:false 不抛；零结果 ok:true+message；仅程序级误用/JS 错误 throw；空时结构化 reason；message 独立维度。
H read 降噪：仅未读完时给 Showing/Continue；超长行截断指明行号；成功移除 dropped undefined。
I Code 内只读 ls/stat/exists；不加 rm/mv；核实 LSP 工具在 builtin_code_surface_tools，缺则补。
J websearch 支持 top_k，返回结构化 results[]。
K 清理知识库约 27 个失效节点引用。

## 定位（只读 @e2a2e46）与分域
| 项 | 位置 | 域 |
|---|---|---|
| A | src/tools/code/render.rs:472-660 render_header_block/失败分支；src/tools/code.rs:866-1050 render_run_output/RunReport 组装（by_tool 已有 ok/err 计数 :195-225） | Runtime |
| B | src/ptc/boundary.rs:595-625 Failure{stack}（运行时异常 stack 已取、location 仅语法错误）；render.rs:595-610 位置行 | Runtime |
| C | render.rs:145-215 budget_result/token_prefix（现按行前缀切）→ 改为对 Value 结构化降级；:275-360 render_result_body/render_table | Runtime |
| D | render.rs:123-135 logs 规则、:219-272 render_failure_logs | Runtime |
| E | src/ptc/boundary.rs:20,48,284 Json::Object 用 BTreeMap（根因）；render.rs:348 列序。注：Cargo.lock 中 serde_json 已带 indexmap（ureq/tree-sitter 特性统一已开 preserve_order），故只需改 boundary 容器 | Runtime |
| F | src/tools/run.rs、src/tools/command_job.rs:131-200 compose_output/stream_text、:37-42 schema；src/terminal/agent_jobs.rs:1142 from_utf8_lossy（转码点）；wait 文档 src/tools/wait.rs | Tools（run/command_job）；agent_jobs.rs 转码 Runtime 不碰，Tools 独占 |
| G | src/tools/read.rs:160-245（coded/throw 路径）、edit.rs、grep.rs、apply_edits.rs；参数校验入口 src/tools/code.rs:~120-150 嵌套调用分发 | Tools（各工具）＋Runtime（code.rs 分发 1 处，串行在 Runtime 之后） |
| H | read.rs:406-470 render_text_page；boundary.rs:98-105 dropped undefined + render.rs:77 | Tools(read.rs)；boundary/render 归 Runtime |
| I | src/ptc/engine.rs:587-700 install_*（新增只读原语）；src/tool/registry.rs:388-412 builtin_code_surface_tools（当前无 LSP 七工具→补） | Runtime(engine) / Tools(registry) |
| J | src/tools/websearch.rs:107-140 WebSearchData/schema、provider 解析 | Tools |
| K | knowledge/**（如 src/tools/bash.rs 2 处）、docs/knowledge-references.md | 上下文 |
| 文档/SDK | src/tool/code_manifest.rs:16 PRELUDE；src/tool/fixtures/run_code_sdk.txt；src/tool/sdk_snapshot.rs | Tools（最后） |

## 串并行
- 阶段1（并行，文件不相交）：Runtime［boundary.rs、render.rs、engine.rs、code.rs］A/B/C/D/E/H(boundary部分)/I(engine)；Tools［run.rs、command_job.rs、agent_jobs.rs 转码段、read.rs、edit.rs、grep.rs、apply_edits.rs、websearch.rs、registry.rs］F/G/H(read)/I(registry)/J；上下文［knowledge/、docs/］K。
- code.rs 只归 Runtime；G 的参数校验改动由 Runtime 在阶段1末顺带做。
- 阶段2（串行）：Tools 按最终文案改 PRELUDE/工具文档并刷 SDK fixture/快照。
- 阶段3：我写 examples.md（A–K before/after + cl100k）、acceptance.md、提交推送。

## cargo
一次只一个重任务，-j 2，每次前 free 确认 available ≥2GiB。顺序：各域单测(--lib 定向) → `cargo test --tests --no-run -j 2` → 最终 tip 全量 `cargo test -j 2` → cargo-full.txt。阶段1 三域编译排队，不并行跑。

## 测试（仅核心，加入 protected.txt）
新增（约 10 条）：
1 失败附部分结果+头部区分调用计数/程序结局(A)；2 运行时异常带 line/column/栈(B)；3 超预算数组 `… N more` 且结果为合法 JSON 前缀/不切字符串(C)；4 表格整行截(C)；5 成功 logs 200tok 上限+`N lines omitted`(D)；6 key 顺序贯通桥+表格列序(E)；7 run stdout/stderr 分字段+CP936 转码(F)；8 read 缺文件/edit 未匹配/参数校验失败 → ok:false 不抛(G，一条表驱动)；9 ls/stat/exists 只读且无 rm/mv(I)；10 websearch top_k + results[](J)。
预计需改断言的 protected 测试：render.rs 中 WP-209 新增的表格列字母序、logs omitted、超预算前缀截断测试；read.rs Continue/dropped undefined 相关；run.rs:315 output 含 [stderr]；websearch.rs:263 data 形状；SDK 快照测试。落地时逐条列入 acceptance，每条注明原因。

## Q 列表（附推荐）
Q1 失败部分结果预算：与成功同 8k 结果预算，摘要为每次已完成调用一行(tool, ok, 简短结果)，超出按 C 截。推荐：是。
Q2 头部措辞：`program: failed · tool calls: 5 ok / 1 failed`。推荐：是（英文，同 Q7 前例）。
Q3 栈深：最多 5 帧，去掉 libs.js 内部帧。推荐：是。
Q4 C 降级顺序：先截最长数组尾 → 再把深层对象值替换为 `{…N keys}` → 最后字符串截断带 `…(N chars)`。推荐：是。
Q5 D 成功 logs：保留开头还是结尾的 200 tok？推荐：开头（关键日志一般早于结果），标 `N lines omitted`。
Q6 E：boundary 改 IndexMap（serde_json 已开 preserve_order），human_data pretty 同步保序，不另加开关。推荐：是。
Q7 F 转码：先试 UTF-8，失败按平台 ANSI 代码页（Windows GetACP，CP936 用 encoding_rs）解码；非 Windows 仍 lossy。推荐：是，新增依赖 encoding_rs（若已在锁文件则复用）。
Q8 F shell 身份：结果 data 新增 `shell` 字段（如 "bash (MINGW64)"），MSYS 提示仅在参数匹配 `^/[a-zA-Z]$` 等时附在 message。推荐：是。
Q9 F 字段：`output`=stdout，新增 `stderr`；schema/SDK 同步，这是破坏性变更但仅 Code 面。推荐：是。
Q10 G reason 枚举：not_found / no_match / invalid_args / empty / permission / io。推荐：是，固定小枚举。
Q11 G 校验失败不抛：Code 面嵌套调用统一包装为 ok:false；非 Code 面（直连工具调用）行为不变。推荐：是。
Q12 I：ls 不递归，最多 500 条，返回 {name,type,size}；stat 返回 {type,size,mtime}；exists 返回 bool；路径限工作区内。推荐：是。
Q13 I：LSP 七工具补进 builtin_code_surface_tools（冷 stub 同 WP-203 网络工具）。推荐：是。
Q14 J：top_k 默认 5、上限 10；results[]={title,url,snippet}；保留 text 吗？推荐：去掉 text（省 token）。
Q15 K：「失效」以 knowledge 解析器 unresolved 结果为准，只改引用不改契约语义。推荐：是。
Q16 A–K 合一包还是拆子包？推荐：直接做（direct），分阶段并按域提交。

## 风险
- E 根因在 boundary BTreeMap，非 Cargo 特性；改容器影响所有 run_code 输出顺序与大量快照。
- F 转码新增依赖/平台分支，Linux CI 难测 CP936（用字节夹具单测）。
- G 改语义面广，若某调用方依赖 throw 会回归；--tests --no-run 必须覆盖。
- protected 测试需改断言数可能 >6，逐条报备。
- 工作区现有 icon.ico 未提交删除（他人），不碰。
