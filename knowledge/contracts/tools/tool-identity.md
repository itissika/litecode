```node
node : tool-identity
status : pending
summary : 工具身份是 name，一次调用身份是 call_id；name 可以重名，call_id 只负责结果配对。
x : 2394
y : 594
w : 362
h : 421
domain : tools-permission
```

**主张：** 工具在管线里的身份是 `Tool::name()`，一次调用的身份是 `call_id`；两者不能互相顶替——工具重名时 call_id 只保证结果配对，不回答“到底跑了哪一份”。

- 身份键是 name、schema 是接口：schema 决定参数如何校验与呈现，不参与身份；name 由装配产生，不保证唯一 [trait_.rs:32-35](src/tool/trait_.rs#L32-L35)。
- 名字是装配产物而非全局唯一保证：生产模型层只暴露 `run_code`，所有工具均挂在 `tools.*` 下；MCP 的能力卡与 `run_code` 命名空间为 `mcp_<server_id>`（调用形式 `tools.mcp_<server_id>.<tool>`，底层 `tools/call` 仍传原始名）；同一 server 原始名按名去重，重名后一份丢弃并 warn，跨能力同名仍由运行期 first-match 处理 [McpTool.new](src/tools/mcp_tool.rs#McpTool.new)、[mcp_members](src/tool/registry.rs#mcp_members)。
- 另一层键是目录/绑定 id（core id、custom 名、`mcp_<server_id>`），只管装配与 agent 绑定 [mcp_catalog_id](src/config/global_db/tools.rs#mcp_catalog_id)。
- 运行期按模型给的 name 找实现（查找入口见 [tool-request-entry](knowledge/contracts/tools/tool-request-entry.md)）；call_id 配对与按调用顺序落地见 [tool-result-delivery](knowledge/contracts/tools/tool-result-delivery.md)。
- 模型看到的是 name/description/schema 投影，按装配顺序原样发出（投影入口见 [tool-request-entry](knowledge/contracts/tools/tool-request-entry.md)）。

父节点：[tool-implementation-contract](knowledge/contracts/tools/tool-implementation-contract.md)。
相关：[tool-implementation-contract](knowledge/contracts/tools/tool-implementation-contract.md)、[tool-adapters](knowledge/contracts/tools/tool-adapters.md)、[tool-request-entry](knowledge/contracts/tools/tool-request-entry.md)、[tool-result-delivery](knowledge/contracts/tools/tool-result-delivery.md)。

依据：[trait_.rs](src/tool/trait_.rs)。
