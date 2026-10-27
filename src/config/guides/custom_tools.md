# custom_tools.json — agent runbook

目标：写出一个能被 `validate_custom` / `refresh custom_tools` 接受、能被 Agents 启用、能被模型或 `run_code` 正确调用的自定义工具。文件：`.litecode/custom_tools.json`，`version` 必须为 2。

## 0. 模型

一个工具 = 一条命令 + stdin JSON 契约 + stdout 契约。`surface` 决定调用入口；standalone / suite / member 决定权限和绑定。

| 形态 | 写在哪 | 必填 | surface / rules | 调用名 | Agents |
|---|---|---|---|---|---|
| standalone | `tools.<name>` | `name` `command` `schema` + 恰好一个 `surface`；code 还要 `output_schema` | 自己拥有 | direct `<name>` / code `tools.<name>` | 单独一张卡 |
| suite member | `tools.<name>` | `name` `command` `schema` + 恰好一个 `suite`；code 还要 `output_schema` | 继承 suite；禁止 `surface`/`rules` | direct `<name>` / code `tools.<suite>.<member>` | 跟随 suite |
| suite | `suites.<id>` | `surface`；`label`/`rules` 可选 | 拥有 surface/rules | 不是工具 | 一张卡、一行 binding、一个权限面 |

- `tools.<name>.name` 必须等于 map key；suite id 就是 `suites` 的 key。同一层内工具名和 suite 名共享命名空间，不能互撞，也不能撞内置/保留名。
- 跨层同名是覆盖，不是错误：workspace 同名工具条目（standalone/member）整份覆盖 global 的同名工具；同名 suite 整组覆盖 global suite 及成员。不要为了“全局唯一”改名。
- `direct` suite 的成员调用名是平的（`<member>`）；`code` suite 永远是 `tools.<suite>.<member>`，即使只有一个成员。standalone code 是 `tools.<name>`。平铺只指调用名；Agents 里 suite 仍是一张卡。

## 1. 选 surface

| surface | 调用位置 | 适合 | 硬限制 |
|---|---|---|---|
| `direct` | 模型/人直接调用，和其他 Direct 工具并列 | 一次性命令、人工交互、长任务、media、后台 | 不进 `run_code` SDK |
| `code` | 只在 `run_code` 程序里 | 循环/分支/批量/聚合，和 Code 工具组合 | 同步；必须 `output_schema`；无后台；media 不进程序结果 |

不确定选 `direct`。只需要模型单独调一次的工具选 `code` 不会增加能力，只会把它藏进程序。

## 2. 模板

~~~json
{
  "version": 2,
  "suites": {
    "deck": {
      "surface": "code",
      "label": "Deck",
      "rules": [{ "id": "outside", "when": { "kind": "path_outside_workspace", "name": "path" }, "action": "deny" }]
    }
  },
  "tools": {
    "repo_overview": {
      "name": "repo_overview", "surface": "direct",
      "description": "Summarize a repository path.",
      "command": "python", "args": ["scripts/repo_overview.py"], "timeout": 60,
      "schema": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] },
      "rules": [{ "id": "outside", "when": { "kind": "path_outside_workspace", "name": "path" }, "action": "deny" }]
    },
    "file_stats": {
      "name": "file_stats", "suite": "deck",
      "description": "Count lines in one file.",
      "command": "python", "args": ["scripts/file_stats.py"],
      "schema": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] },
      "output_schema": { "type": "object", "properties": { "lines": { "type": "integer" } }, "required": ["lines"] }
    }
  }
}
~~~

direct standalone + code suite。改成 direct suite 时，把 `suites.<id>.surface` 改成 `direct`，并**删掉成员的 `output_schema`**（direct 禁止）；成员调用名从 `tools.deck.file_stats` 变成 `file_stats`，Agents 仍是 suite 一张卡。

## 3. 字段

| 字段 | 适用 | 必填 | 说明 |
|---|---|---|---|
| `name` | tool | 是 | `[a-z][a-z0-9_]*`；必须等于 map key |
| `description` | tool | 否 | 给模型看的说明；空则用 `External tool: <name>` |
| `command` | tool | 是 | 直接执行的程序，不经过 shell；cwd 是工作区根 |
| `args` | tool | 否 | string[]；需要管道/重定向时显式用 `bash -lc` / `cmd /C` |
| `timeout` | tool | 否 | 秒，默认 120；必须 >= 1；不要写 0 |
| `schema` | tool | 是 | 输入 schema；顶层必须是 object |
| `surface` | standalone/suite | 是 | `direct` / `code`；standalone 不能和 `suite` 同时 |
| `suite` | member | 是 | 已存在的 suite id；不能和 `surface` 同时 |
| `rules` | standalone/suite/member | standalone/suite 否；member 必须空 | member 由 suite 统一拥有 |
| `output_schema` | code tool | 是 | object schema；direct 写了会被拒绝 |
| `label` | suite | 否 | 显示名；设置页空则用 id |

### schema

- 所有 tool 的 `schema` 顶层必须是 `"type": "object"`；direct 也一样。
- `schema` 只保留 `type` / `properties` / `required`；根级其他关键字会被丢弃。把限制写进 `properties` 的子 schema。
- `properties` 必须是对象；`required` 是字符串数组。
- direct 会自动加 `run_in_background`；不要在自己的 `schema` 里定义它，任何 surface 定义了都会被拒绝。
- code 的 `schema` 和 `output_schema` 必须能被 Code SDK 渲染：顶层 object；不要用 `$ref`、`allOf`、`not`、`patternProperties`、`dependencies`、`if`、`unevaluatedProperties`；`additionalProperties` 只能是布尔；用简单类型/对象/数组，不确定就保持最小 schema。
- `output_schema` 只对 code；必须是 `type: object` 的非空 schema。SDK 类型由它生成，但运行时只检查 stdout 顶层是对象，不校验字段；脚本和 schema 自行保持一致，否则 SDK 类型会骗你。
- 不要在 `output_schema` 里定义 `ok`：平台只在缺失时补 `ok: true`，已有 `ok` 不会覆盖。业务失败用非 0 + error envelope，不要用 `ok: false`。
- 写了 `output_schema` 就不要再套 media/level envelope；stdout 本身就是数据。

## 4. 运行时

### stdin / stdout / exit

- 参数以 JSON 对象写入 **stdin**；从 **stdout** 输出结果；stderr 用于错误信息。
- cwd = 工作区根；取消/超时会 best-effort 杀掉进程树。
- exit 0 = 成功；exit 2 = 工具拒绝执行（硬错误，不是权限 Ask）；其他非 0 = 失败。
- 环境：额外注入 `LITECODE_WORKSPACE` / `LITECODE_CALL_ID` / `LITECODE_SESSION_ID` / `LITECODE_TOOL_NAME`；后台还有 `LITECODE_JOB_ID`。其余环境继承父进程；没有自定义 `env` 字段，也不会注入 `PYTHONIOENCODING`。
- Windows：stdin/stdout 按 UTF-8 处理；Python 用 `-X utf8`，或让脚本自己处理编码，不要依赖系统代码页。

### direct stdout envelope（可选）

只有 exit 0 且 stdout 是 JSON **对象**、并且至少含 `media` 或 `level` 时，才按 envelope 解析；否则整段当纯文本。

| 字段 | 约束 |
|---|---|
| `content` | string，可选；文本正文；不是 `text`/`message`；缺省 "" |
| `level` | `ok` / `warning` / `error`（大小写不敏感）；省略 ok；未知值硬失败；`error` 让结果以 Error 呈现（即使 exit 0） |
| `media` | 数组；附件，不嵌正文；每项 `mime_type` 必填，且 `url` / `file_path` 至少一个；两者都有时优先 `url`；mime 只接受 image/ video/ audio/ |
| `hint` | 不是本协议；忽略 |

后台结果只回文本；media 不会随 `CustomToolSettled` 回传。后台工具主要产出 media 时，用同步调用。

### code 成功

- `output_schema` 必填；stdout 必须是 JSON **对象**。程序拿到 stdout 对象，并在缺失时补 `ok: true`；SDK 类型来自 `output_schema`。
- 没有 `output_schema` 会在注册/refresh 时被拒绝；stdout 不是对象/JSON 时本次调用以 `invalid_output` 失败。
- 需要纯文本也输出对象，例如 `{"text":"..."}`；不要输出裸文本。
- 程序只检查顶层是对象，不校验字段；stdout 自带 `ok: false` 时程序会看到 false，不会自动改成 true。不要定义 `ok`。
- code 不解析 direct envelope；不要写 `media`/`level`。code 无 `run_in_background`；media 不进程序可见结果。

### code 失败

非 0 退出时，stdout 可以带：

~~~json
{"error":{"code":"not_found","message":"order 42 does not exist","details":{"order_id":42},"retryable":false}}
~~~

- 软失败码（变成 `{ok:false, reason, message}`，不中断 `Promise.all` 的其他调用）：`not_found`、`invalid_input`/`invalid_regex`、`permission_denied`、`io`、`no_match`、`empty`、`conflict`、`stale`。reason 映射：`invalid_input`/`invalid_regex` -> `invalid_args`；`permission_denied` -> `permission`；其他同名。
- 其他 code（包括 `timeout`、`stale_cursor`、`output_limit`）或没有 envelope：抛 `ToolError`，带 `code`/`details`/`retryable`。
- 自定义工具里 `retryable` 实际只在硬 `timeout` 上有效；软失败不携带 `retryable`，`details` 也不进程序。
- `message` 省略时用 stderr；`details` 只在硬失败进入 `ToolError.details`。
- direct 不解析这个 envelope；exit 2 是 `blocked execution`，其他非 0 是 `exited with code N: stderr`。

### timeout / background

- `timeout` 是硬失败；会杀进程树。`timeout: 0` 不是无限：手写文件实际 1s，设置页保存会改成 120；写明确 >= 1。
- `run_in_background` 只对 direct 可用；平台自动加进 direct 的 schema；调用时传 `true`，立即返回 `job_id`，结果稍后以 `CustomToolSettled` 提醒进会话（不要自己发明 FunctionCallOutput）。`validate_custom` 会剥掉它，只测同步路径。
- code 传 `run_in_background` 会返回 invalid-input；SDK 不暴露它。

## 5. rules

standalone 或 suite 没有 `rules` 时只有开/关，调用前允许，不能切 ALL/SAFE；member 的规则看 suite。有至少一条 rules 时 Agents 可切 ALL/SAFE：ALL 允许这次调用；SAFE 按顺序匹配，第一条生效，都没匹配到就允许；要默认拒绝，最后放一条 `any` + `deny`。子 agent 里 `ask` 视为拒绝。

- 规则字段：`id` / `action` (`allow`/`ask`/`deny`) / `when`。`id` 必须非空、同一列表唯一、不能是 `__default`。
- suite 的 `rules` 由 suite 拥有；member 的 `rules` 必须为空。
- `when.kind` 与字段：

| kind | 字段 | 语义 |
|---|---|---|
| `any` | 无 | 总是匹配 |
| `arg_equals` | `name`, `value` | 字符串参数精确相等；非字符串不匹配 |
| `arg_glob` | `name`, `pattern` | 字符串参数；只支持一个 `*`（前缀/后缀）；不是正则 |
| `path_outside_workspace` | `name` | 取该参数路径；`path`/`file_path` 可互换；在工作区外匹配 |
| `bash_readonly_command` | 无 | 看 `command`；只读命令匹配 |
| `all_of` | `matchers` | 全部匹配 |
| `any_of` | `matchers` | 任一匹配 |

## 6. 流程

1. **定 surface**：模型单独调/后台/media -> `direct`；程序循环/分支/聚合 -> `code`。不确定 `direct`。
2. **定形态**：单个工具 standalone；多个相关工具共享 surface/rules 用 suite + members。
3. **写脚本**：读 stdin JSON，写 stdout；失败写 stderr + 非 0；遵守上面的成功/失败/envelope 契约。
4. **写定义**：按模板；先过下面的 preflight。
5. **验证**：standalone 或**已存在 suite** 的 member 用 `validate_custom`，传 `definition`（单个 tool body，不是整份文件）+ `sample_input`（object）。**新 suite** 见 6.2。
6. **注册并 refresh**：设置页保存或写文件；`litecode_workspace refresh custom_tools`；修 rejected。
7. **启用**：请人在设置页 Agents 打开；有 rules 时可切 ALL/SAFE。不要替人打开。
8. **使用**：新 turn 后可见；direct 不进 SDK；code standalone `tools.<name>`；code suite `tools.<suite>.<member>`。

preflight：

- `version: 2`；没有 `data_schema`/`suite_label`/`version: 1`。
- `name` 匹配正则且等于 key；standalone 恰好一个 `surface`；member 恰好一个 `suite` 且没有 `surface`/`rules`。
- `command` 非空；`timeout` >= 1；`schema` 顶层 object；code 有 `output_schema`；direct 没有 `output_schema`。
- member 引用的 suite 在当前层已定义；规则 id 合法。

### 6.1 验证入口

| 入口 | 能做什么 | 不能做什么 / 边界 |
|---|---|---|
| `validate_custom` | 单个 tool body；suite 已加载；跑命令；检查定义/进程/输出/code 准入/当前已启用 Code 工具组合 | 不跑 rules；不能验新 suite+成员或单独 suite；不测后台；不写配置但命令自身可能写盘 |
| `refresh custom_tools` | 文件级 version/JSON/名字/suite 引用/撞名/command/surface 继承/code 准入 | 不跑进程；suite 自身的保留/内置名不看，rules 只在有成员引用时看；rejected tool 不进运行时 |

- `validate_custom` 会剥掉 `run_in_background`，只测同步路径；`sample_input` 会被真实执行，样例要安全。
- `validate_custom` 的 PASS 不代表 rules 通过，也不保证以后启用新 Code 工具不冲突。
- suite 定义本身最稳用设置页保存一次（会校验 suite 名和 rules）；只靠 `refresh` 可能漏掉 suite 名与内置/保留名的冲突。

### 6.2 新 suite

先把 suite + members 写入文件 -> `refresh custom_tools`（结构/引用/准入）-> 等文件重载 -> 再 `validate_custom` 一个 member（进程/输出/组合）-> 启用。`refresh` 不会跑进程，也不会替代 `validate_custom`；suite 名/rules 最好让设置页保存一次再继续。

## 7. 反馈

| 输出 | 含义 / 处理 |
|---|---|
| `field:` + `expected`/`actual`/`fix` | 定义错误；按字段改，不要重试同一定义 |
| `code admission: FAIL` | input 或 output schema 不可渲染；去 `$ref`/allOf/...，保持 object；或改 direct |
| `detail: output_schema is declared` | code 的 stdout 不是 JSON 对象；改脚本，不要删 `output_schema` |
| `composition: FAIL` | SDK 类型名/命名空间冲突；改名/改 schema 字段，或先禁用冲突 Code 工具 |
| 其他 `FAIL` + `detail` | 进程失败/超时/envelope 错；看 stderr、退出码 |
| `PASS` | rules 未评价；以后启用新 Code 工具仍可能冲突 |
| refresh rejected | 该 tool 不进入运行时；修 reported field |

## 8. 生效

- 无进行中的 turn 时工作区文件重载；新 turn 后模型可见。
- suite 没有 rules 不能切 ALL/SAFE；有 rules 才能切。子 agent 的 `ask` = 拒绝。
- 未打开时 status 会显示 off for you；只在真正需要时请人打开，不要替人启用。
