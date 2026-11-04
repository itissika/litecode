# NocoDB

Airtable 风格的本地数据库界面。LiteCode 把固定版本的 NocoDB 安装到托管目录，并在 loopback 启动。

- 数据保存在当前工作区的 `.litecode/apps/nocodb/`。
- 首次安装会下载 npm 包，之后启动很快。
- 卸载会删除应用目录，但保留工作区数据。
- 数据库和表格在应用自己的界面里创建。
- 要用 agent 读写这些表：NocoDB 自带 MCP 端点，但它的端口每次启动都会变。把客户端配置写成固定命令（端口由命令在每次请求时解析）：
  `litecode app-mcp nocodb --path /mcp/<endpoint> --header "xc-mcp-token: <token>"`
  不写这条配置就不会建立任何连接。
