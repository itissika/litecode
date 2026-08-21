# LiteCode 会话数据（目标）

每层只答四件事：**职责、身份、形状、规则**。  
种类只写在 `SessionLog.kind`。SDK `Item` 只出现在 `item/*` 的 `body`，以及 AgentView 的装配结果。

---

## 标识

| 名 | 指什么 |
|---|---|
| `SessionId` | 哪一条会话 |
| `LogSeq` | `SessionLog` 第几行。单调、不改号、不复用 |
| `CallId` | 一次 `item/tool_call` 与 `item/tool_result` 配对（Responses `call_id`） |
| `ItemId` | provider 流式/续传（Responses `id`）。不是 `LogSeq` |
| `TurnId` | Live 里这一轮是否在跑。可回放靠 `turn/end` 行 |

---

## 分层

```text
存   SessionMeta          现在的指针
     SessionLog           会话流（磁盘为准，内存是热镜像）

算   脊骨 LogSeq[]        SessionLog 按 kind 折出的可见序列

用   GateRow[]            脊骨的可提交形态（进门）
     AgentView            脊骨 → Item[]（给模型）
     HumanView            脊骨 → 卡片（给人）

活   Live                 本轮相位。不是 SessionLog
```

写 SessionLog 只有三种：`append`（新 `LogSeq`）· `seal`（同一 `LogSeq` 写终态）· `truncate`（从某条 `item/user` 起含该行删掉其后）。  
AgentView 与 HumanView 都不回写。GateRow 是唯一提交口。能否开下一轮只问 Live。

---

## SessionMeta

| | |
|---|---|
| 职责 | 会话行：身份 + 这一刻的绑定与指针。不是时间线，也不是 SessionLog |
| 身份 | `SessionId`（`id`） |
| 形状 | 下表全部字段。无隐藏列 |
| 规则 | 创建后不变的只写一次。会变的只表示「现在」。曾经怎样，写 `request/header` / `SessionLog`。Codex 把 `SessionMeta` 塞进 rollout 第一行；DSH 的 `SessionHeader` 在日志外。LC 跟 DSH：Meta 在 `sessions` 行，不进 SessionLog |

子会话是独立 Session（自己的 Meta + Log + Live）。父子只靠本表指针；子在跑 ≠ 父在跑。

### 身份（创建后不变）

对齐 DSH `SessionHeader`：id / cwd / 子会话谱系。不抄 Codex 的 originator、cli_version、analytics `source`、`forked_from`（LC 的 truncate 仍是同一 `id`）。

| 字段 | 类型 | 职责 |
|---|---|---|
| `id` | `SessionId` | 本会话 |
| `created_at` | i64 | 创建时刻 |
| `project` | String | 工作区路径（DSH `cwd`） |
| `parent_session_id` | `Option<SessionId>` | 子会话的父。根会话 `None`。不是 fork 谱系 |
| `parent_call_id` | `Option<CallId>` | 拉起本会话的那次 `item/tool_call`。与父 id 成对；DSH 无此列，LC 要按 call 找回子会话 |
| `subagent_depth` | u32 | 委托深度：根 `0`，子 = 父 + 1。对齐 DSH `delegationDepth`，重启后仍能限递归 |

### 现在绑定（可改指针）

当前选用。换绑定时改这里；该次请求的完整装配另 `append request/header`（DSH `EpochHeader`：config / system / tools）。不抄 Codex `base_instructions`、`dynamic_tools`、capability roots 进 Meta。

| 字段 | 类型 | 职责 |
|---|---|---|
| `agent_id` | String | 当前 agent（DSH `agentPreset` / Codex `agent_role`） |
| `model_id` | `Option<String>` | 当前模型目录 id；`None` = 未选定。不是 `api_model_id`（那是目录投影） |
| `thinking_tier` | String | 当前思考档 |
| `context_mode` | String | 当前上下文档 |
| `updated_at` | i64 | Meta 或预览最后一次改动 |

### 现在窗口指针

不是 `MAX(LogSeq)`。最大 seq 从 SessionLog 算。对齐 Codex compact 窗口指针的精简版：只留「当前摘要行」和「脊骨从哪起」。

| 字段 | 类型 | 职责 |
|---|---|---|
| `compacted_seq` | `Option<LogSeq>` | 当前有效的 `compacted` 行。从未 compact 则为 `None`（现列 `checkpoint_seq`，`0` 改成 `None`） |
| `spine_from` | `LogSeq` | 脊骨起点。无 compact 时为日志起点。被换掉的行仍在 SessionLog |

### 现在产品指针

本体可在旁路文件；这里只留「现在用哪份」。

| 字段 | 类型 | 职责 |
|---|---|---|
| `todos` | `[{ content, status }]` | 会话待办整表；compact 不改这一列 |
| `plan_slug` | `Option<String>` | 当前 plan 文件的 slug；正文在旁路 |
| `preview` | String | 会话列表摘要，通常来自最近一条 `item/user` |

### 不进 SessionMeta

| 落在 | 字段 |
|---|---|
| Live / 线投影 | `TurnId`、相位、是否在跑、compacting、bash jobs、权限、token 统计 |
| 目录投影 | `api_model_id`、`label`、`context_window` |
| SessionLog | 换过的模型/工具集（`request/header`、`request/context`）、回合起止 |
| 派生 | `max_seq` / `next_seq`、文件撤回锚点、FTS、meter |
| 不抄 | Codex `WorldState`、Guardian、Realtime、fork 线程 id、`memory_mode` |

---

## SessionLog

| | |
|---|---|
| 职责 | 会话流唯一真源 |
| 身份 | 每行一个 `LogSeq` |
| 形状 | `{ seq, time, kind, body, cites?: LogSeq[] }` |
| 规则 | 新能力只加 `kind`。未知 kind：行保留，不进脊骨，两 View 跳过 |

`body` 的 schema 由 `kind` 决定，不要求是 `Item`。过大则 `body_ref`，`LogSeq` 不变。  
`cites` 只表达「引用哪些先前行」，不承担配对或折叠（配对用 `CallId`，折叠用 `compacted.body` 的 `from`/`to`）。

`kind` 决定：进不进脊骨、如何改脊骨、两 View 如何读 `body`。没有平行的 `surface_op`。

### kind

#### `item/*` — `body` 即 Responses `Item`

| kind | 脊骨 | HumanView | AgentView | body |
|---|---|---|---|---|
| `item/user` | 追加 | 气泡 | 原样 | `message` + `role=user` |
| `item/assistant` | 追加 | 气泡或思考 | 原样 | assistant `message` 或 `reasoning` |
| `item/tool_call` | 追加 | 按工具名 | 原样 | `function_call`（含 `call_id`） |
| `item/tool_result` | 追加 | 按工具名 | 原样 | `function_call_output`（含 `call_id`） |

权威里 `role=user` 的 `message` 只来自 `item/user`。`truncate` 锚点也只许这个 kind。  
工具卡片：bash / kill_shell / wait_shell → 命令；edit / write → diff；mcp → MCP；subagent → 子会话；其余 → 通用行。

#### `compacted` — 改脊骨

| kind | 脊骨 | HumanView | AgentView | body |
|---|---|---|---|---|
| `compacted` | 本行替换 `[from, to)`。被换行仍在 SessionLog | 切痕，不展示摘要 | 脊骨**最前**一条由 `summary` 装配的 assistant `Item`（不回写） | `{ summary, from, to }` |

#### 注入 — 不是人键入；`body` 不是 `Item`

| kind | 脊骨 | HumanView | AgentView | body |
|---|---|---|---|---|
| `hook/prompt` | 追加 | 默认隐藏 | 带标记的 user/developer `Item` | `{ text, hook_run_id, placement? }` |
| `reminder/job_exit` | 追加 | 默认隐藏 | 带标记的 user `Item` | `{ job_id?, reason: exit \| kill \| timeout, text }` |
| `reminder/turn_aborted` | 追加 | 默认隐藏 | 带标记的 user `Item` | `{ text }` |

`reminder/job_exit` 只记账；要不要因此开一轮问 Live。后台 job 结束用它，不用 `seal`。  
`reminder/turn_aborted` 在 `seal` 取消本轮之后、下一轮 AgentView 需要知情时再 `append`。

#### 控制面 — 不进脊骨，两 View 都不读

| kind | body |
|---|---|
| `turn/start` | `{ turn }` |
| `turn/end` | `{ turn, reason: completed \| cancelled \| error \| max_steps \| hook_blocked }` |
| `request/header` | `{ provider, model, thinking_tier?, context_mode?, … }` 当时调用绑定（DSH `EpochHeader.config`） |
| `request/context` | 当时系统提示、工具集等装配快照 |

### 不是 kind

| | 落在哪 |
|---|---|
| `seal` | 写原语：同一 `LogSeq` 上把未终态的 `item/assistant` 或 `item/tool_call` 写成终态。取消本轮用这个 |
| 未配对 call 的补 `Item` | 只出现在 AgentView |
| `step/*`、流式 chunk | Live |
| 网络失败 | Live；`turn/end.reason=error`。不另写 reminder |

---

## Item

| | |
|---|---|
| 职责 | 模型原子 |
| 身份 | 无（`ItemId` / `CallId` 都不是 `LogSeq`） |
| 形状 | SDK `message` / `reasoning` / `function_call` / `function_call_output` |
| 规则 | 入账的 user `message` 只有 `item/user`。AgentView 可另造带标记的 user/developer `Item`。bash、websearch 仍是 `function_call` |

---

## 脊骨

| | |
|---|---|
| 职责 | 折叠之后，两个 View 和 GateRow 共同看见的 `LogSeq` 序列 |
| 身份 | 无，从 SessionLog 算出 |
| 形状 | `LogSeq[]` |
| 规则 | 打开全量算一次，之后随 `append` / `seal` / `truncate` / `compacted` 增量改。不是 token 预算，也不是 AgentView |

---

## GateRow

| | |
|---|---|
| 职责 | 把脊骨变成可 `append` / `seal` 的工作集 |
| 身份 | `log_seq: Option<LogSeq>`（`None` = 尚未入账） |
| 形状 | `{ log_seq, kind, body }` |
| 规则 | 一份。只交相对上次的 delta。从脊骨装入时 `log_seq` 皆 `Some`。`seal` 只作用于未终态的 `item/assistant` 与 `item/tool_call` |

---

## AgentView

| | |
|---|---|
| 职责 | 这一步发给 provider 的 `Item[]` |
| 身份 | 无 |
| 形状 | 按脊骨取 `body`，再按 kind 装配 |
| 规则 | 可跳过、裁剪、补未完成 call。不回写 SessionLog |

---

## HumanView

| | |
|---|---|
| 职责 | 人怎么看见脊骨上的每一行 |
| 身份 | 投影仍挂在那一行的 `LogSeq` |
| 形状 | 由 kind（工具则再加 `name`）决定 |
| 规则 | 不另存一份时间线 |

---

## Live

| | |
|---|---|
| 职责 | 这一轮跑到哪 |
| 身份 | `TurnId` |
| 形状 | 相位、流式、权限、token |
| 规则 | 与 SessionLog 分权。开不开下一轮只问这里 |

---

## 回合

```text
SessionLog → 脊骨 → GateRow（已入账行 log_seq 皆 Some）
append  item/user | hook/prompt | reminder/* | item/* | compacted | 控制面
seal    未闭合的 item/assistant 或 item/tool_call     → 取消本轮
append  reminder/turn_aborted                         → 仅当下一轮模型需要知情
AgentView = 装配(脊骨)
结束    已入账 LogSeq 不再当新行提交
```

---

## 旁路

meter、FTS、snapshot、plan 文件正文、配置：派生或邻接，不另立时间线。当前 plan 的 slug 在 SessionMeta。
