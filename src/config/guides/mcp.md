### `mcp.json`

**干什么。** 本工作区 MCP 服务器（可覆盖同名的全局项）。目前只支持 **stdio**（`command` 必填）。

**怎么改。** 设置页增删改；或按格式改 JSON。id 必须是 `[a-z][a-z0-9_]*`，不能和内置工具撞名。设置页会校验；直接改文件绕过校验的项，界面和运行时可能对不上。

**格式。** `version` 为 `1`。`timeout` 单位秒，省略或 `0` 则为 60。`transport` 默认 stdio。子进程 cwd 是工作区根。

```json
{
  "version": 1,
  "servers": {
    "<id>": {
      "command": "npx",
      "args": [],
      "env": {},
      "transport": { "type": "stdio" },
      "timeout": 60
    }
  }
}
```

**怎么生效。** 工作区没有 running session 时，这个文件会自动生效。告诉人即可。已经给这个 agent 打开的服务器，会在那时按新定义重新拉起；人也可以在设置页启动。

**怎么验证。** `litecode_workspace refresh mcp` 校验本文件（JSON 合法性、id 规则 `[a-z][a-z0-9_]*`、是否与内置工具撞名、`command` 是否为空），并标出还没生效的差异。它不拉起进程。

**下一步。** 写好定义还不够：需要人类在设置页 **Agents** 里打开 `mcp_<id>`（审阅开关，不要替人打开）。未打开时 `litecode_workspace status` 会把它列成 off for you，并补一句：只有你这次真的需要它，才去找人打开；否则不要提。
