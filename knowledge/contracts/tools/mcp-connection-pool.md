```node
node : mcp-connection-pool
status : pending
domain : tools-permission
summary : MCP 连接池：按 server 启停与握手；工具 schema 与 call 只经 hub，不另开旁路。
```

**准入：** A2/A4。MCP 进程生命周期（Stopped/…）与 `tools/list`、`tools/call` 都归 `McpConnectionPool`；agent 面工具只是带前缀的薄适配。池坏了会出现「目录里有 MCP、执行时无连接」或绕过握手直调。适配差异见 tool-adapters，此处只钉连接与调用出口。

依据：[@ file="src/mcp/pool.rs" symbol="impl McpConnectionPool › fn get_or_create"]、[@ file="src/mcp/pool.rs" symbol="impl McpConnectionPool › fn start"]、[@ file="src/mcp/pool.rs" symbol="impl McpConnectionPool › fn stop"]、[@ file="src/mcp/client.rs" symbol="impl McpStdioClient › fn call_tool"]、[@ file="src/tools/mcp_tool.rs" symbol="fn hub_call"]。
父节点：[@ key="tool-adapters"]。
相关：[@ key="tool-request-entry"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 启停与 get_or_create —— [@ file="src/mcp/pool.rs" symbol="impl McpConnectionPool › fn start"]、[@ file="src/mcp/pool.rs" symbol="impl McpConnectionPool › fn get_or_create"]、[@ file="src/mcp/pool.rs" symbol="impl McpConnectionPool › fn stop"]。
- call 经 hub —— [@ file="src/tools/mcp_tool.rs" symbol="fn hub_call"]、[@ file="src/mcp/client.rs" symbol="impl McpStdioClient › fn call_tool"]。
- 「配置存在 ≠ Warm」是刻意边界，已由 [@ key="tool-request-entry"] 写明；TP-B2-02 不升 issue。
