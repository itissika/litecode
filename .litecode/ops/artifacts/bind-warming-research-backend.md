# Bind × Warming 只读调研（后端+前端代码阅读，af0963e，ops/team-framework）

范围：只读 rg/文件阅读；未改代码、未编译、未运行。

## 结论摘要
- 【已验证】没有任何代码因 Warming / 引擎不可用 / 重建 / 出错去改写库里的 bind（agent_tools.enabled）或 engines.json 的 desired。Andy 的“运行时改写期望”假设在当前代码里不成立。
- 【已验证】code_search 从未被默认绑定：seed 只绑 core_configurable_tools，其中不含 code_search（它是 optional builtin）。启用引擎（put_engines）也不会补绑定。前端把缺行显示为 Off。所以对 code_search 来说，“手点 bind”是第一次写入这一行，而不是恢复被改写的值。
- 【已验证】工具列表每个回合重建一次（每回合 build_runtime → 首步 build_tool_list），且回合开始时会 sync readiness；可见性门禁读的是实时引擎状态。所以 Warming→Warm 不需要专门信号，下一回合就会生效；但回合进行中不会刷新。
- 【推断】Andy 看到的现象最可能是“code_search 本来就没绑”（档 2/3），叠加“正在跑的回合在 Warming 时已冻结工具表”（档 4）。session_search 默认已绑定，所以只受后一种影响。

## 1) bind 存在哪里、语义；有没有运行时改写
【已验证】
- 存储：全局 SQLite 的 agent_tools 表（agent_id, tool_id, enabled, policy_json, path_mode, last_applied_preset, allowed_tools_json），写入入口为 src/config/global_db/mod.rs:516 upsert（ON CONFLICT 更新 enabled）。
- 引擎期望：工作区 engines.json 的 retrieval.desired，读 src/config/workspace.rs:284-292（workspace_engine_desired），写 :294 set_workspace_engine_desired；生产代码中唯一的写调用是 :340 enable_code_search_engine(true)，其余调用全在测试里（tools/code_search.rs:535、tools/session_search.rs:647、tool/availability.rs:404 等）。
- 语义：bind 是“期望”，可见性另行与实时状态做 AND：src/tool/availability.rs:171-195 should_include_in_llm_list = is_available && agent_tool_enabled(:144-150，读 binding.enabled) && （检索类工具额外要求）retrieval_tools_visible(:158-168 = desired && workspace_engines.is_searchable)。
- 改写排查：agent_tools 的 DELETE/UPDATE 只出现在 seed/migrate（seed.rs:17-19,308-320 只清理 hidden/subagent 角色的 primary-only/subagent 系列，355-357，388，961-966，1018）以及 settings 全量 replace_all（global_db/mod.rs:291，由设置写入触发）。engines/、tools/、serve/ 里没有任何因 Warming/NotWarm/rebuild/error 去写 agent_tools 或 desired 的代码。引擎出错只会改内存里的 EngineState（engines/mod.rs:1033-1075 Warming/Warm/Failed；:275-281 worker 退出时设为 Idle），不落盘到期望。
- 写 agent 的路径不会按可用性裁剪：settings_writer.rs:633-651 write_agent 只做 expand_binding_presets 和 normalize_agent_profile（tool/agent_bindings.rs:7-24，只按角色裁剪）。前端 AgentsSection.tsx:547-548 的注释写明不在目录里的绑定也保留；syncToolEnableSeries（api/settings.ts）只处理 bash/subagent 系列（schema.rs:99-105）。
- 默认绑定：global_db/tools.rs:3-16 core_configurable_tools 含 session_search，不含 code_search；:53-55 code_search/lsp 是 optional_builtin。seed.rs:107-113 和 :397-420（ensure_default_core_bindings）只补 core 工具。所以 default agent 默认没有 code_search 这一行。
- 前端缺行显示：AgentsSection.tsx:63 `tools[toolId] ?? { enabled: false, ... }`，缺行时显示为 Off。
- put_engines（serve/settings.rs:888-899）只写 engines.json 并 reload，不碰 agent_tools。

## 2) agent 工具列表何时、按什么条件计算
【已验证】
- 每回合：start_turn 先调 sync_workspace_tool_readiness（client_protocol/controller/session_lifecycle.rs:107；实现在 runtime/mod.rs:246-251，从磁盘重读 readiness 和 MCP/custom 定义），然后每个回合新建一个 AgentRuntime（runtime/mod.rs:464 build_runtime），在 run_with_turn 首步惰性调用 build_tool_list（runtime/mod.rs:935-996），回合内一直用 self.tool_pipeline。子代理回合同理（tools/subagent/turn.rs:24、launch.rs:56）。
- 条件：registry.rs:172-289 遍历 available_tools（availability.rs:69-142，code_search 只要 desired 就以 readiness=Ready 列入，见 workspace.rs:316-328，与 Warm 无关）→ should_include_in_llm_list。
- 实时门禁：is_searchable（engines/mod.rs:622-628）= state==Warm && code_search_call_gate==Ready（:694-710，Building/NeedsRebuild/Absent 时为 Wait）。session_search 共用同一个门禁（availability.rs:154-156、184），即 session_search 也要等 code_search 引擎 Warm。
- 信号：Warming→Warm 只是改内存 states（engines/mod.rs:1053-1058），不广播，也不重算正在进行的回合的工具表。没有按会话缓存，也不需要专门信号：下个回合会重新计算。
- 结论：Warming 期间开始的回合整轮都看不到检索工具，即使中途已经 Warm；下一回合才会出现。

## 3) 手点 bind 做了什么、为什么能修好
【已验证】前端 saveAgent（stores/settingsStore.ts:621-632，withTurnGuard，即回合进行中不能点）→ putAgent PUT /api/settings/agents/{id}（api/settings.ts:397-411）→ serve/settings.rs:298-323 put_agent → settings_writer.write_agent → reload_runtime_after_settings_write（serve/settings.rs:839-844：runtime.apply + sync_workspace_tool_readiness）。它不会刷新正在进行的回合，只会改库，让下一回合的 agent_tool_enabled 为 true。
【推断】它能修好有两种可能：(a) code_search 原本没有绑定行（默认就是这样），点击后第一次写入 enabled=true，这是真正的修复；(b) 只是巧合：点击时上一回合已经结束，下一回合本来就会看到 Warm。若 Andy 当时看到的卡片是 Off，就是 (a)。

## 4) 根因分类
- 期望被运行时改写：【已验证】不存在。
- 档 2 产品缺陷【已验证】：开启检索引擎（desired=true）不会让 agent 获得 code_search 绑定，默认 agent 也不预绑；用户必须到 Agents 页再手点一次。从用户角度看，“绑上就期望能用”没有达成，因为从来没有绑上。
- 档 3 架构设计问题【已验证+推断】：期望存了两处（engines.json 的 desired + agent_tools 的 bind），可见性还要再 AND 实时状态。session_search 是 core 且已绑定，却被 code_search 引擎的 Warm 状态门禁，期望维度混在一起。另外 available_tools 用 readiness=Ready 表示“desired”，名字和含义不符（workspace.rs:321-322），容易误读成运行时状态。
- 档 4 高频 bug【已验证机制，推断频率】：工具表在回合开始时冻结，Warming 期间开始的长回合（含子代理）整轮看不到检索工具；没有在 Warm 时刷新下一步工具表的路径。
- 档 1 待定产品行为：引擎没就绪时，检索工具应该隐藏，还是可见但调用时返回 “warming, retry”？（现在是隐藏，执行层另有 Wait 错误，engines/mod.rs:369-385。）
- 档 5 罕见边界【推断】：worker 退出时设为 Idle（engines/mod.rs:275-281），工具会一直隐藏，直到有人触发 reconcile；没有自动恢复信号。
- 总体：缺了刷新 + 设计问题两者都有，但不是 Andy 所说的“改写期望”那一种。

## 5) 参考与最小方向（只写方向）
- VS Code：贡献的 LM tools / 命令用 `when` 上下文键声明可用性，上下文键变化时重新求值；工具集合在每次请求时取当时的 enabled 集合，用户勾选的是期望，运行时不改写。Zed：settings 是期望，language server/工具的可用性来自运行时观察到的状态，状态变化后重新订阅、刷新 UI 和工具。
- 方向：
  1. 期望只存一份：开启检索引擎即视为期望可用，或至少在开启时给 primary agent 默认绑定 code_search；绑定只由人改。
  2. 可见性 = 期望 ∧ 实时就绪，并在每个 LLM 步（而不仅是回合开始）重新求值，或在 Warm 转换时发事件让下一步刷新工具表。
  3. session_search 的门禁与 code_search 引擎状态解耦，或明确写进产品规则。
  4. 未就绪时考虑“可见但返回 warming”的方案，交由档 1 决策。

## 已验证 / 推断 对照
- 已验证：无运行时改写 bind/desired；code_search 无默认绑定；前端缺行显示 Off；工具表每回合构建、回合内冻结；Warm 转换不发信号；bind 点击走 put_agent→apply+sync；session_search 受 code_search Warm 门禁。
- 推断：Andy 的实际触发是 (a) 未绑定还是 (b) 回合冻结；档 4/5 的频率。验证办法：查 Andy 机器上的 agent_tools 是否有 default/code_search 行及其写入时间。
