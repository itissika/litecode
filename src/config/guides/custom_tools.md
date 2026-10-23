### `custom_tools.json`

**干什么。** 本工作区自定义工具（一条命令 + JSON Schema；可覆盖同名的全局项）。名字不能和内置工具撞名。子进程 cwd 为工作区根；环境会注入 `LITECODE_WORKSPACE` / `LITECODE_CALL_ID` / `LITECODE_SESSION_ID` / `LITECODE_TOOL_NAME`（后台任务还有 `LITECODE_JOB_ID`）。**只有这些**——定义里没有自定义 `env` 字段，也不会注入 `PYTHONIOENCODING` 等。

**先选调用面（`surface`）。** 一个 custom tool 只有一个调用面，默认 `direct`。不要按工具重要不重要选，按它将来怎么被调用选：

| `surface` | 在哪里调用 | 适合 | 限制 |
|---|---|---|---|
| `direct` | 模型/人直接调用，和其他 Direct 工具并列 | 一次性命令、人工交互、长任务、需要 media/后台 | 不能写进 `run_code` 程序 |
| `code` | 只能在 `run_code` 程序里调用：`await tools.<name>(args)` | 要被循环、分支、批量、聚合，或与 Code 工具组合 | 同步返回；结果统一为 `{ ok: true, text }`；`run_in_background` 不可用；media 不进入程序可见结果 |

不确定就选 `direct`。如果这个工具只需要模型单独叫一次，选 `code` 只会把它隐藏进程序，不会增加能力。

**怎么改。** 设置页增删改；或按格式改 JSON。名字必须是 `[a-z][a-z0-9_]*`，且必须和 map 的键相同。

**格式。** `version` 为 `1`。`surface` 可省略，默认 `direct`。`timeout` 单位秒，省略则为 120。不要写 `timeout: 0`（手写文件会立刻超时；设置页会改成 120）。Schema 类型字段是 `"type"`。

`surface: "code"` 的输入 schema 必须能被 Code SDK 渲染：顶层必须是 `object`，`properties` 是对象，不要使用未解析的 `$ref` 或递归 schema。`validate_custom` 和设置保存会先做准入检查；失败时不要登记或让用户启用。

`rules` 可省略，也可以写成 `[]`。这时这个工具只有开/关，调用前允许。写了至少一条，就可以在 ALL 和 SAFE 之间切换。ALL 允许这次调用。SAFE 按顺序看规则，先匹配到的那条生效；都没匹配到，就允许。

一条规则有 `id`、`action`、`when`。`action` 是 `allow`、`ask`、`deny`。`when.kind` 只能是 `any`、`arg_equals`、`arg_glob`、`path_outside_workspace`、`bash_readonly_command`、`all_of`、`any_of`。其它情况都要拒绝时，最后加一条 `"when": { "kind": "any" }`，`"action": "deny"`。

**调用约定。** `run_in_background` 是平台保留参数，不要在自己的 schema 里重复定义。参数以 JSON 对象写入命令的 **stdin**（`run_in_background` 不会进 stdin；`validate_custom` 同样会剥掉），读 stdout；exit 0 成功；exit 2 表示工具拒绝执行。取消或超时会 best-effort 杀掉进程树。

**stdout envelope（opt-in）。** 仅在 **exit 0** 时解析。stdout 必须是 JSON **对象**，且至少带 `media` 和/或 `level`；否则整段当纯文本。字段名以代码为准：

- `content`（string，可选）— 文本正文；**不是** `text` / `message`。缺省当 `""`。
- `level`（string，可选）— 只能是 `ok` / `warning` / `error`（大小写不敏感）。省略当 `ok`。未知值 → 硬失败（FAIL）。`error` 会让这次工具结果以 Error/FAIL 表面呈现（即使进程 exit 0）。
- `media`（array，可选）— 附件式媒体，不是把图嵌进正文。每项必须有：
  - `mime_type`（必填 string）— 目前只接受 `image/*`、`video/*`、`audio/*`
  - 以及 **`url` 或 `file_path` 二选一**（非空 string）
- `hint` **不是**本协议的一部分（LSP 专用）；写了会被忽略。

**Code 面的结果。** `surface: "code"` 时，程序拿到的是统一投影：

```json
{ "ok": true, "text": "<content 或纯文本 stdout>" }
```

stdout 仍是文本，不会被平台自动解析成对象。需要字段就写 `JSON.parse(result.text)`；需要失败分支就看 `result.ok` 和捕获 `ToolError`。`media` 不会进入 Code 程序可见结果；如果这个工具主要产出 media，选 `direct`。`run_in_background` 在 Code 面会返回明确的 invalid-input 错误，也不会被 SDK 暴露。

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

**后台。** 仅 Direct 可用。调用时传 `"run_in_background": true`：立刻返回 `job_id`，结果稍后以 `CustomToolSettled` 提醒进会话（不要自己发明 FunctionCallOutput）。默认同步阻塞到结束。

```json
{
  "version": 1,
  "tools": {
    "<name>": {
      "name": "<name>",
      "surface": "direct",
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

Code 面示例：

```json
{
  "name": "collect_metrics",
  "surface": "code",
  "description": "Collect metrics for one repository path.",
  "command": "python",
  "args": ["scripts/collect_metrics.py"],
  "timeout": 300,
  "schema": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "Workspace-relative path" }
    },
    "required": ["path"]
  }
}
```

**推荐流程（给 agent）。**

1. **先定 surface** — 只需要模型单独调用、需要后台/media/人工交互：`direct`。要被程序循环/分支/聚合，或与 Code 工具组合：`code`。不确定选 `direct`。
2. **写脚本** — 读 stdin JSON，写 stdout；失败写 stderr 非 0 退出。需要 envelope 时用上面的字段名。`code` 面优先输出稳定文本；要让程序消费 JSON，就让 `text` 是合法 JSON，由程序自己 `JSON.parse`。
3. **validate（不注册）** — 调用 `litecode_workspace`：`action=validate_custom`，并传 `definition`（同上面的工具体）与 `sample_input`。它会检查定义、`code` 准入、进程/stdout；看 PASS/FAIL 和具体字段，不要写文件。**PASS 不评价 `rules`**；有 rules 时响应里会提醒。
4. **注册** — 设置页保存，或写入 `.litecode/custom_tools.json`。
5. **refresh** — `litecode_workspace refresh custom_tools` 校验文件；输出会显示每个工具的 surface、code 准入状态和 rejected 原因。修好 broken / 撞名 / 空 command / 不支持的 code schema。
6. **启用** — 请人在设置页 **Agents** 打开该工具（有 rules 时可切 ALL/SAFE）。不要替人打开。
7. **使用** — 新 turn 后模型可见。`code` 工具只在 `run_code` 的 `tools.<name>` 中出现；`direct` 工具不会出现在 SDK 中。

**校验反馈怎么读。**

- `validate_custom — FAIL / definition`：配置本身不合法（name、command、rules 或 code schema）。读 `detail`，按其中字段和原因修正，不要重试同样定义。
- `validate_custom — FAIL / code admission`：`detail` 含 `surface 'code' input schema cannot be admitted`。常见原因是 `$ref`、非 object、不可渲染的 schema。改 schema 或改回 `direct`。
- `validate_custom — FAIL / composition`：单工具准入通过，但和当前 agent 已启用且可用的 Code 工具产生 SDK 类型名冲突。按 `detail` 改工具名/schema 类型字段，或先禁用冲突工具；不要忽略后直接启用。
- `validate_custom — FAIL / detail`：进程执行失败。看 stderr、退出码、超时或 envelope 字段错误。
- `validate_custom — PASS`：定义、进程/stdout、单工具准入，以及它与当前 agent 已启用 Code 工具的组合检查都通过。仍不代表 rules 通过；之后新启用另一个 Code 工具时仍可能产生新冲突。规则只在真实调用时按 agent 的 ALL/SAFE 生效。
- `refresh custom_tools` 中某个条目 rejected：该工具体不会进入运行时；先修报错，不要假设先注册再试。

**怎么生效。** 无进行中的 turn 时工作区文件会重载。有 `rules` 时 Agents 可切 ALL/SAFE；改规则后下次调用按新规则。工作区同名条目整份覆盖全局；工作区这份没有规则时，不能切换 ALL/SAFE。子 agent 里 Ask 视为拒绝。

**怎么验证。** `litecode_workspace refresh custom_tools` 校验 JSON、名字、撞名、`command`、`surface` 和单工具 Code 准入；标出未生效差异。`validate_custom` 还会对当前 agent 已启用且可用的 Code 工具做组合期 SDK 类型冲突检查。设置页 available-tools 的 `code_rejections` 是静态目录侧的同类诊断。`validate_custom` 不写盘、不绑定、不跑 rules，只剥掉 `run_in_background` 后跑一遍进程（与生产 stdin 一致）。

**下一步。** 未打开时 status 会列成 off for you：只有这次真的需要才请人打开；否则不要提。
