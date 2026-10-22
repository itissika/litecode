目标：测试总数 2714 → ≤1000（lib 2212→≤790，tests/ 502→≤200），默认删除，只守关键模块核心链路与引用了 commit/WP/ISSUE 的回归测；不改产品码，不加 #[ignore]。
规则：见 dig-survey §2（整类删 8 类；必须保留 5 类）。删前对每个候选 grep 回归引用，命中转人工。以删为主，不做表驱动改写（仅当一个文件只剩近似变体时留 1 条）。
分组（文件互不重叠）：
 D1 tools/ + tool/（src/tools/**、src/tool/**）→ 150 + 60
 D2 engines/（src/engines/**）→ 110
 D3 session/ + context_pipeline/ + runtime/（src/session/**、src/context_pipeline/**、src/runtime/**）→ 90+25+30
 D4 llm/ + provider_catalog/ + ptc/（src/llm/**、src/provider_catalog/**、src/ptc/**）→ 70+?+40
 D5 workspace/ config/ lsp/ terminal/ client_protocol/ permission/ 其他小模块 → 45+40+35+15+20+20+余
 D6 tests/（全部集成测 + tests/common；death_list 合并为 tests/architecture_guards.rs）→ ≤200
cargo：串行，每组改完跑该组模块过滤 + 编译（free≥2GiB，-j 2）；顺序 D1→D2→D3→D4→D5→D6→最终 tip 全量 `cargo test -j 2 --no-fail-fast`。
交付：coverage-map.md（每核心链路删前/删后测试名）、deletions.md（按类别计数 + 按文件删除数，保留回归测注引用）、cargo 删前/删后日志与按区计数。
验收：总数 ≤~1000；每条核心链路 ≥1 条有意义测试；最终 tip 全量全绿；sdk_snapshot 绿；无产品码改动；无新增 ignore；push feat/tool-contract-ptc，不合 main。
Q1–Q6 见 dig-survey §5。
