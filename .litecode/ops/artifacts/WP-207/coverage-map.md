# WP-207 核心路径覆盖地图（删前 43007f4 → 删后 b81a42d）
计数来自 cargo-full-pre.txt / cargo-full-post.txt 实际运行的测试名（lib 按模块前缀；集成测按文件）。

| 核心流程 | 覆盖测试 | 删前 | 删后 | 说明 |
|---|---|---|---|---|
| run_code 管线 · ptc 运行时 | ptc::* | 57 | 58 | G1 栈溢出封口 +1（Runtime） |
| run_code 管线 · code 工具/渲染 | tools::code::* | 24 | 24 | |
| run_code 管线 · run/wait/kill | tools::run::* / tools::wait / tools::kill | 7/4/4 | 7/4/4 | |
| run 锁语义 | tools::run_lock_gate、tool::resource_gate | 7/8 | 7/8 | 非迁移 gate，未动 |
| 执行器 / code caller 契约 | tool::executor::* | 19 | 21 | +2 并入 phase1_gate |
| 工具契约 · read | tools::read::* | 19 | 26 | +7 并入 phase2_gate（分页/链路） |
| 工具契约 · grep | tools::grep::* | 24 | 30 | +6 并入 phase2_gate（message 五态+literal） |
| 工具契约 · glob | tools::glob::* | 19 | 19 | |
| 工具契约 · write | tools::write::* | 12 | 16 | +4 并入 phase3_gate（expected_hash） |
| 工具契约 · edit | tools::edit::* | 31 | 32 | +2 并入、−1 描述文本测 |
| 工具契约 · apply_edits | tools::apply_edits::* | 3 | 2 | −1 描述文本测（快照覆盖） |
| 工具 data/error 锁定 | tools::baseline::* | 2 | 2 | 替代 contract_matrix |
| SDK fixture 快照 | tool::sdk_snapshot::* | 2 | 2 | 唯一描述/PRELUDE 文本守卫 |
| LSP | lsp::* + tests/optional_lsp.rs | 114 + 25 | 114 + 25 | optional_lsp idle 测改写 |
| session 持久化 | session::* + tests/session_data_* 等 | 228 | 228 | |
| provider 流式 | llm::* | 192 | 192 | |
| code 面集成（审批/取消/锁/并发写） | tests/code_surface*.rs | 38（5 文件） | 34（1 文件） | 删 4 过时/重复，行为断言保留 |
| run_code 集成 | tests/code_tool_runtime.rs | 4 | 4 | 改写对齐契约 |
| 权限 floor | tests/permission_pipeline.rs | 16 | 16 | 2 条改用 run |
| 迁移 gate | tool::phase1_gate/contract_matrix/deletion_gate、tools::phase2/3_gate | 34 | 0 | 独有 21 条已并入上表模块 |
| 架构守卫 death_list | types::death_list_gate + tests/*_death_list.rs | 13 + 22 | 13 + 18 | 删无引入路径的 4 条 |

总计：lib 2226 → 2212；lib + tests 2739 → 2714（含 ignored 4）。删后全量 0 failed（EXIT 0，2m25s）。核心流程覆盖无下降。
