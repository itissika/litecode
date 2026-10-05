### `custom_tools.json`

**干什么。** 本工作区自定义工具（一条命令 + JSON Schema；可覆盖同名的全局项）。名字不能和内置工具撞名。没有 `env`；cwd 跟进程走。

**怎么改。** 设置页增删改；或按格式改 JSON。名字必须是 `[a-z][a-z0-9_]*`，且必须和 map 的键相同。

**格式。** `version` 为 `1`。`timeout` 单位秒，省略则为 120。直接写 `"timeout": 0` 会变成 0 秒超时（立刻失败）；不要写 0。Schema 类型字段是 `"type"`。

`rules` 可省略，也可以写成 `[]`。这时这个工具只有开/关，调用前允许。写了至少一条，就可以在 ALL 和 SAFE 之间切换。ALL 允许这次调用。SAFE 按顺序看规则，先匹配到的那条生效；都没匹配到，就允许。

一条规则有 `id`、`action`、`when`。`action` 是 `allow`、`ask`、`deny`。`when.kind` 只能是 `any`、`arg_equals`、`arg_glob`、`path_outside_workspace`、`bash_readonly_command`、`all_of`、`any_of`。其它情况都要拒绝时，最后加一条 `"when": { "kind": "any" }`，`"action": "deny"`。

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
      "timeout": 120,
      "rules": [
        {
          "id": "outside_workspace",
          "when": { "kind": "path_outside_workspace", "name": "path" },
          "action": "deny"
        }
      ]
    }
  }
}
```

**怎么生效。** 工作区没有 running session 时，这个文件会自动生效。告诉人即可。有 `rules` 时，Agents 里可以切 ALL / SAFE；改规则后，下一次调用就按新规则来。工作区同名条目整份覆盖；工作区这份没有规则时，不能切换 ALL / SAFE。子 agent 不能问人：SAFE 里结果是 `ask` 时，直接拒绝。

**怎么验证。** `litecode_workspace refresh custom_tools` 校验本文件（JSON 合法性、名字规则且与 map 键一致、是否与内置工具撞名、`command` 是否为空），并标出还没生效的差异。`timeout: 0` 会按文件里的值报告立刻失败，不是校验拒绝。`when.kind` 写错时，整个文件解析失败，refresh 会把它标成 broken。

**下一步。** 需要人类在设置页 **Agents** 里打开该工具（审阅开关，不要替人打开）。有规则时，打开之后还可以切 ALL / SAFE，同样不要替人切。未打开时 `litecode_workspace status` 会把它列成 off for you，并补一句：只有你这次真的需要它，才去找人打开；否则不要提。
