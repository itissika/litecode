目标：测试瘦身（删过期 gate/重复/文本断言，不留 ignore）+ 修 ptc 栈溢出可捕获；lib+tests/ 全量绿。
步骤：
0 G4 先修集成测编译（code_surface_phase* 补 ToolCallOrigin.deadline，测试侧），跑全量 cargo test 入 artifacts（删前基线，含测数）。
1 G1 PTC（engine.rs + ptc/tests.rs + tests/ptc_contract_harness.rs）：原生 max stack = stack_bytes + headroom 使 watch 先封口；两 stack 测绿+try/catch 不可捕获测；删 ptc_contract_harness。
2 G3 Tools（src/tools/phase2_gate.rs、phase3_gate.rs、read/grep/glob/write/edit/apply_edits/code.rs 测、litecode_workspace.rs）：独有断言并入模块后删 gate；MCP 冲突改用 run；删快照已覆盖的描述断言。
3 G2 Tool 框架（src/tool/deletion_gate.rs、contract_matrix.rs、phase1_gate.rs、mod.rs、code_manifest.rs 测）：删整文件（独有断言并入）；删 PRELUDE 子串断言（快照为唯一守卫）。
4 G4 集成（tests/code_surface_phase* → tests/code_surface.rs，death_list 按 Q2）。
文件互不重叠；cargo 串行 G1→G3→G2→G4→全量，free≥2GiB，-j 2。tests/ 全量预估：编译 15–20 min（-j 2，65 个 bin 链接），运行 3–5 min。
交付：coverage-map.md（核心流程→测试，删前/删后对照）、deletions.md（逐条理由+替代覆盖）、cargo 日志（删前/删后测数）、dig-survey.md。
验收：stack 两测绿且 try/catch 吞不掉；全量 cargo test（lib+tests/ 实际运行）全绿；核心覆盖不降；除 stack 外不改产品码；push feat/tool-contract-ptc，不合 main。
Q1-Q5 见 dig-survey §7。
