### `custom_tools.json`

**干什么。** 本工作区自定义工具（一条命令 + JSON Schema；可覆盖同名的全局项）。名字不能和内置工具撞名。没有 `env`；cwd 跟进程走。

**怎么改。** 设置页增删改；或按格式改 JSON。名字必须是 `[a-z][a-z0-9_]*`，且必须和 map 的键相同。

**格式。** `version` 为 `1`。`timeout` 单位秒，省略则为 120。直接写 `"timeout": 0` 会变成 0 秒超时（立刻失败）；不要写 0。Schema 类型字段是 `"type"`。

调用约定：参数以 JSON 写入命令的 **stdin**，读 stdout；exit 0 成功；exit 2 表示工具拒绝执行。stdout 若是带 `media` / `level` 的 JSON 对象，按 envelope 解析，否则当纯文本。

```json
{
  "version": 1,
  "tools": {
    "<name>": {
      "name": "<name>",
      "description": "",
      "schema": { "type": "object", "properties": {}, "required": [] },
      "command": "your-cmd",
      "args": [],
      "timeout": 120
    }
  }
}
```

**怎么生效。** 工作区没有 running session 时，这个文件会自动生效。告诉人即可。

**怎么验证。** `litecode_workspace refresh custom_tools` 校验本文件（JSON 合法性、名字规则且与 map 键一致、是否与内置工具撞名、`command` 是否为空），并标出还没生效的差异。`timeout: 0` 会按文件里的值报告立刻失败，不是校验拒绝。

**下一步。** 需要人类在设置页 **Agents** 里打开该工具（审阅开关，不要替人打开）。未打开时 `litecode_workspace status` 会把它列成 off for you，并补一句：只有你这次真的需要它，才去找人打开；否则不要提。
