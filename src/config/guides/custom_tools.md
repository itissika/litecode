### `custom_tools.json`

目标：写出一个能被 `workspace.refresh()` 接受、能在 Settings → **Agents** 启用、并能在 `run_code` 内直接通过 `tools.<name>` 或 `tools.<suite>.<member>` 正确调用的自定义工具。文件：`.litecode/custom_tools.json`，`version` 必须为 2（工作区同名条目整份覆盖全局配置）。

**干什么。** 一个自定义工具 = 一条子进程命令 + stdin JSON 契约 + stdout/退出码契约。所有自定义工具均运行在 `run_code` 沙箱接口之后（不向模型单独暴露直调工具，也不支持 `run_in_background`、`surface` 或 `output_schema`）：

| 形态 | 写在哪 | 必填 | rules / 调用名 | Agents 展示 |
|---|---|---|---|---|
| standalone | `tools.<name>` | `name` `command` `schema` | 自己拥有 `rules`；调用名 `tools.<name>(args)` | 单独一张卡 |
| suite member | `tools.<name>` | `name` `command` `schema` `suite` | 继承 `suites.<id>`，禁止自带 `rules`；调用名 `tools.<suite>.<member>(args)` | 跟随 suite |
| suite | `suites.<id>` | 可选 `label` / `rules` | 统一拥有该组 `rules`（不是可调用工具） | 一张卡、一行 binding |

**怎么改。** 设置页 **Custom Tools** 可直接编辑并实时校验；也可以直接改 `.litecode/custom_tools.json`。每个 `<name>`（standalone / member）和 `<id>`（suite）都必须满足 `^[a-z][a-z0-9_]*$`，`tools.<key>` 里的 key 必须等于 `name`，且不能与内置工具、JavaScript 保留字或同批 `custom_tools` 重名；suite 成员禁止再写 `rules`（若写旧字段 `surface` / `output_schema` 会被直接忽略/不生效，不要在新定义里保留）。

**格式。** 最小 standalone + suite 示例：

```json
{
  "version": 2,
  "suites": {
    "deck": {
      "label": "Deck",
      "rules": []
    }
  },
  "tools": {
    "echo_py": {
      "name": "echo_py",
      "description": "Echo message from stdin",
      "command": "python",
      "args": ["tools/echo_py.py"],
      "timeout": 120,
      "schema": {
        "type": "object",
        "properties": {
          "message": { "type": "string", "description": "Text to echo" }
        },
        "required": ["message"]
      }
    },
    "render_slide": {
      "name": "render_slide",
      "suite": "deck",
      "command": "python",
      "args": ["tools/render_slide.py"],
      "schema": {
        "type": "object",
        "properties": {
          "slide": { "type": "integer" }
        },
        "required": ["slide"]
      }
    }
  }
}
```

关键字段规则：

- `schema`：顶层必须是 `"type": "object"`，尽量写清字段类型与 `required`（`schema` 同时用于参数校验与生成 TypeScript 类型签名，不支持复杂 `$ref`/`allOf`）；
- `timeout`：单位秒，>= 1，默认 120；
- `rules`：权限规则数组（仅 standalone 与 `suites.<id>` 可设），决定 **SAFE** 档位下的拦截条件；不设 `rules` 或 `rules` 为空时仅提供开关绑定。

**执行与 stdin / stdout 契约。**

- 输入与环境：参数对象作为 JSON 写入子进程 `stdin`；工作目录为工作区根；注入 `LITECODE_WORKSPACE`、`LITECODE_CALL_ID`、`LITECODE_SESSION_ID`、`LITECODE_TOOL_NAME`。
- 退出码 0（成功）：
  - stdout 为合法 JSON **对象**：字段原样进入 `run_code` 返回值（`Promise<unknown>`）；可选识别 `level`（`"ok"` | `"warning"` | `"error"`，为 `"error"` 时以 `message`/`content` 失败）与 `media` 数组（元素含 `mime_type` 与 `url` 或 `file_path`；返回到程序外才向模型交付多模态，**不 return 不给**）；
  - stdout 为合法 JSON **数组**：数组原样返回给程序；
  - 其余纯文本 / 非对象标量：包装为 `{ "text": "<stdout>" }` 返回给程序。
- 退出码 2：默认按拒绝/拦截报错（`blocked execution`）；其他非 0：优先解析 stdout 中的 `{"error":{"code","message","retryable?","details?"}}` 结构化失败信封，否则回退 `exited with code N: <stderr>`。

**怎么生效与验证。**

- 磁盘修改后下一个 turn 生效，进行中的 turn 不受影响。
- 修改后调用 `workspace.refresh()` 一次校验 `.litecode/custom_tools.json`（及 `excludes.json` / `mcp.json` / `apps.json`）的结构、命名与 `run_code` 代码准入；被拒绝的工具不会进入运行时。
- 调 `workspace.status()` 查看哪些 custom 工具当前已在运行态挂载，或被标成 off for you / disabled。

**下一步。** 写好并确认 `workspace.refresh()` 无错误后，若本次任务需要人启用该工具，请提醒人类在设置页 **Agents** 打开对应工具或 suite 开关（不要替人开启）。
