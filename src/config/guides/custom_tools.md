### `custom_tools.json`

**干什么。** 本工作区自定义工具（一条命令 + JSON Schema；可覆盖同名的全局项）。名字不能和内置工具撞名。子进程 cwd 为工作区根；环境会注入 `LITECODE_WORKSPACE` / `LITECODE_CALL_ID` / `LITECODE_SESSION_ID` / `LITECODE_TOOL_NAME`（后台任务还有 `LITECODE_JOB_ID`）。**只有这些**——定义里没有自定义 `env` 字段，也不会注入 `PYTHONIOENCODING` 等。

**怎么改。** 设置页增删改；或按格式改 JSON。名字必须是 `[a-z][a-z0-9_]*`，且必须和 map 的键相同。

**格式。** `version` 为 `1`。`timeout` 单位秒，省略则为 120。不要写 `timeout: 0`（手写文件会立刻超时；设置页会改成 120）。Schema 类型字段是 `"type"`。

`rules` 可省略，也可以写成 `[]`。这时这个工具只有开/关，调用前允许。写了至少一条，就可以在 ALL 和 SAFE 之间切换。ALL 允许这次调用。SAFE 按顺序看规则，先匹配到的那条生效；都没匹配到，就允许。

一条规则有 `id`、`action`、`when`。`action` 是 `allow`、`ask`、`deny`。`when.kind` 只能是 `any`、`arg_equals`、`arg_glob`、`path_outside_workspace`、`bash_readonly_command`、`all_of`、`any_of`。其它情况都要拒绝时，最后加一条 `"when": { "kind": "any" }`，`"action": "deny"`。

**调用约定。** 参数以 JSON 对象写入命令的 **stdin**（`run_in_background` 不会进 stdin；`validate_custom` 同样会剥掉），读 stdout；exit 0 成功；exit 2 表示工具拒绝执行。取消或超时会 best-effort 杀掉进程树。

**stdout envelope（opt-in）。** 仅在 **exit 0** 时解析。stdout 必须是 JSON **对象**，且至少带 `media` 和/或 `level`；否则整段当纯文本。字段名以代码为准：

- `content`（string，可选）— 文本正文；**不是** `text` / `message`。缺省当 `""`。
- `level`（string，可选）— 只能是 `ok` / `warning` / `error`（大小写不敏感）。省略当 `ok`。未知值 → 硬失败（FAIL）。`error` 会让这次工具结果以 Error/FAIL 表面呈现（即使进程 exit 0）。
- `media`（array，可选）— 附件式媒体，不是把图嵌进正文。每项必须有：
  - `mime_type`（必填 string）— 目前只接受 `image/*`、`video/*`、`audio/*`
  - 以及 **`url` 或 `file_path` 二选一**（非空 string）
- `hint` **不是**本协议的一部分（LSP 专用）；写了会被忽略。

示例：

```json
{
  "content": "ok",
  "level": "warning",
  "media": [
    { "mime_type": "image/png", "file_path": "out/chart.png" },
    { "mime_type": "image/jpeg", "url": "https://example.com/a.jpg" }
  ]
}
```

**Windows 编码。** 子进程常走系统代码页（中文 Windows 多为 GBK）。脚本若按 UTF-8 写中文/非 ASCII，stdout 可能乱码或解析失败。没有自定义 `env` 可配。workaround：把解释器的 UTF-8 开关写进 `command`/`args`（例如 Python：`command`=`python`，`args` 含 `-X`、`utf8`、脚本路径）；或让脚本只输出 ASCII / 自行处理编码。不要指望平台注入 `PYTHONIOENCODING`。

**后台。** 调用时传 `"run_in_background": true`：立刻返回 `job_id`，结果稍后以 `CustomToolSettled` 提醒进会话（不要自己发明 FunctionCallOutput）。默认同步阻塞到结束。

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

**推荐流程（给 agent）。**

1. **写脚本** — 读 stdin JSON，写 stdout；失败写 stderr 非 0 退出。需要 envelope 时用上面的字段名。
2. **validate（不注册）** — 调用 `litecode_workspace`：`action=validate_custom`，并传 `definition`（同上面工具体）与 `sample_input`。看 PASS/FAIL，不要写文件。**PASS 只表示进程/envelope 跑通，不评价 `rules`**；有 rules 时响应里会提醒。规则只在真正调用（SAFE）时生效。
3. **注册** — 设置页保存，或写入 `.litecode/custom_tools.json`。
4. **refresh** — `litecode_workspace refresh custom_tools` 校验文件；修好 broken / 撞名 / 空 command。
5. **启用** — 请人在设置页 **Agents** 打开该工具（有 rules 时可切 ALL/SAFE）。不要替人打开。
6. **使用** — 新 turn 后模型可见；需要长任务时再开 `run_in_background`。

**怎么生效。** 无进行中的 turn 时工作区文件会重载。有 `rules` 时 Agents 可切 ALL/SAFE；改规则后下次调用按新规则。工作区同名条目整份覆盖全局；工作区这份没有规则时，不能切换 ALL/SAFE。子 agent 里 Ask 视为拒绝。

**怎么验证文件。** `litecode_workspace refresh custom_tools` 校验 JSON、名字、撞名、`command`；标出未生效差异。`validate_custom` 不写盘、不绑定、不跑 rules，只剥掉 `run_in_background` 后跑一遍进程（与生产 stdin 一致）。

**下一步。** 未打开时 status 会列成 off for you：只有这次真的需要才请人打开；否则不要提。
