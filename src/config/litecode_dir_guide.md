# `.litecode/` 工作区运行时目录

本文件由 LiteCode 在打开工作区时**自动生成并覆盖**，请勿改内容。升级产品后以新版本为准。

`.litecode/` 是本仓库的运行时数据，不是源码。索引和快照始终跳过本目录。`grep` / `glob` 默认也硬跳过嵌套 `.litecode/`（不在 `excludes.json` 里，改排除列表无效）；要搜磁盘文件，把工具的 `path` 指到本目录或其子路径，或这次调用 `no_ignore`。也建议写进 `.gitignore`。会话稿不是磁盘文件，见下文 `.litecode/sessions/`。

**给 Agent：** 动手前先读本文件。能用专用工具就不要用 `write` / `edit` / `bash` 改这里。`CLAUDE.md` / 仓库根 `AGENTS.md` 才是项目契约，不要写进本目录。

**增查改删约定（下表「人 / Agent」列）**

| 标记 | 含义 |
|------|------|
| 读 | 可以打开、搜索 |
| 改 | 可以按格式编辑；改完由 LiteCode 重载或下次打开生效 |
| 增 | 可以新增该类型条目（通常走 UI / 专用工具，而不是手写文件名） |
| 删 | 可以删该类型条目 |
| 禁 | 不要创建、覆盖、移动、删除 |

快照不在本目录：文件回退在 `~/.litecode/snapshots/<workspace_id>/`；主机登记在 `~/.litecode/workspace-registry.json`。

---

## 启动时一定存在

这些在 `init_workspace` 里创建（已有则保留，本 README 每次覆盖）。

### `README.md`（本文件）

| | |
|--|--|
| 做什么 | 路径地图 |
| 格式 | Markdown（产品写入） |
| 人 / Agent | 读 · **禁改禁删**（下次打开会被覆盖） |

### `workspace.json`

| | |
|--|--|
| 做什么 | 稳定 `workspace_id`，把本树和主机侧快照目录绑在一起。不是会话 id、不是密钥 |
| 格式 | JSON：`{ "version": 1, "workspace_id": "<id>" }` |
| 人 / Agent | 读 · **禁改禁删**。复制工程会生成新 id；移动目录由产品对登记表做迁移 |

### `excludes.json`

| | |
|--|--|
| 做什么 | 工作区排除列表（资源管理器 / 搜索 / 文件监视）。设置页会写这个文件；磁盘改动会被监视器重载 |
| 格式 | JSON，`version` 为 `1`： |
| | `files_exclude` / `search_exclude` / `watcher_exclude`：glob 字符串数组 |
| | `git_ignore`：搜索与索引是否尊重 `.gitignore`（默认 `true`） |
| | `explorer_git_ignore`：资源管理器是否尊重 `.gitignore`（默认 `false`） |
| 人 / Agent | 读 · 改（设置页或按格式改 JSON）· 删文件会在下次打开时按内置默认重新种子。空行、`#` 注释、重复 glob 会被丢掉。本目录 `.litecode/` 的发现硬跳不走这份列表 |

### `sessions.db`

| | |
|--|--|
| 做什么 | 会话日志的唯一真源（SQLite）。侧车：`sessions.db-wal` / `sessions.db-shm` |
| 格式 | SQLite。不要当文本编辑 |
| 人 / Agent | **禁改**。查历史用 `session_search`，再 `read` / `grep` 虚拟路径 `.litecode/sessions/<session_id>.md`。整库删除等于毁掉本工作区会话（仅在你明确要重建时） |

### `logs/`

| | |
|--|--|
| 做什么 | 进程日志 |
| 格式 | `logs/litecode.log` 文本 |
| 人 / Agent | 读 · **禁改** 运行中的日志。排障可看，不要当会话记录 |

### `plan/`

| | |
|--|--|
| 做什么 | 当前工作区的计划 Markdown。文件名由 `plan` 工具生成（如 `calm-river.md`），不要自拟文件名 |
| 格式 | Markdown。会话里只记「当前计划」指针；`plan` 的 `finish` 只清指针，不删文件 |
| 人 / Agent | 读 · **增/结束走 `plan` 工具** · **禁** 用 `write` / `edit` / `bash` 覆盖、移动、删除 `plan/` 或其中文件 |

---

## 按需出现（第一次用到才建）

未出现是正常的，不要提前建空壳。

### `engines.json`

| | |
|--|--|
| 做什么 | 本工作区引擎是否启用。生命周期只认这个文件，不认工具目录 |
| 格式 | JSON：`{ "version": 1, "lsp": { "desired": bool, "servers": ["rust_analyzer", ...] }, "retrieval": { "desired": bool } }` |
| | `lsp.desired` 为 true 时 `servers` 不能为空。`retrieval.desired` 打开代码语义检索 |
| 人 / Agent | 读 · 改（设置 / 引擎开关；可按格式改 JSON）· 不要手改却指望不经重载就生效 |

### `mcp.json`

| | |
|--|--|
| 做什么 | 本工作区 MCP 服务器 |
| 格式 | JSON：`{ "version": 1, "servers": { "<id>": { "command", "args", "env", "transport", "timeout?" } } }` |
| | `transport`：`{ "type": "stdio" }`（默认）或 `{ "type": "remote", "url": "...", "headers": {} }` |
| | `timeout` 秒；省略则 60 |
| 人 / Agent | 增查改删走设置页或按格式改 JSON。不要把密钥写进此文件（用环境 / 主机配置） |

### `custom_tools.json`

| | |
|--|--|
| 做什么 | 本工作区自定义工具（命令 + JSON Schema） |
| 格式 | JSON：`{ "version": 1, "tools": { "<name>": { "name", "description", "schema": { "schema_type", "properties", "required" }, "command", "args", "timeout" } } }` |
| | `timeout` 默认 120 秒 |
| 人 / Agent | 增查改删走设置页或按格式改 JSON |

### `workspace.lock`

| | |
|--|--|
| 做什么 | 跨进程互斥：同一时刻只有一个 serve/CLI 占用本工作区 |
| 格式 | 锁文件；不要当配置读 |
| 人 / Agent | **禁** 删、改。进程异常退出后若无法再打开工作区，先确认没有其它 LiteCode 进程，再处理残留锁 |

### `bash/`

| | |
|--|--|
| 做什么 | 后台 bash 的 stdout/stderr，给 Agent 用 `read`（Safe 权限）看 |
| 格式 | `bash/<id>.output` 纯文本。Agent 任务 id 形如 `bg_<8 位 hex>` |
| 人 / Agent | 读 · **禁** 改正在写的 `.output`。任务结束后可当普通文件删 |

### `index/`（代码语义检索）

| | |
|--|--|
| 做什么 | `code_search` 引擎产物。打开检索引擎时建目录和 `meta.json` 壳，不立刻建向量 |
| 格式 | `meta.json`（模型 / pipeline / 统计）；`vectors.usearch`；`chunks.jsonl`；可选 `bm25/`；`pending_hint.json`（待增量条数） |
| 人 / Agent | **禁改禁删单文件**。重建走引擎，不要手编向量或 chunks |

### `session-index/`（会话语义检索）

| | |
|--|--|
| 做什么 | 会话语料的 ANN 索引。字面检索走 `sessions.db`，不在这里 |
| 格式 | `meta.json`；`vectors.usearch`；`chunks.jsonl` |
| 人 / Agent | **禁改**。查询用 `session_search` |

### `text-index/`（文本索引）

| | |
|--|--|
| 做什么 | 工作区文本索引（Tantivy） |
| 格式 | `meta.json`（`format` / `workspace_root` / `file_count` / `built_unix_ms`）；`tantivy/` 引擎目录 |
| 人 / Agent | **禁改** |

### `.litecode/sessions/`（虚拟，磁盘上通常没有这个目录）

| | |
|--|--|
| 做什么 | 把 `sessions.db` **投影**成可读 Markdown。真源仍是 SQLite，产品不往这里写文件 |
| 格式 | 虚拟路径 `.litecode/sessions/<完整 session_id>.md` |
| 人 / Agent | **只读**：`read` / `grep` / `glob`（`glob`/`grep` 必须把 path 指到 `.litecode/sessions` 或某个 `.md`，扫整个 `.litecode/` 不会列出这些虚文件）· `write` / `edit` 会拒绝 · 不要 `mkdir` 这个目录 |

---

## 不要放进 `.litecode/` 的东西

| 路径 | 说明 |
|------|------|
| `CLAUDE.md`、仓库根 `AGENTS.md` | 项目契约，在仓库根 |
| `.litecode/snapshots/` | 旧位置；打开工作区时会清掉。回退在 `~/.litecode/snapshots/` |
| 密钥、邮箱、内部地址 | 不要写入本目录任何 JSON |
| 源码、构建产物 | 本目录不是项目文件树 |

---

## Agent 速查

1. 改项目规则 → 仓库根 `CLAUDE.md`（及已有的 `AGENTS.md`），不是本 README。
2. 改排除规则 → `.litecode/excludes.json` 或设置页（不管 `.litecode/` 自身；搜本目录用 `grep`/`glob` 的 `path`）。
3. 计划 → `plan` 工具；待办 → `todo` 工具（待办在会话状态里，不在本目录单独成文件）。
4. 旧会话 → `session_search`，再读 `.litecode/sessions/<id>.md`。
5. 代码语义 → 确认 `engines.json` 里 `retrieval.desired`，用 `code_search`，不要碰 `index/`。
6. 后台命令输出 → `read` `.litecode/bash/<id>.output`。
7. 不要删 `.litecode/`、`sessions.db`、`workspace.json`、`workspace.lock`、`plan/`、各 `*-index/`。
