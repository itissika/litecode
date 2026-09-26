## 只读

排障或用专用工具可以读。**不要**用 `write` / `edit` / `bash` 创建、覆盖、移动、删除。例外只有：`engines.json` 由人类在设置页开关；计划的创建/结束走 `plan` 工具（不要自拟文件名），正文可用 `edit` 修订，但不要 `write`、`rm`、重命名。打开工作区会建空的 `logs/`、`plan/`，并覆盖本 README。

路径均相对 `.litecode/`。

| 路径 | 是什么 |
|------|--------|
| `README.md` | 本地图。每次打开覆盖 |
| `workspace.json` | 稳定工作区 id，绑主机侧快照。不是会话 id、不是密钥。复制工程会生成新 id |
| `engines.json` | 是否开启 LSP / 代码语义检索。人类在设置页 **Engines** 开关；不要手改本文件（监视器不放行，引擎不会跟着磁盘改动起来）。有 `code_search` 再用 |
| `workspace.lock` | 同一时刻只允许一个 serve/CLI 占用本工作区。异常退出后先确认没有其它 LiteCode 再处理残留锁 |
| `sessions.db` | 会话日志真源（SQLite，可能还有 `-wal`/`-shm`）。打开工作区不预建。查历史用 `session_search`。不要删 |
| `logs/` | 进程日志 `logs/litecode.log`。排障可读 |
| `plan/` | 工作区计划稿。创建/结束走 `plan` 工具，正文用 `edit` 修订（不要 `write`/`rm`/重命名；`finish` 只清会话指针，不删文件） |
| `bash/` | 后台命令输出：`bash/<id>.output`（id 形如 `bg_<8 位 hex>`）。用 `read` 看，不要改正在写的文件 |
| `index/` | `code_search` 语义索引产物。不要手编 |
| `session-index/` | 会话语料的语义索引。字面检索走 `sessions.db` |
| `text-index/` | `grep` 加速索引。语料跟检索规则（`files_exclude` ∪ `search_exclude` + `git_ignore`）对齐，不把 `watcher_exclude` 当第四套搜索排除 |
| `sessions/` | **虚拟**，磁盘上通常没有。投影为 `sessions/<完整 session_id>.md`。`read` / `grep` / `glob` 必须把 `path` 指到 `sessions` 或某个 `.md`。不要 `mkdir` |

旧的 `snapshots/` 打开工作区会清掉。文件回退在 `~/.litecode/snapshots/<workspace_id>/`。
