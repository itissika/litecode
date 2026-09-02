# `.litecode/` 工作区运行时目录

本文件由 LiteCode 在打开工作区时**自动生成并覆盖**，请勿改内容。升级产品后以新版本为准。

`.litecode/` 是本仓库的运行时数据，不是源码。建议写入 `.gitignore`。索引和快照始终跳过本目录。`grep` / `glob` 默认也硬跳过嵌套 `.litecode/`（不在 `excludes.json` 里，改排除列表无效）；要搜这里的磁盘文件，把工具的 `path` 指到本目录或其子路径，或这次调用 `no_ignore`。

项目契约在仓库根 `CLAUDE.md` / `AGENTS.md`，不要写进本目录。

下面分两类：**可改配置**（干什么、怎么改、格式、怎么生效）和**只读**（能看，不要动手）。未出现的文件是正常的（第一次用到才建），不要提前建空壳。

---

## 可改的配置

优先走设置页，也可以按格式改 JSON。不要把密钥、邮箱、内部地址写进这些文件。

### `excludes.json`

打开工作区时若不存在会按内置默认种子。

**干什么。** 工作区排除。三套 glob **不要并成一套**（对齐 VS Code：树 / 检索 / 监视器）：

- `files_exclude`：资源管理器当文件不存在；检索默认也不碰。
- `search_exclude`：资源管理器还能看见，但人的搜索、`grep` / `glob`、文本索引、语义索引默认不扫。生成物（`Cargo.lock`、`package-lock.json`、`*.min.js` 等）不想被检索时写在这里，不要靠引擎再滤一层语言表。
- `watcher_exclude`：监视器**源上硬切**，命中的路径不上变化总线（引擎和界面都收不到增量）。默认含 `**/.litecode/**`。**例外**（硬编码放行，否则设置无法热加载）：`excludes.json`、`mcp.json`、`custom_tools.json`。不放行 `index/` 等产物。

另两个开关：`git_ignore`（检索是否尊重 `.gitignore`，默认 `true`）；`explorer_git_ignore`（资源管理器是否尊重 `.gitignore`，默认 `false`）。检索只认排除名单 + 是否用 ignore 文件；不另藏 hidden。本目录自身的硬跳不走这份列表。

**怎么改。** 设置页；或按下面格式改本文件。目录写 `dir` 或 `**/dir`，不要写 `dir/`（`.gitignore` 的尾斜杠语义这里没有；保存时会去掉尾 `/`）。空行、`#` 行、重复 glob 会被丢掉。删掉本文件会在下次打开时重新种子。

**格式。** `version` 必须为 `1`。

```json
{
  "version": 1,
  "files_exclude": ["**/.git", "**/.DS_Store"],
  "search_exclude": ["**/node_modules"],
  "watcher_exclude": [".git/objects/**", "*.litecode-tmp*"],
  "git_ignore": true,
  "explorer_git_ignore": false
}
```

**怎么生效。** 监视器放行本文件，重载进程内列表（JSON 无效则保持上一份）。文本索引按新语料调和；已打开代码语义检索时会同步索引（对齐完成前 `code_search` 可能提示稍后再试）。改仓库里的 `.gitignore` 同样会调和索引，不经本文件。

### `mcp.json`

**干什么。** 本工作区 MCP 服务器（可覆盖同名的全局项）。

**怎么改。** 设置页增删改；或按格式改 JSON。

**格式。** `version` 为 `1`。`timeout` 单位秒，省略则为 60。`transport` 默认 stdio。

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

远程：`"transport": { "type": "remote", "url": "https://...", "headers": {} }`。

**怎么生效。** 空闲时改本文件或设置页保存：进程重读定义（JSON 无效则保持上一份），设置页立刻看到增删。对话进行中不重载（与设置页相同），下一轮开始再读盘。已启动的 MCP 进程不自动重启；要换命令请在设置页手动重启。

### `custom_tools.json`

**干什么。** 本工作区自定义工具（一条命令 + JSON Schema）。名字不能和内置工具撞名。

**怎么改。** 设置页增删改；或按格式改 JSON。

**格式。** `version` 为 `1`。`timeout` 单位秒，省略或 `0` 则为 120。`name` 必须和 map 的键相同。Schema 类型字段是 `"type"`。

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

**怎么生效。** 空闲时改本文件或设置页保存：进程重读定义（JSON 无效则保持上一份），设置页立刻看到增删。对话进行中不重载（与设置页相同）；下一轮对话读盘后可用。

---

## 只读

排障或用专用工具可以读。**不要**用 `write` / `edit` / `bash` 创建、覆盖、移动、删除。例外只有：`engines.json` 由人类在设置页开关；计划用 `plan` 工具（不要自拟文件名）。打开工作区会建空的 `logs/`、`plan/`，并覆盖本 README。

路径均相对 `.litecode/`。

| 路径 | 是什么 |
|------|--------|
| `README.md` | 本地图。每次打开覆盖 |
| `workspace.json` | 稳定工作区 id，绑主机侧快照。不是会话 id、不是密钥。复制工程会生成新 id |
| `engines.json` | 是否开启 LSP / 代码语义检索。人类在设置页开关；Agent 只读。有 `code_search` 再用 |
| `workspace.lock` | 同一时刻只允许一个 serve/CLI 占用本工作区。异常退出后先确认没有其它 LiteCode 再处理残留锁 |
| `sessions.db` | 会话日志真源（SQLite，可能还有 `-wal`/`-shm`）。打开工作区不预建。查历史用 `session_search`。不要删 |
| `logs/` | 进程日志 `logs/litecode.log`。排障可读 |
| `plan/` | 工作区计划稿。增/结束只走 `plan` 工具（`finish` 只清会话指针，不删文件） |
| `bash/` | 后台命令输出：`bash/<id>.output`（id 形如 `bg_<8 位 hex>`）。用 `read` 看，不要改正在写的文件 |
| `index/` | `code_search` 语义索引产物。不要手编 |
| `session-index/` | 会话语料的语义索引。字面检索走 `sessions.db` |
| `text-index/` | `grep` 加速索引。语料跟检索规则（`files_exclude` ∪ `search_exclude` + `git_ignore`）对齐，不把 `watcher_exclude` 当第四套搜索排除 |
| `sessions/` | **虚拟**，磁盘上通常没有。投影为 `sessions/<完整 session_id>.md`。`read` / `grep` / `glob` 必须把 `path` 指到 `sessions` 或某个 `.md`。不要 `mkdir` |

旧的 `snapshots/` 打开工作区会清掉。文件回退在 `~/.litecode/snapshots/<workspace_id>/`。

---

## Agent 速查

1. 项目规则 → 仓库根 `CLAUDE.md` / `AGENTS.md`。
2. 排除 / MCP / 自定义工具 → 「可改的配置」或设置页。
3. 待办 → `todo` 工具（在会话里，不在本目录）。
4. 旧会话 → `session_search`，再 `read` `.litecode/sessions/<id>.md`。
5. 不要删整个 `.litecode/`，也不要动「只读」表里的路径。
