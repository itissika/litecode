```node
node : engine-lifecycle
status : pending
summary : 引擎契约：desired、运行 state、索引状态与工具可见性各有含义；code/session 执行门解耦。
x : 756
y : 684
w : 398
h : 720
```

**人类与 agent 消费工作区服务；是否启用、是否运行、数据是否就绪不能压成一个布尔值。**

### 数据
- engines.json 保存 desired 与配置；owner 持有 Idle／Warming／Warm／Failed／Stopped 状态及 error。
- 索引 meta／work 描述语料是否需要构建或更新；usability 是面向消费者的派生视图，不是配置。

### 流转
配置意图 → owner reconcile → start／stop 与状态发布 → 人类状态面板、工具执行门；索引维护独立推进语料状态。

### 不变量
- reconcile 按 engines.json 的意图工作，工具绑定不拥有引擎生命周期；显式 refresh 可在 desired 已开启时请求启动／维护，不能反向开启配置。
- desired 不等于 Warm；Warm 也不保证 code 索引新鲜。失败保留详情，不以一个 Ready 标志掩盖。
- **LLM 列表不是 Warm 门闸**：workspace readiness 来自配置意图（LSP 还需配置 server），再与 agent binding 合取；运行未就绪时工具仍可能可见。
- code_search 执行门同时检查运行／维护与 code 索引状态；Wait 等待，Failed 报错，不拿旧索引假装成功。
- session 语义门只要求共享 worker Warm 且存在，不因 code corpus 的重建或 stale 而一起关闭；旧 session 索引可能损召回，命中仍需真源水合验证。
- HTTP 与 agent 消费同一服务 owner 的状态／数据；界面状态只能读投影，不能回写 desired 或伪造引擎生命周期。

依据：[@ file="src/engines/mod.rs" symbol="impl WorkspaceEngines › fn reconcile" label="生命周期 owner"]、[@ file="src/engines/mod.rs" symbol="impl WorkspaceEngines › fn code_search_call_gate" label="code 执行门"]、[@ file="src/engines/mod.rs" symbol="impl WorkspaceEngines › fn session_semantic_gate" label="session 执行门"]、[@ file="src/config/workspace.rs" symbol="fn workspace_readiness_from_engines" label="配置 readiness"]、[@ file="src/tool/availability.rs" symbol="fn should_include_in_llm_list" label="列表可见性"]。
验证：[@ file="src/engines/status_view.rs" label="可用性投影与测试"]。

上层：[@ id="config" label="config"]、[@ id="engine-design" label="engine 设计"]。
