```node
node : tool-identity
status : pending
domain : tools-permission
summary : 工具身份是 name，一次调用身份是 call_id；name 可以重名，call_id 只负责结果配对。
x : 3960
y : 396
w : 362
h : 421
```

**主张：** 工具在管线里的身份是 `Tool::name()`，一次调用的身份是 `call_id`；两者不能互相顶替——工具重名时 call_id 只保证结果配对，不回答“到底跑了哪一份”。

- 身份键是 name、schema 是接口：schema 决定参数如何校验与呈现，不参与身份；name 由装配产生，不保证唯一 [@ file="src/tool/trait_.rs" lines="32-35"]。
- 名字是装配产物而非全局唯一保证：MCP 的 agent 面名由装配生成 `mcp_{server_id}_{原始名}`（原始名只用于 `tools/call`，前缀拼接不保证唯一）；装配不去重、运行期 first-match，重名时排后的一份被广告却不可达 [@ file="src/tools/mcp_tool.rs" symbol="impl McpTool › fn new"]、[@ file="src/tool/registry.rs" symbol="fn instantiate_tool"]。
- 另一层键是目录/绑定 id（core id、custom 名、`mcp_<server_id>`），只管装配与 agent 绑定 [@ file="src/config/global_db/tools.rs" symbol="fn mcp_catalog_id"]。
- 运行期按模型给的 name 找实现（查找入口见 [@ key="tool-request-entry"]）；call_id 配对与按调用顺序落地见 [@ key="tool-result-delivery"]。
- 模型看到的是 name/description/schema 投影，按装配顺序原样发出（投影入口见 [@ key="tool-request-entry"]）。

父节点：[@ key="tool-implementation-contract"]。
相关：[@ key="tool-implementation-contract"]、[@ key="tool-adapters"]、[@ key="tool-request-entry"]、[@ key="tool-result-delivery"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- name vs call_id —— [@ file="src/tool/trait_.rs"]；MCP 前缀装配 [@ file="src/tools/mcp_tool.rs" symbol="impl McpTool › fn new"]、[@ file="src/tool/registry.rs" symbol="fn instantiate_tool"]。
- 目录 id `mcp_<server>` —— [@ file="src/config/global_db/tools.rs" symbol="fn mcp_catalog_id"]。
