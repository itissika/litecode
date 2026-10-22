# WP-210 run_code 体验闭环 — brief

分支：`feat/tool-contract-ptc`（不合 main）。来源：Andy 本地 agent 实测报告，产品决策均已由 Andy 拍板（2026-10-22），**无开放问题，仅剩实现细节**。不得触碰 `desktop/build/icon.ico`。

## 已定范围

**A. 失败保留部分结果**：程序抛错或返回非法输出时，在预算内附上已完成工具调用及已收集结果的摘要，不再只剩一行错误。头部计数措辞须区分“工具调用 N 成功/M 失败”与“程序结局（failed/ok）”，避免误读为程序成功。

**B. 运行时异常带位置**：与语法错误同级，给出 line/column + 简短栈（QuickJS 原生可得）。

**C. 超预算结构化截断**：数组截尾并标 `… N more`；对象保留字段（逐字段降级）；表格按整行截；永不在 JSON 字符串/记号中间切。仍只留开头、无找回通道（设计如此）；超预算提示仍在结果开头。

**D. 成功日志车道**：成功时 logs 窄预算约 200 token，超出丢弃并标 `N lines omitted`；失败仍保留尾部约 2k。文档写明“想看过程就省着用，只打关键日志”。

**E. 保留 agent 给的 key 顺序**：开 serde_json `preserve_order`（或等价方案，贯通 QuickJS→Rust 桥）；表格列按首行对象 key 顺序。核实人看 UI（pretty JSON）路径不受影响。

**F. run 工具自身修复（不动管线，保持 run 简单，不加 timeout_ms/background）**：
- 结果与文档声明 shell 身份（如 Windows 上 bash (MINGW64)）；对被 MSYS 路径转换的参数（如 `/c`）给出提示；
- 非 UTF-8 输出（CP936）转码为 UTF-8；
- stdout / stderr 分字段，`output` = 纯 stdout；
- 文档中 `wait(id)` 跨调用有效性按实现更正。

**G. 统一结果契约**：每个工具返回 bool `ok`，语义唯一：“工具有没有把活干好”。
- read 缺文件 → ok:false（不抛）；edit 未匹配 → ok:false；grep 参数错 → ok:false；参数校验失败 → ok:false（不抛，批处理不被打崩）；
- 零结果 → ok:true + message；
- 仅程序级误用 / JS 错误才 throw；
- 为空时给结构化 `reason`；message 是独立维度（ok:true 也可带）。

**H. read 降噪**：仅在未读完时给 “Showing lines… Continue with”；超长行截断须指明哪几行。成功时移除 `dropped undefined` 提示。

**I. 只读 IO 原语**：Code 内新增只读 `ls` / `stat` / `exists`；**不加 rm/mv**（libs 只读是不变量，多 agent 共享）。核实 LSP 工具已暴露在 Code surface（WP-202 遗留 `builtin_code_surface_tools`），缺则补。

**J. websearch**：支持 `top_k`，返回结构化 `results[]`，削减 token。

**K. 知识库失效引用**：清理约 27 个失效节点引用（如 `src/tools/bash.rs`、已删除的 tests）。

## 验收标准
1. 全量 `cargo test` 绿（先查空闲内存 ≥2GiB，`-j 2`），产物 `cargo-full.txt`。
2. 新增核心测试仅针对核心行为，加入 `protected.txt`；精简，不堆测试量。
3. SDK fixture 与 prompt/工具文档同步更新。
4. artifacts 中每项（A–K）给出 before/after 示例（`examples.md`），附 token 对比（cl100k）如适用。
5. 提交 `acceptance.md` 自检清单。
