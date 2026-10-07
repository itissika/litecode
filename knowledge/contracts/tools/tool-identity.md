```node
node : tool-identity
status : pending
summary : 工具身份是 name，一次调用身份是 call_id；name 可以重名，call_id 只负责结果配对。
x : 3960
y : 396
w : 362
h : 421
```

**主张：** 工具在管线里的身份是 `Tool::name()`，一次调用的身份是 `call_id`；两者不能互相顶替——工具重名时 call_id 只保证结果配对，不回答“到底跑了哪一份”。

- 身份键是 name、schema 是接口：schema 决定参数如何校验与呈现，不参与身份；name 由装配产生，不保证唯一 [@ file="src/tool/trait_.rs" lines="32-35"]。
- 名字是装配产物而非全局唯一保证：MCP 工具以服务器原始名进入数组，可与内置/Custom 同名，装配不去重（见问题节点）[@ file="src/tool/registry.rs" symbol="fn instantiate_tool" lines="83-120"]。
- 另一层键是目录/绑定 id（core id、custom 名、`mcp_<server_id>`），只管装配与 agent 绑定 [@ file="src/config/global_db/tools.rs" symbol="fn mcp_catalog_id"]。
- 运行期按模型给的 name 找实现 [@ file="src/tool/executor.rs" symbol="fn run_tool" lines="303-308"]；call_id 把结果配回本次调用并按调用顺序落地 [@ file="src/tool/executor.rs" symbol="fn outputs_from_tool_results" lines="454-483"]、[@ file="src/tool/pipeline.rs" lines="196-203"]。
- 模型看到的是 name/description/schema 投影，按装配顺序原样发出 [@ file="src/runtime/context.rs" symbol="impl RuntimeContext › fn tool_defs"]。

相关：[@ key="tool-implementation-contract"]、[@ key="tool-adapters"]、[@ key="tool-mcp-name-collision-issue"]。
